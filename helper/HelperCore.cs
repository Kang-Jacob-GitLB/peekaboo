using Peekaboo.Helper.Input;
using Peekaboo.Helper.Ipc;
using Peekaboo.Helper.Shell;
using Windows.Win32;
using Windows.Win32.UI.Input.KeyboardAndMouse;

namespace Peekaboo.Helper;

/// <summary>헬퍼가 바깥으로 내보내는 이벤트. 프로토콜 모드(JSON Lines)와 콘솔 모드가 각각 구현한다.</summary>
internal interface IHelperSink
{
    void Key(KeyName key, bool consumed);
    void Selection(int? id, SelectionResult selection);
    void Error(int? id, string code, string message);
}

/// <summary>
/// 키 라우팅과 선택 조회를 묶는다.
/// - 키 판정은 훅 스레드에서 즉시 끝낸다(COM 없음).
/// - 선택 조회와 변경 감시는 STA 스레드에서 한다.
/// </summary>
internal sealed class HelperCore : IDisposable
{
    private const int ArrowRefreshDelayMs = 60;   // 탐색기가 방향키를 처리해 선택을 옮길 시간
    private const int ChangeCoalesceDelayMs = 30; // SelectionChanged 연속 발생 묶기

    private readonly IHelperSink _sink;
    private readonly StaDispatcher _sta = new();
    private readonly ShellSelection _shell = new();
    private readonly SelectionWatcher _watcher;
    private readonly KeyboardHook _hook;
    private readonly HashSet<uint> _swallowed = new(); // 훅 스레드 전용: 삼킨 키의 key-up/자동반복도 삼키기 위함

    private volatile bool _enabled = true;
    private volatile bool _previewOpen;
    private volatile FileViewTarget? _lastTarget;
    private SelectionResult? _lastSent; // STA 전용
    private bool _refreshPending;       // STA 전용

    public HelperCore(IHelperSink sink)
    {
        _sink = sink;
        _watcher = new SelectionWatcher(OnSelectionChangedEvent);
        _hook = new KeyboardHook(OnKey, FocusInspector.PrimeForWindow);
    }

    public bool PreviewOpen => _previewOpen;

    public void Start()
    {
        _hook.Start();
        FocusInspector.PrimeForWindow(PInvoke.GetForegroundWindow());
        // 첫 Space 의 지연을 줄이려고 COM 객체 생성·JIT 를 미리 한 번 거친다(실측: 첫 조회 100~200ms → 이후 ~10ms)
        _sta.Post(() =>
        {
            try { _shell.EnumerateAll(); }
            catch (Exception e) { Log.Warn($"warmup failed: {e.Message}"); }
        });
    }

    public void SetEnabled(bool enabled) => _enabled = enabled;

    public void SetPreviewOpen(bool open)
    {
        _previewOpen = open;
        _sta.Post(() =>
        {
            if (open)
            {
                var target = FocusInspector.GetFileViewTarget() ?? _lastTarget;
                if (target is not null) TryWatch(target);
            }
            else
            {
                _watcher.Detach();
                _lastSent = null;
            }
        });
    }

    public void RequestSelection(int? id) => _sta.Post(() =>
    {
        var target = FocusInspector.GetFileViewTarget() ?? _lastTarget;
        if (target is null)
        {
            _sink.Error(id, ErrorCodes.NoView, "Foreground window is not an Explorer file view");
            return;
        }
        try
        {
            var sel = _shell.Query(target);
            _lastSent = sel;
            _sink.Selection(id, sel);
            if (_previewOpen) TryWatch(target);
        }
        catch (ShellAccessException e)
        {
            Log.Warn($"selection query failed: {e.Code} {e.Message}");
            _sink.Error(id, e.Code, e.Message);
        }
        catch (Exception e)
        {
            // 응답이 없으면 호스트가 열기 대기 상태로 남으므로 반드시 오류로라도 답한다
            Log.Error($"selection query crashed: {e}");
            _sink.Error(id, ErrorCodes.Internal, e.Message);
        }
    });

    /// <summary>STA 스레드에서 응답 → 헬퍼 생존 확인이 STA 정지까지 잡아낸다.</summary>
    public void Ping(Action reply) => _sta.Post(reply);

    // ---- 훅 스레드 ----

