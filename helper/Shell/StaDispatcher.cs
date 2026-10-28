using System.Collections.Concurrent;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.UI.WindowsAndMessaging;

namespace Peekaboo.Helper.Shell;

/// <summary>
/// Shell COM 객체 전용 STA 스레드. 작업 큐 + 지연 작업 + Win32 메시지 펌프를 함께 돌린다.
/// (연결 지점 이벤트 싱크가 STA 에서 콜백을 받으려면 메시지 펌프가 필요하다.)
/// </summary>
internal sealed class StaDispatcher : IDisposable
{
    private readonly ConcurrentQueue<Action> _queue = new();
    private readonly List<(long due, Action action)> _timers = new(); // STA 스레드에서만 접근
    private readonly AutoResetEvent _wake = new(false);
    private readonly Thread _thread;
    private volatile bool _stopping;

    public StaDispatcher(string name = "shell-sta")
    {
        _thread = new Thread(Run) { IsBackground = true, Name = name };
        _thread.SetApartmentState(ApartmentState.STA);
        _thread.Start();
    }

    public void Post(Action action)
    {
        _queue.Enqueue(action);
        _wake.Set();
    }

    /// <summary>delayMs 뒤에 STA 스레드에서 실행.</summary>
    public void PostDelayed(int delayMs, Action action) =>
        Post(() => _timers.Add((Environment.TickCount64 + delayMs, action)));

    private unsafe void Run()
    {
        var handle = (HANDLE)_wake.SafeWaitHandle.DangerousGetHandle();
        while (!_stopping)
        {
            uint timeout = PInvoke.INFINITE;
            if (_timers.Count > 0)
            {
                long next = _timers.Min(t => t.due) - Environment.TickCount64;
                timeout = (uint)Math.Max(0, next);
            }

            PInvoke.MsgWaitForMultipleObjectsEx(new ReadOnlySpan<HANDLE>(&handle, 1), timeout,
                QUEUE_STATUS_FLAGS.QS_ALLINPUT, MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS.MWMO_INPUTAVAILABLE);

            MSG msg;
            while (PInvoke.PeekMessage(&msg, HWND.Null, 0, 0, PEEK_MESSAGE_REMOVE_TYPE.PM_REMOVE))
            {
                PInvoke.TranslateMessage(&msg);
                PInvoke.DispatchMessage(&msg);
            }

            while (_queue.TryDequeue(out var action)) Invoke(action);

            if (_timers.Count > 0)
            {
                long now = Environment.TickCount64;
                var due = _timers.Where(t => t.due <= now).ToList();
                _timers.RemoveAll(t => t.due <= now);
                foreach (var t in due) Invoke(t.action);
            }
        }
    }

    private static void Invoke(Action action)
    {
        try { action(); }
        catch (Exception e) { Log.Error($"sta task failed: {e}"); }
    }

    public void Dispose()
    {
        _stopping = true;
        _wake.Set();
        _thread.Join(1000);
    }
}
