using Peekaboo.Helper.Ipc;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.Security;
using Windows.Win32.System.Threading;
using Windows.Win32.UI.WindowsAndMessaging;

namespace Peekaboo.Helper.Input;

/// <summary>키 입력 시점의 포커스가 "탐색기/바탕화면의 파일 목록"인지 판정한다.</summary>
/// <param name="TopLevel">포그라운드 최상위 창(CabinetWClass / Progman / WorkerW)</param>
/// <param name="ViewWindow">파일 목록을 담은 SHELLDLL_DefView 창. 탭이 여러 개일 때 활성 탭을 고르는 기준.</param>
internal sealed record FileViewTarget(SelectionSource Source, HWND TopLevel, HWND ViewWindow, uint ProcessId);

internal static class FocusInspector
{
    private static readonly Dictionary<uint, bool> ElevationCache = new();
    private static readonly HashSet<uint> ElevationPending = new();
    private static readonly HashSet<uint> ShellPids = new(); // 훅 스레드 전용

    /// <summary>
    /// 훅 콜백에서 호출되므로 COM 없이 가벼운 Win32 호출만 사용한다.
    /// 허용 목록 방식: 파일 목록(DirectUIHWND 또는 SysListView32, 부모 SHELLDLL_DefView)에 포커스가 있고 캐럿이 없을 때만 대상.
    /// 이름 바꾸기(Edit), 주소창, 검색창 등은 이 조건에 걸리지 않으므로 자동으로 제외된다.
    /// </summary>
    public static unsafe FileViewTarget? GetFileViewTarget()
    {
        var fg = PInvoke.GetForegroundWindow();
        if (fg.IsNull) return null;

        if (SourceOf(fg) is not { } source) return null;

        uint pid;
        var tid = PInvoke.GetWindowThreadProcessId(fg, &pid);
        if (tid == 0) return null;

        var gui = new GUITHREADINFO { cbSize = (uint)sizeof(GUITHREADINFO) };
        if (!PInvoke.GetGUIThreadInfo(tid, ref gui)) return null;
        if (!gui.hwndCaret.IsNull) return null; // 텍스트 입력 중

        // 탐색기 파일 목록은 DirectUIHWND, 바탕화면 아이콘 뷰는 SysListView32 ("FolderView")
        var focus = gui.hwndFocus;
        if (focus.IsNull || ClassOf(focus) is not ("DirectUIHWND" or "SysListView32")) return null;
        var view = PInvoke.GetParent(focus);
        if (view.IsNull || ClassOf(view) != "SHELLDLL_DefView") return null;

        return new FileViewTarget(source, fg, view, pid);
    }

    /// <summary>포그라운드가 탐색기 창 또는 바탕화면인지(포커스 위치는 따지지 않음).</summary>
    public static bool IsShellForeground()
    {
        var fg = PInvoke.GetForegroundWindow();
        return !fg.IsNull && SourceOf(fg) is not null;
    }

    /// <summary>
    /// 포그라운드로 올라온 창이 셸(탐색기 창·바탕화면·작업 표시줄·Alt+Tab 등 explorer.exe) 쪽인지 판정한다.
    /// 훅 스레드에서 불리므로 프로세스 이름 조회(OpenProcess) 대신 창 클래스와 PID 만 쓴다.
    /// 탐색기 창/바탕화면을 본 적 있는 PID 를 기억해, "폴더 창을 별도 프로세스로" 설정에서 그 프로세스의 대화상자도 셸로 본다.
    /// </summary>
    public static unsafe bool IsShellWindow(HWND hwnd, out uint pid)
    {
        pid = 0;
        var root = PInvoke.GetAncestor(hwnd, GET_ANCESTOR_FLAGS.GA_ROOT);
        if (root.IsNull) root = hwnd;
        uint p;
        if (PInvoke.GetWindowThreadProcessId(root, &p) == 0) return true; // 판단할 수 없으면 셸로 간주(닫지 않음)
        pid = p;
        if (SourceOf(root) is not null)
        {
            if (ShellPids.Count > 64) ShellPids.Clear();
            ShellPids.Add(p);
            return true;
        }
        var shell = PInvoke.GetShellWindow();
        uint shellPid;
        if (!shell.IsNull && PInvoke.GetWindowThreadProcessId(shell, &shellPid) != 0 && shellPid == p) return true;
        return ShellPids.Contains(p);
    }

