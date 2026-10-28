using System.Buffers;
using System.Collections.Concurrent;
using System.Text;
using System.Text.Json;

namespace Peekaboo.Helper.Ipc;

/// <summary>
/// stdout 으로 JSON Lines 를 내보내는 채널. 호출 스레드(특히 키보드 훅 스레드)를 막지 않도록
/// 직렬화된 줄을 큐에 넣고 전용 스레드가 기록한다.
/// </summary>
internal sealed class JsonLineChannel : IDisposable
{
    private readonly BlockingCollection<byte[]> _queue = new();
    private readonly Stream _out;
    private readonly Thread _writer;

    public JsonLineChannel()
    {
        _out = Console.OpenStandardOutput();
        _writer = new Thread(WriteLoop) { IsBackground = true, Name = "ipc-writer" };
        _writer.Start();
    }

    public void Send(Action<Utf8JsonWriter> body)
    {
        var buf = new ArrayBufferWriter<byte>(256);
        using (var w = new Utf8JsonWriter(buf))
        {
            w.WriteStartObject();
            w.WriteNumber("v", Protocol.Version);
            body(w);
            w.WriteEndObject();
        }
        var line = new byte[buf.WrittenCount + 1];
        buf.WrittenSpan.CopyTo(line);
        line[^1] = (byte)'\n';
        if (!_queue.IsAddingCompleted) _queue.Add(line);
    }

    private void WriteLoop()
    {
        try
        {
            foreach (var line in _queue.GetConsumingEnumerable())
            {
                _out.Write(line);
                _out.Flush();
            }
        }
        catch (IOException)
        {
            // 상대(Electron)가 파이프를 닫음 — 종료 흐름은 stdin EOF 쪽에서 처리
        }
    }

    public void Dispose()
    {
        _queue.CompleteAdding();
        _writer.Join(1000);
    }

    /// <summary>stdin 에서 한 줄씩 읽는다. EOF(null)는 호스트가 사라졌다는 뜻.</summary>
    public static StreamReader OpenInput() =>
        new(Console.OpenStandardInput(), new UTF8Encoding(false), false, 4096);
}
