using System.Runtime.InteropServices;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.System.Com;
using Windows.Win32.UI.Shell;
using Windows.Win32.Graphics.Gdi;
using Windows.Win32.UI.Shell.PropertiesSystem;
using Windows.Win32.UI.WindowsAndMessaging;

namespace Peekaboo.Helper.Shell;

internal enum NativePreviewStatus { Shown, Unavailable, Failed }

/// <summary>
/// Windows 에 등록된 미리보기 핸들러(IPreviewHandler)를 호스팅한다. 탐색기 미리 보기 창과 같은 방식.
/// - 핸들러 창은 미리보기 창(Electron, 다른 프로세스) HWND 의 자식으로 붙는다.
/// - 핸들러는 항상 별도 프로세스(prevhost.exe 등)에서 돈다(문서: "Preview handlers always run out of process").
///   그래서 CLSCTX_LOCAL_SERVER 로만 만들고, 헬퍼 안에 in-proc 로 로드하지 않는다.
/// - DoPreview 가 수 초 걸릴 수 있어 선택 조회와 다른 전용 STA 스레드에서 처리한다.
/// - 한 번에 하나만 연다. 파일마다 새 인스턴스를 만든다(Unload 후 재초기화 규칙이 문서상 모호함).
/// - 핸들러를 미리보기 창에 직접 붙이지 않고, 헬퍼가 만든 컨테이너 자식 창에 (0,0,w,h)로 붙인다.
///   실측: Word 는 SetWindow 의 RECT 위치를 따르지만 PowerPoint 는 위치를 무시하고 (0,0)에 그려 헤더를 덮었다.
///   SetWindow/SetRect 문서는 좌표계를 명시하지 않으므로, 원점을 맞춰 두면 핸들러별 해석 차이가 사라진다.
/// - 핸들러 class factory 를 캐시하고 LockServer(true) 로 서버 프로세스(WINWORD -Embedding 등)를 살려 둬
///   두 번째 미리보기부터 기동 지연을 없앤다. 상주 메모리를 줄이려고 일정 시간 안 쓰면 놓아준다.
///   (기법 참고: QuickLook OfficeViewer PreviewHandlerHost, QL-Win/QuickLook@2ec219a, GPL-3.0 — 코드는 새로 작성)
/// </summary>
internal sealed class NativePreviewHost : IDisposable
{
    /// <summary>미리보기 핸들러 shellex 키 GUID. https://learn.microsoft.com/windows/win32/shell/how-to-register-a-preview-handler</summary>
    private const string PreviewHandlerShellEx = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";

    private readonly StaDispatcher _sta = new("preview-sta");
    private readonly Action<int, NativePreviewStatus, string?> _reply;

    /// <summary>가장 최근에 요청된 id. 큐에 쌓인 오래된 open 은 건너뛴다(방향키 연타 대비).</summary>
    private volatile int _latestId;

    // STA 스레드 전용
    private int _currentId;
    private IPreviewHandler? _handler;
    private IStream? _stream; // 핸들러 수명 동안 열려 있어야 한다(IInitializeWithStream 문서)
    private HWND _container;
    private HBRUSH _background;
    private WNDPROC? _wndProc; // GC 수거 방지
    private const string ContainerClass = "PeekabooPreviewContainer";

    /// <summary>이 시간 동안 미리보기를 안 쓰면 캐시한 factory 를 풀어 Office 서버 프로세스가 종료되게 한다.</summary>
    private const int FactoryIdleMs = 10 * 60 * 1000;
    /// <summary>RPC_E_CANTCALLOUT_ININPUTSYNCCALL — 입력 동기 호출 중 SetRect. 다음 크기 변경 때 맞춰지므로 무시.</summary>
    private const int RpcCantCallOutInInputSyncCall = unchecked((int)0x8001010D);
    private readonly Dictionary<Guid, IClassFactory> _factories = new(); // STA 전용
    private long _lastUse;           // STA 전용, Environment.TickCount64
    private bool _releaseScheduled;  // STA 전용

