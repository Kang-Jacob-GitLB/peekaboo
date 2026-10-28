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

    /// <summary>
    /// 훅 콜백에서 호출되므로 COM 없이 가벼운 Win32 호출만 사용한다.
    /// 허용 목록 방식: 파일 목록(DirectUIHWND, 부모 SHELLDLL_DefView)에 포커스가 있고 캐럿이 없을 때만 대상.
    /// 이름 바꾸기(Edit), 주소창, 검색창 등은 이 조건에 걸리지 않으므로 자동으로 제외된다.
    /// </summary>
    public static unsafe FileViewTarget? GetFileViewTarget()
    {
        var fg = PInvoke.GetForegroundWindow();
        if (fg.IsNull) return null;

        var fgClass = ClassOf(fg);
        SelectionSource source;
        if (fgClass == "CabinetWClass") source = SelectionSource.Explorer;
        else if (fgClass is "Progman" or "WorkerW") source = SelectionSource.Desktop;
        else return null;

        uint pid;
        var tid = PInvoke.GetWindowThreadProcessId(fg, &pid);
        if (tid == 0) return null;

        var gui = new GUITHREADINFO { cbSize = (uint)sizeof(GUITHREADINFO) };
        if (!PInvoke.GetGUIThreadInfo(tid, ref gui)) return null;
        if (!gui.hwndCaret.IsNull) return null; // 텍스트 입력 중

        var focus = gui.hwndFocus;
        if (focus.IsNull || ClassOf(focus) != "DirectUIHWND") return null;
        var view = PInvoke.GetParent(focus);
        if (view.IsNull || ClassOf(view) != "SHELLDLL_DefView") return null;

        return new FileViewTarget(source, fg, view, pid);
    }

    /// <summary>
    /// 관리자 권한 프로세스인지. 토큰을 열 수 없으면(권한 부족) 관리자 권한으로 간주한다.
    /// 일반 권한 헬퍼는 UIPI 때문에 그런 창의 선택을 읽을 수 없으므로, 키를 가로채지 않는다.
    /// </summary>
    public static unsafe bool IsElevated(uint pid)
    {
        lock (ElevationCache)
        {
            if (ElevationCache.TryGetValue(pid, out var cached)) return cached;
        }

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

        lock (ElevationCache)
        {
            if (ElevationCache.Count > 64) ElevationCache.Clear();
            ElevationCache[pid] = elevated;
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
