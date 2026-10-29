using System.Reflection;
using Peekaboo.Helper.Ipc;

namespace Peekaboo.Helper;

internal static class Program
{
    private static readonly string Version =
        Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "0.0.0";

    private static int Main(string[] args)
    {
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            Log.Error($"unhandled: {e.ExceptionObject}");
            Environment.Exit(3); // Electron 이 감지해 재시작한다
        };

        if (args.Contains("--list")) return RunList();
        return args.Contains("--console") ? RunConsole() : RunProtocol();
    }

    /// <summary>진단: 열린 탐색기 창/바탕화면의 뷰 HWND 와 현재 선택을 한 번 출력하고 끝낸다.</summary>
    private static unsafe int RunList()
    {
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        int rc = 0;
        var t = new Thread(() =>
        {
            try
            {
                foreach (var (top, view, sel) in new Shell.ShellSelection().EnumerateAll())
                {
                    var cls = Input.FocusInspector.ClassOf(view);
                    Console.WriteLine($"{sel.Source} top=0x{(nint)top.Value:X} view=0x{(nint)view.Value:X} ({cls}) focused={sel.Focused ?? "-"}");
                    foreach (var p in sel.Paths) Console.WriteLine($"    {p}");
                }
            }
            catch (Exception e)
            {
                Console.WriteLine($"error: {e.Message}");
                rc = 1;
            }
        });
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
        t.Join();
        return rc;
    }

    /// <summary>Electron 과 stdin/stdout JSON Lines 로 통신한다. stdin EOF 면 종료.</summary>
    private static int RunProtocol()
    {
        using var channel = new JsonLineChannel();
        var sink = new ProtocolSink(channel);
        using var core = new HelperCore(sink);
        using var nativePreview = new Shell.NativePreviewHost((id, status, message) => Outgoing.NativePreview(channel, id, status, message));
        try
        {
            core.Start();
        }
        catch (Exception e)
        {
            Log.Error($"failed to install keyboard hook: {e.Message}");
            Outgoing.Error(channel, null, ErrorCodes.Internal, $"Keyboard hook failed: {e.Message}");
            return 2;
        }

        Outgoing.Ready(channel, Version);
        Log.Info($"helper {Version} ready (pid {Environment.ProcessId})");

        using var input = JsonLineChannel.OpenInput();
        string? line;
        while ((line = input.ReadLine()) is not null)
        {
            if (line.Length == 0) continue;
            HostCommand cmd;
            try
            {
                cmd = HostCommand.Parse(line);
            }
            catch (FormatException e)
            {
                Outgoing.Error(channel, null, ErrorCodes.BadMessage, e.Message);
                continue;
            }

            switch (cmd)
            {
                case HostCommand.Config c: core.SetEnabled(c.Enabled); break;
                case HostCommand.State s: core.SetPreviewOpen(s.PreviewOpen); break;
                case HostCommand.GetSelection g: core.RequestSelection(g.Id); break;
                case HostCommand.Ping p: core.Ping(() => Outgoing.Pong(channel, p.Id)); break;
                case HostCommand.NativePreviewOpen o: nativePreview.Open(o.Id, o.Path, o.Hwnd, o.Rect, o.Dark); break;
                case HostCommand.NativePreviewResize r: nativePreview.Resize(r.Id, r.Rect); break;
                case HostCommand.NativePreviewClose c: nativePreview.Close(c.Id); break;
                case HostCommand.Shutdown: return 0;
                case HostCommand.Unknown: break; // 전방 호환: 모르는 메시지는 무시
            }
        }

        Log.Info("stdin closed, exiting");
        return 0;
    }

    /// <summary>
    /// 단독 테스트 모드. Electron 없이 사람이 읽는 출력으로 동작하며, 미리보기 열림 상태를 스스로 흉내 낸다.
    /// Space = 열기(선택 출력)/닫기, 열린 상태에서 방향키 = 선택 변경 출력, Esc/Enter = 닫기.
    /// </summary>
    private static int RunConsole()
    {
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        var sink = new ConsoleSink();
        using var core = new HelperCore(sink);
        sink.Core = core;
        core.Start();

        Console.WriteLine($"peekaboo-helper {Version} — console mode");
        Console.WriteLine("탐색기나 바탕화면에서 파일을 선택하고 Space 를 누르세요. 종료: 이 창에서 Ctrl+C");
        using var exit = new ManualResetEventSlim();
        Console.CancelKeyPress += (_, e) => { e.Cancel = true; exit.Set(); };
        exit.Wait();
        return 0;
    }

    private sealed class ProtocolSink(JsonLineChannel channel) : IHelperSink
    {
        public void Key(KeyName key, bool consumed) => Outgoing.Key(channel, key, consumed);
        public void Selection(int? id, SelectionResult selection) => Outgoing.Selection(channel, id, selection);
        public void Error(int? id, string code, string message) => Outgoing.Error(channel, id, code, message);
        public void Foreground(uint pid) => Outgoing.Foreground(channel, pid);
    }

    private sealed class ConsoleSink : IHelperSink
    {
        public HelperCore? Core { get; set; }
        private int _nextId;

        public void Key(KeyName key, bool consumed)
        {
            Print($"key {Outgoing.ToWire(key)}{(consumed ? " (consumed)" : "")}");
            if (Core is null) return;
            switch (key)
            {
                case KeyName.Space when !Core.PreviewOpen:
                    Core.SetPreviewOpen(true);
                    Core.RequestSelection(Interlocked.Increment(ref _nextId));
                    Print("[preview OPEN]");
                    break;
                case KeyName.Space:
                case KeyName.Esc:
                case KeyName.Enter:
                    Core.SetPreviewOpen(false);
                    Print("[preview CLOSED]");
                    break;
            }
        }

        public void Selection(int? id, SelectionResult s)
        {
            var origin = id is null ? "changed" : $"reply #{id}";
            Print($"selection ({origin}, {s.Source}, hwnd=0x{s.Hwnd:X}) focused={s.Focused ?? "-"}");
            foreach (var p in s.Paths) Print($"    {p}");
            if (s.Paths.Length == 0) Print("    (선택 없음)");
        }

        public void Error(int? id, string code, string message) => Print($"error {code}: {message}");

        public void Foreground(uint pid)
        {
            Print($"foreground left shell (pid {pid})");
            if (Core is null || !Core.PreviewOpen) return;
            Core.SetPreviewOpen(false);
            Print("[preview CLOSED]");
        }

        private static void Print(string msg) => Console.WriteLine($"{DateTime.Now:HH:mm:ss.fff}  {msg}");
    }
}