    public NativePreviewHost(Action<int, NativePreviewStatus, string?> reply)
    {
        _reply = reply;
        // 렌더러가 넘기는 rect 는 물리 픽셀이고 부모(Electron)는 Per-Monitor V2 다. 이 스레드가 DPI 비인식이면
        // 컨테이너 창 좌표가 배율만큼 다시 변환될 수 있으므로, 창을 만들기 전에 스레드 DPI 인식을 맞춘다.
        _sta.Post(() =>
        {
            if (PInvoke.SetThreadDpiAwarenessContext(Windows.Win32.UI.HiDpi.DPI_AWARENESS_CONTEXT.DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == default)
                Log.Warn("SetThreadDpiAwarenessContext(PMv2) failed");
        });
    }

    public void Open(int id, string path, nint parent, RECT rect, bool dark)
    {
        _latestId = id;
        _sta.Post(() =>
        {
            Unload();
            if (id != _latestId) return; // 더 새 요청이 있음 — 응답은 호스트가 버린다
            var (status, message) = Show(id, path, (HWND)parent, rect, dark);
            _reply(id, status, message);
        });
    }

    public void Resize(int id, RECT rect) => _sta.Post(() =>
    {
        if (id != _currentId || _handler is null) return;
        try
        {
            PInvoke.SetWindowPos(_container, HWND.Null, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top,
                SET_WINDOW_POS_FLAGS.SWP_NOZORDER | SET_WINDOW_POS_FLAGS.SWP_NOACTIVATE);
            var inner = Origin(rect);
            _handler.SetRect(in inner);
        }
        catch (COMException e) when (e.HResult == RpcCantCallOutInInputSyncCall) { }
        catch (Exception e) { Log.Warn($"preview SetRect failed: {e.Message}"); }
    });

    public void Close(int id)
    {
        if (id == _latestId) _latestId = 0;
        _sta.Post(() =>
        {
            if (id == _currentId) Unload();
        });
    }

    private (NativePreviewStatus, string?) Show(int id, string path, HWND parent, RECT rect, bool dark)
    {
        if (!PInvoke.IsWindow(parent)) return (NativePreviewStatus.Failed, "parent window is gone");

        var clsid = FindHandler(Path.GetExtension(path));
        if (clsid is null) return (NativePreviewStatus.Unavailable, null);

        try
        {
            var handler = Create(clsid.Value);
            _handler = handler;
            _currentId = id;
            Initialize(handler, path);
            if (handler is IPreviewHandlerVisuals visuals) ApplyTheme(visuals, dark);
            _container = CreateContainer(parent, rect, dark);
            var inner = Origin(rect);
            handler.SetWindow(_container, in inner);
            handler.DoPreview();
            return (NativePreviewStatus.Shown, null);
        }
        // HRESULT 는 FileNotFound/OutOfMemory 등 여러 예외 형식으로 바뀌어 오므로 전부 잡는다.
        // 여기서 새면 응답이 없어 호스트가 타임아웃까지 기다리고 컨테이너 창이 남는다.
        catch (Exception e)
        {
            Log.Warn($"preview handler {clsid} failed for {Path.GetExtension(path)}: 0x{e.HResult:X8} {e.Message}");
            Unload();
            return (NativePreviewStatus.Failed, $"0x{e.HResult:X8}");
        }
    }

    /// <summary>
    /// 확장자 → 핸들러 CLSID. AssocQueryString(ASSOCSTR_SHELLEXTENSION, pszExtra=미리보기 GUID).
    /// 호스트는 연결 정보로 핸들러를 정해야 한다(IPreviewHandler 문서 Remarks).
    /// </summary>
    private static unsafe Guid? FindHandler(string ext)
    {
        if (string.IsNullOrEmpty(ext)) return null;
        Span<char> buf = stackalloc char[64]; // "{GUID}" 는 38자
        uint len = (uint)buf.Length;
        var hr = PInvoke.AssocQueryString(ASSOCF.ASSOCF_NOTRUNCATE, ASSOCSTR.ASSOCSTR_SHELLEXTENSION, ext, PreviewHandlerShellEx, buf, ref len);
        if (hr.Failed || len == 0) return null;
        // pcchOut 은 종료 NUL 을 포함한 문자 수
        return Guid.TryParse(buf[..((int)len - 1)], out var clsid) ? clsid : null;
    }

    /// <summary>캐시한 factory 로 만든다. 서버 프로세스가 죽어 실패하면 factory 를 새로 얻어 한 번 더 시도.</summary>
    private unsafe IPreviewHandler Create(Guid clsid)
    {
        _lastUse = Environment.TickCount64;
        ScheduleFactoryRelease();
        var iid = typeof(IPreviewHandler).GUID;
        for (int attempt = 0; ; attempt++)
        {
            var factory = GetFactory(clsid);
            try
            {
                factory.CreateInstance(null, &iid, out var ppv);
                try { return (IPreviewHandler)Marshal.GetObjectForIUnknown((nint)ppv); }
                finally { Marshal.Release((nint)ppv); }
            }
            catch (COMException e) when (attempt == 0)
            {
                Log.Warn($"cached factory for {clsid} failed (0x{e.HResult:X8}), retrying");
                DropFactory(clsid);
            }
        }
    }

    private unsafe IClassFactory GetFactory(Guid clsid)
    {
        if (_factories.TryGetValue(clsid, out var cached)) return cached;
        var iid = typeof(IClassFactory).GUID;
        PInvoke.CoGetClassObject(&clsid, CLSCTX.CLSCTX_LOCAL_SERVER, null, &iid, out var obj).ThrowOnFailure();
        var factory = (IClassFactory)obj;
        try { factory.LockServer(true); }
        catch (COMException e) { Log.Warn($"LockServer failed for {clsid}: 0x{e.HResult:X8}"); }
        _factories[clsid] = factory;
        return factory;
    }

    private void DropFactory(Guid clsid)
    {
        if (!_factories.Remove(clsid, out var factory)) return;
        try { factory.LockServer(false); } catch (COMException) { } // 서버가 이미 죽었을 수 있다
        Marshal.FinalReleaseComObject(factory);
    }

    private void ScheduleFactoryRelease()
    {
        if (_releaseScheduled) return;
        _releaseScheduled = true;
        _sta.PostDelayed(FactoryIdleMs, ReleaseIdleFactories);
    }

    private void ReleaseIdleFactories()
    {
        _releaseScheduled = false;
        long idle = Environment.TickCount64 - _lastUse;
        if (_handler is not null || idle < FactoryIdleMs)
        {
            _releaseScheduled = true;
            _sta.PostDelayed((int)Math.Max(1000, FactoryIdleMs - idle), ReleaseIdleFactories);
            return;
        }
        if (_factories.Count > 0) Log.Info($"releasing {_factories.Count} idle preview handler factories");
        foreach (var clsid in _factories.Keys.ToList()) DropFactory(clsid);
    }

    /// <summary>문서 권고 순서: 스트림(가장 안전) → 항목 → 파일 경로. 미리보기는 파일을 잠그지 않아야 하므로 공유 읽기.</summary>
    private unsafe void Initialize(IPreviewHandler handler, string path)
    {
        const STGM Mode = STGM.STGM_READ | STGM.STGM_SHARE_DENY_NONE;
        if (handler is IInitializeWithStream withStream)
        {
            PInvoke.SHCreateStreamOnFileEx(path, (uint)Mode, 0, false, null, out var stream).ThrowOnFailure();
            _stream = stream;
            withStream.Initialize(stream, (uint)STGM.STGM_READ);
        }
        else if (handler is IInitializeWithItem withItem)
        {
            var iid = typeof(IShellItem).GUID;
            object item;
            fixed (char* p = path) PInvoke.SHCreateItemFromParsingName((PCWSTR)p, null, &iid, out item).ThrowOnFailure();
            try { withItem.Initialize((IShellItem)item, (uint)STGM.STGM_READ); }
            finally { Marshal.ReleaseComObject(item); } // 핸들러가 필요하면 자체 참조를 잡는다
        }
        else if (handler is IInitializeWithFile withFile)
        {
            withFile.Initialize(path, (uint)STGM.STGM_READ);
        }
        else
        {
            throw new InvalidCastException("handler supports no initialization interface");
        }
    }

    /// <summary>Visuals 는 제안일 뿐이라 핸들러가 무시해도 된다(Office 는 대부분 자체 배경을 쓴다).</summary>
    private static void ApplyTheme(IPreviewHandlerVisuals visuals, bool dark)
    {
        try
        {
            visuals.SetBackgroundColor(new COLORREF(dark ? 0x00202020u : 0x00F3F3F3u));
            visuals.SetTextColor(new COLORREF(dark ? 0x00F0F0F0u : 0x00202020u));
        }
        catch (Exception e) when (e is COMException or NotImplementedException) { }
    }

    private void Unload()
    {
        var handler = _handler;
        _handler = null;
        _currentId = 0;
        // 단계마다 독립적으로 정리한다 — 한 단계가 실패해도 창·스트림이 남지 않게
        if (handler is not null)
        {
            try { handler.Unload(); }
            catch (Exception e) { Log.Warn($"preview Unload failed: {e.Message}"); }
            try { Marshal.FinalReleaseComObject(handler); }
            catch (Exception e) { Log.Warn($"preview handler release failed: {e.Message}"); }
        }
        if (_stream is not null)
        {
            try { Marshal.FinalReleaseComObject(_stream); }
            catch (Exception e) { Log.Warn($"preview stream release failed: {e.Message}"); }
            _stream = null;
        }
        if (!_container.IsNull)
        {
            PInvoke.DestroyWindow(_container);
            _container = HWND.Null;
        }
        if (!_background.IsNull)
        {
            PInvoke.DeleteObject(_background);
            _background = HBRUSH.Null;
        }
    }

    private static RECT Origin(RECT r) => new() { left = 0, top = 0, right = r.right - r.left, bottom = r.bottom - r.top };

    /// <summary>미리보기 창(다른 프로세스) 안에 rect 위치로 자식 창을 만든다. 핸들러가 그리기 전까지 테마 배경색으로 칠한다.</summary>
    private unsafe HWND CreateContainer(HWND parent, RECT rect, bool dark)
    {
        var module = PInvoke.GetModuleHandle((PCWSTR)null);
        if (_wndProc is null)
        {
            WNDPROC proc = ContainerProc;
            fixed (char* name = ContainerClass)
            {
                var wc = new WNDCLASSEXW
                {
                    cbSize = (uint)sizeof(WNDCLASSEXW),
                    lpfnWndProc = proc,
                    hInstance = (HINSTANCE)module.Value,
                    lpszClassName = name,
                };
                if (PInvoke.RegisterClassEx(in wc) == 0) throw new COMException("RegisterClassEx failed", Marshal.GetHRForLastWin32Error());
            }
            _wndProc = proc; // 등록 성공 후에만 — 실패하면 다음 open 에서 다시 등록을 시도한다
        }
        _background = PInvoke.CreateSolidBrush(new COLORREF(dark ? 0x00202020u : 0x00F3F3F3u));
        var hwnd = PInvoke.CreateWindowEx(0, ContainerClass, null,
            WINDOW_STYLE.WS_CHILD | WINDOW_STYLE.WS_VISIBLE | WINDOW_STYLE.WS_CLIPCHILDREN | WINDOW_STYLE.WS_CLIPSIBLINGS,
            rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, parent, null, null, null);
        if (hwnd.IsNull) throw new COMException("CreateWindowEx failed", Marshal.GetHRForLastWin32Error());
        return hwnd;
    }

    private unsafe LRESULT ContainerProc(HWND hwnd, uint msg, WPARAM wParam, LPARAM lParam)
    {
        if (msg == PInvoke.WM_ERASEBKGND && !_background.IsNull)
        {
            PInvoke.GetClientRect(hwnd, out var rc);
            PInvoke.FillRect((HDC)(nint)wParam.Value, &rc, _background);
            return (LRESULT)1;
        }
        return PInvoke.DefWindowProc(hwnd, msg, wParam, lParam);
    }

    public void Dispose()
    {
        _latestId = 0;
        // 종료 전에 Unload 가 실제로 끝나야 핸들러 프로세스가 파일·스트림을 놓는다(DoPreview 중이면 그 뒤에 실행됨)
        using var done = new ManualResetEventSlim();
        _sta.Post(() =>
        {
            try
            {
                Unload();
                foreach (var clsid in _factories.Keys.ToList()) DropFactory(clsid);
            }
            finally { done.Set(); }
        });
        if (!done.Wait(3000)) Log.Warn("preview host did not unload within 3s");
        _sta.Dispose();
    }
}
