namespace Peekaboo.Helper;

/// <summary>진단 로그는 stderr 로만 쓴다(stdout 은 프로토콜 전용). Electron 이 로그 파일에 합친다.</summary>
internal static class Log
{
    private static readonly object Gate = new();

    public static void Info(string msg) => Write("INFO", msg);
    public static void Warn(string msg) => Write("WARN", msg);
    public static void Error(string msg) => Write("ERROR", msg);

    private static void Write(string level, string msg)
    {
        lock (Gate)
        {
            Console.Error.WriteLine($"{DateTime.Now:HH:mm:ss.fff} [{level}] {msg}");
        }
    }
}
