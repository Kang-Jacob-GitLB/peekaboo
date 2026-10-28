using System.Text.Json;
using Peekaboo.Helper.Shell;
using Windows.Win32.Foundation;

namespace Peekaboo.Helper.Ipc;

// 프로토콜 정의: docs/ipc.md, TS 대응 타입: src/shared/ipc-types.ts

internal static class Protocol
{
    public const int Version = 1;
}

internal enum KeyName { Space, Esc, Enter, Up, Down, Left, Right }

internal enum SelectionSource { Explorer, Desktop }

internal sealed record SelectionResult(string[] Paths, string? Focused, SelectionSource Source, nint Hwnd)
{
    public bool SameAs(SelectionResult? other) =>
        other is not null && other.Hwnd == Hwnd && other.Focused == Focused && other.Paths.AsSpan().SequenceEqual(Paths);
}

internal static class ErrorCodes
{
    public const string Elevated = "ELEVATED";
    public const string NoView = "NO_VIEW";
    public const string ComFail = "COM_FAIL";
    public const string BadMessage = "BAD_MESSAGE";
    public const string Internal = "INTERNAL";
}

/// <summary>Helper -> Electron 메시지 작성기</summary>
internal static class Outgoing
{
    public static string ToWire(KeyName k) => k switch
    {
        KeyName.Space => "space",
        KeyName.Esc => "esc",
        KeyName.Enter => "enter",
        KeyName.Up => "up",
        KeyName.Down => "down",
        KeyName.Left => "left",
        _ => "right",
    };

    public static void Ready(JsonLineChannel ch, string version) => ch.Send(w =>
    {
        w.WriteString("type", "ready");
        w.WriteString("version", version);
        w.WriteNumber("pid", Environment.ProcessId);
    });

    public static void Key(JsonLineChannel ch, KeyName key, bool consumed) => ch.Send(w =>
    {
        w.WriteString("type", "key");
        w.WriteString("key", ToWire(key));
        w.WriteBoolean("consumed", consumed);
    });

    public static void Selection(JsonLineChannel ch, int? id, SelectionResult sel) => ch.Send(w =>
    {
        w.WriteString("type", "selection");
        if (id is int i) w.WriteNumber("id", i);
        w.WriteStartArray("paths");
        foreach (var p in sel.Paths) w.WriteStringValue(p);
        w.WriteEndArray();
        if (sel.Focused is null) w.WriteNull("focused"); else w.WriteString("focused", sel.Focused);
        w.WriteString("source", sel.Source == SelectionSource.Desktop ? "desktop" : "explorer");
        w.WriteNumber("hwnd", (long)sel.Hwnd);
    });

    public static void Pong(JsonLineChannel ch, int id) => ch.Send(w =>
    {
        w.WriteString("type", "pong");
        w.WriteNumber("id", id);
    });

    public static void NativePreview(JsonLineChannel ch, int id, NativePreviewStatus status, string? message) => ch.Send(w =>
    {
        w.WriteString("type", "nativePreview");
        w.WriteNumber("id", id);
        w.WriteString("status", status switch
        {
            NativePreviewStatus.Shown => "shown",
            NativePreviewStatus.Unavailable => "unavailable",
            _ => "failed",
        });
        if (message is not null) w.WriteString("message", message);
    });

    public static void Error(JsonLineChannel ch, int? id, string code, string message) => ch.Send(w =>
    {
        w.WriteString("type", "error");
        if (id is int i) w.WriteNumber("id", i);
        w.WriteString("code", code);
        w.WriteString("message", message);
    });
}

/// <summary>Electron -> Helper 메시지</summary>
internal abstract record HostCommand
{
    public sealed record Config(bool Enabled) : HostCommand;
    public sealed record State(bool PreviewOpen) : HostCommand;
    public sealed record GetSelection(int Id) : HostCommand;
    public sealed record Ping(int Id) : HostCommand;
    public sealed record Shutdown : HostCommand;
    public sealed record Unknown(string Type) : HostCommand;
    /// <param name="Rect">미리보기 창 클라이언트 영역 기준 물리 픽셀</param>
    public sealed record NativePreviewOpen(int Id, string Path, nint Hwnd, RECT Rect, bool Dark) : HostCommand;
    public sealed record NativePreviewResize(int Id, RECT Rect) : HostCommand;
    public sealed record NativePreviewClose(int Id) : HostCommand;

    /// <exception cref="FormatException">JSON 이 아니거나 필수 필드가 없을 때</exception>
    public static HostCommand Parse(string line)
    {
        try
        {
            using var doc = JsonDocument.Parse(line);
            var root = doc.RootElement;
            var type = root.GetProperty("type").GetString() ?? "";
            return type switch
            {
                "config" => new Config(root.GetProperty("enabled").GetBoolean()),
                "state" => new State(root.GetProperty("previewOpen").GetBoolean()),
                "getSelection" => new GetSelection(root.GetProperty("id").GetInt32()),
                "ping" => new Ping(root.GetProperty("id").GetInt32()),
                "shutdown" => new Shutdown(),
                "nativePreview.open" => new NativePreviewOpen(
                    root.GetProperty("id").GetInt32(),
                    root.GetProperty("path").GetString() ?? throw new FormatException("path is null"),
                    (nint)root.GetProperty("hwnd").GetInt64(),
                    ParseRect(root.GetProperty("rect")),
                    root.GetProperty("dark").GetBoolean()),
                "nativePreview.resize" => new NativePreviewResize(root.GetProperty("id").GetInt32(), ParseRect(root.GetProperty("rect"))),
                "nativePreview.close" => new NativePreviewClose(root.GetProperty("id").GetInt32()),
                _ => new Unknown(type),
            };
        }
        catch (Exception e) when (e is JsonException or KeyNotFoundException or InvalidOperationException or FormatException)
        {
            throw new FormatException(e.Message, e);
        }
    }

    /// <summary>{x, y, width, height} → RECT(left, top, right, bottom). 소수는 반올림.</summary>
    private static RECT ParseRect(JsonElement r)
    {
        int x = (int)Math.Round(r.GetProperty("x").GetDouble());
        int y = (int)Math.Round(r.GetProperty("y").GetDouble());
        int w = (int)Math.Round(r.GetProperty("width").GetDouble());
        int h = (int)Math.Round(r.GetProperty("height").GetDouble());
        if (w < 0 || h < 0) throw new FormatException("negative rect size");
        return new RECT { left = x, top = y, right = x + w, bottom = y + h };
    }
}
