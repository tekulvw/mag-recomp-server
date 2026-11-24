using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text.Json.Nodes;
using Server.Medius;

// Live gameplay workers stalled for two to four seconds on a busy disk. The
// bridge's former two-second event and three-second world-frame deadlines turned
// each stall into lost replies or a world-stream gap that released every player.
// A synthetic loopback worker stands in for the stalled one.
internal static class BridgeDeadlineChecks
{
    static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }

    public static void Run()
    {
        Check(GameplayBridge.EventDeadline >= TimeSpan.FromSeconds(6), "Event deadline is shorter than a live stall");
        Check(GameplayBridge.WorldFrameDeadline >= TimeSpan.FromSeconds(12), "World frame deadline is shorter than a live stall");
        Check(GameplayBridge.RequestDeadline == TimeSpan.FromSeconds(2), "Control request deadline changed");
        Task.WhenAll(SlowEventReplyIsAnswered(), SilentWorkerStillTimesOut(), WorldStreamSurvivesAStall(),
            ResolvedAddressIsCachedUntilAConnectionFails()).GetAwaiter().GetResult();
    }

    static object Event(string id) => new { v = 2, request_id = id, op = "event" };

    static string Reply(string id) => new JsonObject {
        ["v"] = 2, ["request_id"] = id, ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray()
    }.ToJsonString();

    /// <summary>Loopback worker that answers each connection with <paramref name="answer"/>.</summary>
    sealed class Worker : IAsyncDisposable
    {
        readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        readonly CancellationTokenSource _stop = new();
        readonly Task _loop;
        public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;
        public int Connections;

        public Worker(Func<string, StreamWriter, CancellationToken, Task> answer)
        {
            _listener.Start();
            _loop = Task.Run(async () =>
            {
                var connections = new List<Task>();
                try
                {
                    while (true)
                    {
                        var socket = await _listener.AcceptTcpClientAsync(_stop.Token);
                        Interlocked.Increment(ref Connections);
                        connections.Add(Task.Run(async () =>
                        {
                            using (socket)
                            using (var stream = socket.GetStream())
                            using (var reader = new StreamReader(stream))
                            using (var writer = new StreamWriter(stream) { AutoFlush = true })
                            {
                                try
                                {
                                    var line = await reader.ReadLineAsync(_stop.Token);
                                    var id = JsonNode.Parse(line!)!["request_id"]!.GetValue<string>();
                                    await answer(id, writer, _stop.Token);
                                }
                                catch (OperationCanceledException) { }
                                catch (IOException) { }
                            }
                        }));
                    }
                }
                catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
                await Task.WhenAll(connections);
            });
        }

        public async ValueTask DisposeAsync()
        {
            _stop.Cancel();
            try { await _loop.WaitAsync(TimeSpan.FromSeconds(15)); }
            finally { _listener.Stop(); _stop.Dispose(); }
        }
    }

    static async Task SlowEventReplyIsAnswered()
    {
        await using var worker = new Worker(async (id, writer, stop) =>
        {
            await Task.Delay(TimeSpan.FromSeconds(3), stop); // A stalled world lock.
            await writer.WriteLineAsync(Reply(id).AsMemory(), stop);
        });
        var bridge = new GameplayBridge("127.0.0.1", worker.Port);
        var reply = await bridge.RequestEventAsync(Event("synthetic-slow-event"));
        Check(reply.Ok && reply.Failure == GameplayBridgeFailure.None,
            $"A three-second stall lost the event reply: failure={reply.Failure}");
    }

    static async Task SilentWorkerStillTimesOut()
    {
        await using var worker = new Worker((id, writer, stop) => Task.Delay(Timeout.Infinite, stop));
        var bridge = new GameplayBridge("127.0.0.1", worker.Port);
        var elapsed = Stopwatch.StartNew();
        var reply = await bridge.RequestEventAsync(Event("synthetic-silent-event"));
        Check(reply.Failure == GameplayBridgeFailure.Timeout, $"Silent worker returned {reply.Failure}");
        Check(elapsed.Elapsed >= TimeSpan.FromSeconds(5.5) && elapsed.Elapsed < TimeSpan.FromSeconds(9),
            $"Event deadline was not bounded near six seconds: {elapsed.ElapsedMilliseconds} ms");
        var control = await bridge.RequestControlAsync(new { v = 2, request_id = "synthetic-control", op = "configure_match" });
        Check(control.Failure == GameplayBridgeFailure.Timeout && control.ElapsedMilliseconds < 4000,
            $"Control requests lost their two-second deadline: {control.ElapsedMilliseconds} ms");
    }

    static async Task WorldStreamSurvivesAStall()
    {
        await using var worker = new Worker(async (id, writer, stop) =>
        {
            await writer.WriteLineAsync(Reply(id).AsMemory(), stop);   // Acknowledgement.
            await writer.WriteLineAsync(Reply(id).AsMemory(), stop);   // First frame.
            await Task.Delay(TimeSpan.FromSeconds(4), stop);           // Engine stall.
            await writer.WriteLineAsync(Reply(id).AsMemory(), stop);   // Frames resume.
        });                                                            // Then the worker closes.
        var bridge = new GameplayBridge("127.0.0.1", worker.Port);
        int frames = 0;
        Exception ended = null;
        try
        {
            await bridge.SubscribeWorldAsync(7, "synthetic-world-stream", _ => { frames++; return Task.CompletedTask; },
                CancellationToken.None).WaitAsync(TimeSpan.FromSeconds(20));
        }
        catch (Exception error) { ended = error; }
        Check(frames == 3, $"World stream ended during a four-second stall after {frames} frames");
        Check(ended is IOException, $"World stream ended with {ended?.GetType().Name ?? "no error"} instead of the close");
    }

    static async Task ResolvedAddressIsCachedUntilAConnectionFails()
    {
        await using var worker = new Worker((id, writer, stop) => writer.WriteLineAsync(Reply(id).AsMemory(), stop));
        // A port with no listener stands in for an address the worker has left.
        var unused = new TcpListener(IPAddress.Loopback, 0);
        unused.Start();
        int stalePort = ((IPEndPoint)unused.LocalEndpoint).Port;
        unused.Stop();

        var bridge = new GameplayBridge("synthetic-gameplay.invalid", worker.Port);
        int lookups = 0;
        var answers = new Queue<IPAddress[]>(new[] {
            new[] { IPAddress.Loopback }, new[] { IPAddress.Loopback }
        });
        Func<string, CancellationToken, Task<IPAddress[]>> resolve = (host, token) =>
        {
            Check(host == "synthetic-gameplay.invalid", "Resolver received another name");
            Interlocked.Increment(ref lookups);
            return Task.FromResult(answers.Dequeue());
        };
        var field = typeof(GameplayBridge).GetField("_resolve", BindingFlags.Instance | BindingFlags.NonPublic)!;
        field.SetValue(bridge, resolve);
        for (int i = 0; i < 4; i++)
            Check((await bridge.RequestEventAsync(Event($"synthetic-cached-{i}"))).Ok, "Cached address request failed");
        Check(lookups == 1, $"Each request resolved the worker name again: {lookups} lookups");

        // The cached answer now names an address that refuses connections; the
        // failure forgets it and the next request resolves the name again.
        var cache = typeof(GameplayBridge).GetField("_addresses", BindingFlags.Instance | BindingFlags.NonPublic)!;
        var stale = Activator.CreateInstance(cache.FieldType, new object[] { new[] { IPAddress.Loopback }, long.MaxValue })!;
        cache.SetValue(bridge, stale);
        var staleBridge = typeof(GameplayBridge).GetField("_port", BindingFlags.Instance | BindingFlags.NonPublic)!;
        int port = (int)staleBridge.GetValue(bridge)!;
        staleBridge.SetValue(bridge, stalePort);
        var refused = await bridge.RequestEventAsync(Event("synthetic-refused"));
        Check(refused.IsTransientFailure, $"Refused connection reported {refused.Failure}");
        Check(cache.GetValue(bridge) == null, "A failed connection kept its cached address");
        staleBridge.SetValue(bridge, port);
        Check((await bridge.RequestEventAsync(Event("synthetic-resolved-again"))).Ok, "Request after a failure did not resolve again");
        Check(lookups == 2, $"Expected one fresh lookup after the failure, saw {lookups - 1}");
        Check(worker.Connections == 5, $"Worker saw {worker.Connections} connections");
    }
}
