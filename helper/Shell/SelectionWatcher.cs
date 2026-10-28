using System.Runtime.InteropServices;
using Windows.Win32.Foundation;
using Windows.Win32.System.Com;
using Windows.Win32.UI.Shell;

namespace Peekaboo.Helper.Shell;

/// <summary>
/// DShellFolderViewEvents 싱크. CsWin32 메타데이터의 dispinterface 는 멤버가 비어 있어 직접 선언한다.
/// DIID_DShellFolderViewEvents = 62112AA2-EBE4-11CF-A5FB-0020AFE7292D, DISPID_SELECTIONCHANGED = 200 (shdispid.h)
/// </summary>
[ComVisible(true)]
[Guid("62112AA2-EBE4-11CF-A5FB-0020AFE7292D")]
[InterfaceType(ComInterfaceType.InterfaceIsIDispatch)]
public interface IShellFolderViewEventsSink
{
    [DispId(200)]
    void SelectionChanged();
}

[ComVisible(true)]
[ClassInterface(ClassInterfaceType.None)]
public sealed class ShellFolderViewEventsSink : IShellFolderViewEventsSink
{
    private readonly Action _onChanged;
    internal ShellFolderViewEventsSink(Action onChanged) => _onChanged = onChanged;
    public void SelectionChanged() => _onChanged();
}

/// <summary>
/// 미리보기가 열려 있는 동안에만 활성 뷰의 선택 변경 이벤트를 구독한다. StaDispatcher 스레드 전용.
/// 구독 실패는 치명적이지 않다 — 방향키 입력 후 재조회가 폴백으로 동작한다.
/// </summary>
internal sealed class SelectionWatcher(Action onChanged)
{
    private static readonly Guid IID_IDispatch = new("00020400-0000-0000-C000-000000000046");

    private IConnectionPoint? _point;
    private uint _cookie;
    private HWND _viewHwnd;

    public bool IsWatching(HWND viewHwnd) => _point is not null && _viewHwnd == viewHwnd;

    public unsafe void Attach(IShellView view, HWND viewHwnd)
    {
        if (IsWatching(viewHwnd)) return;
        Detach();
        try
        {
            var iidDispatch = IID_IDispatch;
            view.GetItemObject(_SVGIO.SVGIO_BACKGROUND, &iidDispatch, out var ppv);
            if (ppv is null) return;
            object automation;
            try { automation = Marshal.GetObjectForIUnknown((nint)ppv); }
            finally { Marshal.Release((nint)ppv); }

            if (automation is not IConnectionPointContainer container) return;
            var diid = typeof(IShellFolderViewEventsSink).GUID;
            container.FindConnectionPoint(&diid, out var point);
            point.Advise(new ShellFolderViewEventsSink(onChanged), out _cookie);
            _point = point;
            _viewHwnd = viewHwnd;
        }
        catch (Exception e) when (e is COMException or InvalidCastException or UnauthorizedAccessException)
        {
            Log.Warn($"selection watch unavailable, falling back to key-driven refresh: {e.Message}");
        }
    }

    public void Detach()
    {
        if (_point is null) return;
        try { _point.Unadvise(_cookie); }
        catch (COMException) { /* 뷰가 이미 사라짐 */ }
        _point = null;
        _cookie = 0;
        _viewHwnd = HWND.Null;
    }
}
