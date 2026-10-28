using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.UI.Accessibility;
using Windows.Win32.UI.WindowsAndMessaging;

namespace Peekaboo.Helper.Input;

/// <summary>
/// WH_KEYBOARD_LL 훅을 전용 스레드에 설치하고 메시지 루프를 돌린다.
/// 콜백은 LowLevelHooksTimeout(최대 1초)을 넘기면 Windows 가 훅을 조용히 제거하므로,
/// 판정 함수는 반드시 빠르게 반환해야 한다(COM 호출 금지).
/// 같은 스레드에서 포그라운드 창 변경도 구독한다(WINEVENT_OUTOFCONTEXT 는 설치한 스레드의 메시지 루프로 전달됨).
/// </summary>
internal sealed class KeyboardHook : IDisposable
{
    /// <summary>(vkCode, isKeyUp) → true 면 키를 삼킨다.</summary>
    public delegate bool KeyHandler(uint vkCode, bool isKeyUp);

    private const uint LLKHF_UP = 0x80;

    private readonly KeyHandler _handler;
    private readonly HOOKPROC _proc; // GC 수거 방지를 위해 필드로 유지
    private readonly Action<HWND> _onForeground;
    private readonly WINEVENTPROC _winEventProc;
    private readonly Thread _thread;
    private readonly ManualResetEventSlim _installed = new();
    private uint _threadId;
    private Exception? _installError;

    public KeyboardHook(KeyHandler handler, Action<HWND> onForeground)
    {
        _handler = handler;
        _onForeground = onForeground;
        _proc = HookProc;
        _winEventProc = WinEventProc;
        _thread = new Thread(Run) { IsBackground = true, Name = "keyboard-hook" };
    }

    public void Start()
    {
        _thread.Start();
        _installed.Wait();
        if (_installError is not null) throw _installError;
    }

    private unsafe void Run()
    {
        _threadId = PInvoke.GetCurrentThreadId();
        HHOOK hook;
        try
        {
            var module = PInvoke.GetModuleHandle((PCWSTR)null);
            hook = PInvoke.SetWindowsHookEx(WINDOWS_HOOK_ID.WH_KEYBOARD_LL, _proc, (HINSTANCE)module.Value, 0);
            if (hook.IsNull) throw new System.ComponentModel.Win32Exception();
        }
        catch (Exception e)
        {
            _installError = e;
            _installed.Set();
            return;
        }
        _installed.Set();

        // 실패해도 키 처리는 되므로 경고만 남긴다(포그라운드 사전 준비가 빠질 뿐)
        var winEvent = PInvoke.SetWinEventHook(PInvoke.EVENT_SYSTEM_FOREGROUND, PInvoke.EVENT_SYSTEM_FOREGROUND, HMODULE.Null,
            _winEventProc, 0, 0, PInvoke.WINEVENT_OUTOFCONTEXT | PInvoke.WINEVENT_SKIPOWNPROCESS);
        if (winEvent.IsNull) Log.Warn("SetWinEventHook(EVENT_SYSTEM_FOREGROUND) failed");

        MSG msg;
        while (PInvoke.GetMessage(&msg, HWND.Null, 0, 0) > 0)
        {
            PInvoke.TranslateMessage(&msg);
            PInvoke.DispatchMessage(&msg);
        }
        if (!winEvent.IsNull) PInvoke.UnhookWinEvent(winEvent);
        PInvoke.UnhookWindowsHookEx(hook);
    }

    private void WinEventProc(HWINEVENTHOOK hook, uint @event, HWND hwnd, int idObject, int idChild, uint idEventThread, uint dwmsEventTime)
    {
        try { _onForeground(hwnd); }
        catch (Exception e) { Log.Error($"foreground handler failed: {e}"); }
    }

    private unsafe LRESULT HookProc(int nCode, WPARAM wParam, LPARAM lParam)
    {
        if (nCode == (int)PInvoke.HC_ACTION)
        {
            var info = (KBDLLHOOKSTRUCT*)lParam.Value;
            bool swallow;
            try
            {
                swallow = _handler(info->vkCode, (info->flags & (KBDLLHOOKSTRUCT_FLAGS)LLKHF_UP) != 0);
            }
            catch (Exception e)
            {
                Log.Error($"key handler failed: {e}");
                swallow = false;
            }
            if (swallow) return (LRESULT)1;
        }
        return PInvoke.CallNextHookEx(HHOOK.Null, nCode, wParam, lParam);
    }

    public void Dispose()
    {
        if (_threadId != 0) PInvoke.PostThreadMessage(_threadId, PInvoke.WM_QUIT, default, default);
        _thread.Join(1000);
    }
}