    private bool OnKey(uint vk, bool isKeyUp)
    {
        var key = MapKey(vk);
        if (key is null) return false;

        if (isKeyUp) return _swallowed.Remove(vk);
        if (_swallowed.Contains(vk)) return true; // 삼킨 키의 자동 반복

        if (!_enabled || ModifierHeld(key.Value)) return false;

        bool open = _previewOpen;
        var target = FocusInspector.GetFileViewTarget();
        if (target is null)
        {
            // 미리보기가 열린 채 포커스가 탭 표시줄·검색창 등으로 옮겨 갔을 때도 Esc 로 닫을 수 있게 한다.
            // 그 입력란의 Esc 동작을 막지 않도록 키는 통과시킨다.
            if (open && key.Value == KeyName.Esc && FocusInspector.IsShellForeground())
                _sink.Key(KeyName.Esc, false);
            return false;
        }
        if (FocusInspector.IsElevatedCached(target.ProcessId)) return false; // 선택을 읽을 수 없으니 탐색기에 맡긴다

        bool consume, emit;
        switch (key.Value)
        {
            case KeyName.Space: consume = true; emit = true; break;
            case KeyName.Esc: consume = open; emit = open; break;
            default: consume = false; emit = open; break; // 방향키·Enter 는 탐색기가 처리하도록 통과
        }

        if (emit)
        {
            _lastTarget = target;
            _sink.Key(key.Value, consume);
            if (open && key.Value is KeyName.Up or KeyName.Down or KeyName.Left or KeyName.Right)
                _sta.PostDelayed(ArrowRefreshDelayMs, RefreshSelection);
        }
        if (consume) _swallowed.Add(vk);
        return consume;
    }

    private static KeyName? MapKey(uint vk) => (VIRTUAL_KEY)vk switch
    {
        VIRTUAL_KEY.VK_SPACE => KeyName.Space,
        VIRTUAL_KEY.VK_ESCAPE => KeyName.Esc,
        VIRTUAL_KEY.VK_RETURN => KeyName.Enter,
        VIRTUAL_KEY.VK_UP => KeyName.Up,
        VIRTUAL_KEY.VK_DOWN => KeyName.Down,
        VIRTUAL_KEY.VK_LEFT => KeyName.Left,
        VIRTUAL_KEY.VK_RIGHT => KeyName.Right,
        _ => null,
    };

    /// <summary>Ctrl/Alt/Win 조합은 탐색기 단축키이므로 건드리지 않는다. Shift 는 Space 일 때만 제외(Shift+방향키 = 선택 확장).</summary>
    private static bool ModifierHeld(KeyName key)
    {
        static bool Down(VIRTUAL_KEY k) => (PInvoke.GetAsyncKeyState((int)k) & 0x8000) != 0;
        if (Down(VIRTUAL_KEY.VK_CONTROL) || Down(VIRTUAL_KEY.VK_MENU) || Down(VIRTUAL_KEY.VK_LWIN) || Down(VIRTUAL_KEY.VK_RWIN))
            return true;
        return key == KeyName.Space && Down(VIRTUAL_KEY.VK_SHIFT);
    }

    // ---- STA 스레드 ----

    private void OnSelectionChangedEvent()
    {
        if (_refreshPending) return;
        _refreshPending = true;
        _sta.PostDelayed(ChangeCoalesceDelayMs, RefreshSelection);
    }

    /// <summary>미리보기가 열려 있을 때 현재 선택을 다시 읽고, 바뀌었으면 알린다(id 없는 selection).</summary>
    private void RefreshSelection()
    {
        _refreshPending = false;
        if (!_previewOpen) return;
        var target = FocusInspector.GetFileViewTarget();
        if (target is null) return;
        try
        {
            var sel = _shell.Query(target);
            TryWatch(target); // 폴더 이동으로 뷰가 바뀌었으면 새 뷰를 구독
            if (sel.SameAs(_lastSent)) return;
            _lastSent = sel;
            _sink.Selection(null, sel);
        }
        catch (ShellAccessException e)
        {
            Log.Warn($"selection refresh failed: {e.Code} {e.Message}");
        }
    }

    private void TryWatch(FileViewTarget target)
    {
        if (_watcher.IsWatching(target.ViewWindow)) return;
        try
        {
            _watcher.Attach(_shell.FindView(target), target.ViewWindow);
        }
        catch (ShellAccessException e)
        {
            Log.Warn($"watch failed: {e.Code} {e.Message}");
        }
    }

    public void Dispose()
    {
        _hook.Dispose();
        _sta.Post(_watcher.Detach);
        _sta.Dispose();
    }
}
