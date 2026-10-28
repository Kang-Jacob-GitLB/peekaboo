using System.Runtime.InteropServices;
using Peekaboo.Helper.Input;
using Peekaboo.Helper.Ipc;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.System.Com;
using Windows.Win32.UI.Shell;

namespace Peekaboo.Helper.Shell;

internal sealed class ShellAccessException(string code, string message, Exception? inner = null)
    : Exception(message, inner)
{
    public string Code { get; } = code;
}

/// <summary>
/// 포그라운드 탐색기/바탕화면의 선택 항목 조회. 반드시 StaDispatcher 스레드에서 호출한다.
/// 경로: ShellWindows → (HWND 일치 항목) → IServiceProvider.QueryService(SID_STopLevelBrowser, IShellBrowser)
///       → QueryActiveShellView → IFolderView2.GetSelection
/// </summary>
internal sealed class ShellSelection
{
    private const int HResultNotFound = unchecked((int)0x80070490); // HRESULT_FROM_WIN32(ERROR_NOT_FOUND)

    private IShellWindows? _shellWindows;

    private IShellWindows ShellWindows => _shellWindows ??= (IShellWindows)new ShellWindows();

    /// <summary>대상 창의 활성 IShellView 를 찾는다. 탭이 여러 개면 SHELLDLL_DefView HWND 가 일치하는 뷰를 고른다.</summary>
    public unsafe IShellView FindView(FileViewTarget target)
    {
        try
        {
            return target.Source == SelectionSource.Desktop ? FindDesktopView() : FindExplorerView(target);
        }
        catch (ShellAccessException)
        {
            throw;
        }
        catch (UnauthorizedAccessException e)
        {
            throw new ShellAccessException(ErrorCodes.Elevated, "Access denied to the shell window (elevated?)", e);
        }
        catch (COMException e)
        {
            _shellWindows = null; // 탐색기 재시작 등으로 깨졌을 수 있으니 다음에 새로 만든다
            throw new ShellAccessException(ErrorCodes.ComFail, $"COM call failed: 0x{e.HResult:X8}", e);
        }
    }

    private unsafe IShellView FindExplorerView(FileViewTarget target)
    {
        IShellView? fallback = null;
        int candidates = 0;
        int count = ShellWindows.Count;
        for (int i = 0; i < count; i++)
        {
            object? item;
            try { item = ShellWindows.Item(i); }
            catch (COMException) { continue; }
            if (item is not IWebBrowserApp app) continue;
            if ((nint)app.HWND.Value != (nint)target.TopLevel.Value) continue;

            var view = ActiveViewOf(item);
            if (view is null) continue;
            candidates++;

            HWND viewHwnd;
            view.GetWindow(&viewHwnd);
            if (viewHwnd == target.ViewWindow) return view;
            fallback ??= view;
        }

        // 뷰 HWND 가 정확히 일치하지 않았지만 같은 창의 뷰가 하나뿐이면 그것을 쓴다
        if (fallback is not null && candidates == 1) return fallback;
        throw new ShellAccessException(ErrorCodes.NoView, $"No matching shell view (candidates={candidates})");
    }

    private IShellView FindDesktopView()
    {
        // 바탕화면: FindWindowSW(CSIDL_DESKTOP, empty, SWC_DESKTOP, SWFO_NEEDDISPATCH)
        object loc = (int)PInvoke.CSIDL_DESKTOP;
        object? root = null;
        var disp = ShellWindows.FindWindowSW(in loc, in root!, ShellWindowTypeConstants.SWC_DESKTOP, out _,
            ShellWindowFindWindowOptions.SWFO_NEEDDISPATCH);
        return ActiveViewOf(disp) ?? throw new ShellAccessException(ErrorCodes.NoView, "Desktop view not found");
    }

    private static unsafe IShellView? ActiveViewOf(object? browserObject)
    {
        if (browserObject is not Windows.Win32.System.Com.IServiceProvider sp) return null;

        var sid = PInvoke.SID_STopLevelBrowser;
        var iid = typeof(IShellBrowser).GUID;
        sp.QueryService(&sid, &iid, out var ppv);
        if (ppv is null) return null;
        try
        {
            var browser = (IShellBrowser)Marshal.GetObjectForIUnknown((nint)ppv);
            browser.QueryActiveShellView(out var view);
            return view;
        }
        finally
        {
            Marshal.Release((nint)ppv);
        }
    }

