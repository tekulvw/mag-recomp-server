using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json.Nodes;
using Server.Medius;

// Invented reservations and payload bytes only (reply_budget_per_recipient_v1).
// A worker that gives each recipient its own allowance of shared frames sends
// replies above the legacy 128-message / 24,000-byte bound, which Horizon
// accepts only after that worker's hello advertised the capability.
internal static class ReplyBudgetChecks
{
    static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }

    static JsonObject Reply(string id, int messages, int bytes, params string[] capabilities)
    {
        var list = new JsonArray();
        for (int i = 0; i < messages; i++)
            list.Add(new JsonObject { ["reservation_id"] = $"synthetic-reservation-{i % 32:D4}", ["world_id"] = 1,
                ["player_id"] = i % 32, ["plugin_id"] = 1, ["payload_hex"] = Convert.ToHexString(new byte[bytes]) });
        var reply = new JsonObject { ["v"] = 2, ["request_id"] = id, ["ok"] = true, ["error"] = null, ["messages"] = list };
        if (capabilities.Length > 0) reply["capabilities"] = new JsonArray(capabilities.Select(c => (JsonNode)c).ToArray());
        return reply;
    }
    static bool Parse(JsonObject reply, GameplayReplyLimits limits) =>
        GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(reply.ToJsonString()), "synthetic", true, limits, out _);

    public static void Codec()
    {
        var legacy = GameplayReplyLimits.Legacy;
        var wide = GameplayReplyLimits.PerRecipient;
        Check(legacy == new GameplayReplyLimits(128, 24000, 131072), "Legacy reply bounds changed");
        Check(GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(Reply("synthetic", 128, 187).ToJsonString()),
            "synthetic", true, out _), "The legacy overload refused a legacy-sized reply");
        // 32 recipients x 16 messages of 384 bytes: 512 messages and 196,608 bytes.
        var shared = Reply("synthetic", 512, 384);
        Check(!Parse(shared, legacy), "A per-recipient reply passed the legacy bounds");
        Check(!GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(shared.ToJsonString()), "synthetic", true, out _),
            "The legacy overload accepted a per-recipient reply");
        Check(Parse(shared, wide), "A full per-recipient reply was refused");
        Check(Parse(Reply("synthetic", 1024, 256), wide), "The per-recipient message bound was refused");
        Check(!Parse(Reply("synthetic", 1025, 1), wide), "More than 1,024 messages were accepted");
        Check(!Parse(Reply("synthetic", 189, 1397), wide), "More than 262,144 payload bytes were accepted");
        Check(!Parse(Reply("synthetic", 1, 1398), wide), "A message above 1,397 bytes was accepted");
        Check(Encoding.UTF8.GetByteCount(Reply("synthetic", 1024, 256).ToJsonString()) < wide.ResponseBytes,
            "A full per-recipient reply does not fit the response bound");
    }

    /// <summary>Loopback worker: answers hello with <paramref name="hello"/>, every other request with a
    /// 512-message reply, and the world subscription with that reply as acknowledgement and frame.</summary>
    static async Task<(GameplayBridgeResponse hello, GameplayBridgeResponse wide, Exception stream)> Exchange(JsonObject hello)
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        using var stop = new CancellationTokenSource(TimeSpan.FromSeconds(20));
        var loop = Task.Run(async () =>
        {
            while (!stop.IsCancellationRequested)
            {
                TcpClient socket;
                try { socket = await listener.AcceptTcpClientAsync(stop.Token); } catch (OperationCanceledException) { return; }
                _ = Task.Run(async () =>
                {
                    using (socket)
                    using (var stream = socket.GetStream())
                    using (var reader = new StreamReader(stream))
                    using (var writer = new StreamWriter(stream) { AutoFlush = true })
                    {
                        try
                        {
                            var request = JsonNode.Parse((await reader.ReadLineAsync(stop.Token))!)!;
                            var id = request["request_id"]!.GetValue<string>();
                            var op = request["op"]!.GetValue<string>();
                            if (op == "hello") { var answer = hello.DeepClone(); answer["request_id"] = id; await writer.WriteLineAsync(answer.ToJsonString()); return; }
                            var wide = Reply(id, 512, 384).ToJsonString();
                            await writer.WriteLineAsync(op == "subscribe_world" ? Reply(id, 0, 1).ToJsonString() : wide);
                            if (op == "subscribe_world") await writer.WriteLineAsync(wide);
                        }
                        catch (Exception error) when (error is IOException || error is OperationCanceledException) { }
                    }
                });
            }
        });
        try
        {
            var bridge = new GameplayBridge("127.0.0.1", ((IPEndPoint)listener.LocalEndpoint).Port);
            var helloReply = await bridge.RequestHelloAsync(new { v = 2, request_id = "synthetic-hello", op = "hello" });
            var event_ = await bridge.RequestEventAsync(new { v = 2, request_id = "synthetic-event", op = "event" });
            Exception streamError = null; int frames = 0;
            try
            {
                await bridge.SubscribeWorldAsync(1, "synthetic-world", _ => { frames++; return Task.CompletedTask; }, stop.Token)
                    .WaitAsync(TimeSpan.FromSeconds(10));
            }
            catch (Exception error) { streamError = error; }
            Check(frames <= 2, "Unexpected world frames");
            return (helloReply, event_, frames == 2 && streamError is IOException ? null : streamError);
        }
        finally { stop.Cancel(); listener.Stop(); await loop; }
    }

    public static void Bridge()
    {
        var (hello, wide, stream) = Exchange(Reply("x", 0, 1, GameplayReplyLimits.PerRecipientCapability)).GetAwaiter().GetResult();
        Check(hello.Ok && hello.SupportsPerRecipientReplyBudget, "The advertising hello was not parsed");
        Check(wide.Ok && wide.Messages.Count == 512, $"A per-recipient event reply was refused: {wide.Failure}");
        Check(stream == null, $"A per-recipient world frame was refused: {stream?.GetType().Name}: {stream?.Message}");

        (hello, wide, stream) = Exchange(Reply("x", 0, 1)).GetAwaiter().GetResult();
        Check(hello.Ok && !hello.SupportsPerRecipientReplyBudget, "A plain hello advertised per-recipient budgets");
        Check(!wide.Ok && wide.Failure == GameplayBridgeFailure.MalformedResponse,
            "A worker without the capability had a per-recipient reply accepted");
        Check(stream is InvalidDataException, $"A worker without the capability had a large world frame accepted: {stream?.GetType().Name}");
    }
}