    private static SelectionSource? SourceOf(HWND fg) => ClassOf(fg) switch
    {
        "CabinetWClass" => SelectionSource.Explorer,
        "Progman" or "WorkerW" => SelectionSource.Desktop,
        _ => null,
    };

    /// <summary>
    /// 관리자 권한 프로세스인지 캐시에서만 확인한다(훅 스레드용, 블로킹 없음).
    /// 실측에서 탐색기 프로세스의 토큰 조회가 처음 한 번 400~500ms 걸려 LowLevelHooksTimeout 을 넘길 수 있다.
    /// 그래서 계산은 포그라운드 변경 시점(<see cref="PrimeElevation"/>)에 백그라운드에서 해 두고,
    /// 캐시가 없으면 일반 권한으로 간주한 뒤 계산만 예약한다.
    /// </summary>
    public static bool IsElevatedCached(uint pid)
    {
        lock (ElevationCache)
        {
            if (ElevationCache.TryGetValue(pid, out var cached)) return cached;
        }
        PrimeElevation(pid);
        return false;
    }

    /// <summary>탐색기/바탕화면 창이면 그 프로세스의 관리자 권한 여부를 백그라운드에서 계산해 둔다.</summary>
    public static unsafe void PrimeForWindow(HWND hwnd)
    {
        if (hwnd.IsNull || SourceOf(hwnd) is null) return;
        uint pid;
        if (PInvoke.GetWindowThreadProcessId(hwnd, &pid) != 0) PrimeElevation(pid);
    }

    private static void PrimeElevation(uint pid)
    {
        lock (ElevationCache)
        {
            if (ElevationCache.ContainsKey(pid) || !ElevationPending.Add(pid)) return;
        }
        ThreadPool.QueueUserWorkItem(_ =>
        {
            bool elevated = ComputeElevated(pid);
            lock (ElevationCache)
            {
                ElevationPending.Remove(pid);
                if (ElevationCache.Count > 64) ElevationCache.Clear();
                ElevationCache[pid] = elevated;
            }
        });
    }

    /// <summary>
    /// 토큰을 열 수 없으면(권한 부족) 관리자 권한으로 간주한다.
    /// 일반 권한 헬퍼는 UIPI 때문에 그런 창의 선택을 읽을 수 없으므로, 키를 가로채지 않는다.
    /// </summary>
    private static unsafe bool ComputeElevated(uint pid)
    {
        bool elevated;
        using var process = PInvoke.OpenProcess_SafeHandle(PROCESS_ACCESS_RIGHTS.PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        if (process.IsInvalid)
        {
            elevated = true;
        }
        else if (!PInvoke.OpenProcessToken(process, TOKEN_ACCESS_MASK.TOKEN_QUERY, out var token))
        {
            elevated = true;
        }
        else
        {
            using (token)
            {
                TOKEN_ELEVATION info;
                elevated = PInvoke.GetTokenInformation(token, TOKEN_INFORMATION_CLASS.TokenElevation, &info, (uint)sizeof(TOKEN_ELEVATION), out _)
                    && info.TokenIsElevated != 0;
            }
        }

        return elevated;
    }

    public static unsafe string ClassOf(HWND hwnd)
    {
        const int Max = 64;
        char* buf = stackalloc char[Max];
        int len = PInvoke.GetClassName(hwnd, buf, Max);
        return len > 0 ? new string(buf, 0, len) : "";
    }
}