    /// <summary>진단용(--list): 열린 모든 탐색기 창/탭의 뷰와 선택을 나열한다. 포커스와 무관.</summary>
    public unsafe List<(HWND top, HWND view, SelectionResult selection)> EnumerateAll()
    {
        var result = new List<(HWND, HWND, SelectionResult)>();
        int count = ShellWindows.Count;
        for (int i = 0; i < count; i++)
        {
            object? item;
            try { item = ShellWindows.Item(i); }
            catch (COMException) { continue; }
            if (item is not IWebBrowserApp app) continue;
            var top = (HWND)(nint)app.HWND.Value;
            var view = ActiveViewOf(item);
            if (view is null) continue;
            HWND viewHwnd;
            view.GetWindow(&viewHwnd);
            var target = new FileViewTarget(SelectionSource.Explorer, top, viewHwnd, 0);
            result.Add((top, viewHwnd, ReadSelection(view, target)));
        }

        var desktop = FindDesktopView();
        HWND desktopView;
        desktop.GetWindow(&desktopView);
        result.Add((HWND.Null, desktopView,
            ReadSelection(desktop, new FileViewTarget(SelectionSource.Desktop, HWND.Null, desktopView, 0))));
        return result;
    }

    public SelectionResult Query(FileViewTarget target) => ReadSelection(FindView(target), target);

    private unsafe SelectionResult ReadSelection(IShellView view, FileViewTarget target)
    {
        try
        {
            if (view is not IFolderView2 folderView)
                throw new ShellAccessException(ErrorCodes.NoView, "Active view does not implement IFolderView2");

            var paths = new List<string>();
            // 선택이 없을 때: 문서상 S_FALSE + 빈 배열이지만, 실측(Win11 26200)에서는
            // HRESULT_FROM_WIN32(ERROR_NOT_FOUND) 예외가 난다. 둘 다 "선택 없음"으로 처리.
            IShellItemArray? items;
            try { folderView.GetSelection(false, out items); }
            catch (COMException e) when (e.HResult == HResultNotFound) { items = null; }
            if (items is not null)
            {
                items.GetCount(out uint n);
                for (uint i = 0; i < n; i++)
                {
                    items.GetItemAt(i, out var shellItem);
                    if (PathOf(shellItem) is string p) paths.Add(p);
                }
            }

            return new SelectionResult(paths.ToArray(), FocusedPath(folderView), target.Source, (nint)target.TopLevel.Value);
        }
        catch (COMException e)
        {
            throw new ShellAccessException(ErrorCodes.ComFail, $"Selection query failed: 0x{e.HResult:X8}", e);
        }
    }

    /// <summary>포커스(점선 테두리) 항목. 포커스 항목이 없으면 GetFocusedItem 이 실패하므로 null.</summary>
    private static unsafe string? FocusedPath(IFolderView2 folderView)
    {
        try
        {
            folderView.GetFocusedItem(out int focusIndex);
            if (focusIndex < 0) return null;
            var iid = typeof(IShellItem).GUID;
            folderView.GetItem(focusIndex, &iid, out var ppv);
            if (ppv is null) return null;
            try { return PathOf((IShellItem)Marshal.GetObjectForIUnknown((nint)ppv)); }
            finally { Marshal.Release((nint)ppv); }
        }
        catch (COMException e)
        {
            Log.Info($"no focused item: 0x{e.HResult:X8}");
            return null;
        }
    }

    /// <summary>파일 시스템 경로. 가상 항목(내 PC, zip 내부 등)은 실패하므로 null.</summary>
    private static unsafe string? PathOf(IShellItem item)
    {
        PWSTR name;
        try
        {
            item.GetDisplayName(SIGDN.SIGDN_FILESYSPATH, &name);
        }
        catch (Exception e) when (e is COMException or ArgumentException)
        {
            return null;
        }
        try { return name.ToString(); }
        finally { Marshal.FreeCoTaskMem((nint)name.Value); }
    }
}
