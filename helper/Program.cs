namespace Peekaboo.Helper;

internal static class Program
{
    private static int Main(string[] args)
    {
        // 골격 단계: 프로토콜 ready 만 알리고 종료. 훅/COM 은 다음 단계에서 구현.
        Console.Out.WriteLine($"{{\"v\":1,\"type\":\"ready\",\"version\":\"0.1.0\",\"pid\":{Environment.ProcessId}}}");
        return 0;
    }
}
