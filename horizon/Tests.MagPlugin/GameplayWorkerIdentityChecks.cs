using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Server.Medius;

internal static class GameplayWorkerIdentityChecks
{
    private const string Identity = """{"worker_id":"alpha","incarnation_id":"start-1"}""";
    private const string RestartedIdentity = """{"worker_id":"alpha","incarnation_id":"start-2"}""";
    private const string OtherWorker = """{"worker_id":"beta","incarnation_id":"start-1"}""";
    private const string Capability = ""","capabilities":["worker_registry_v1"]""";
    private const string Message = """
        {"reservation_id":"synthetic-reservation","world_id":7,"player_id":2,"plugin_id":1,"payload_hex":"ab"}
        """;
    private const string Reward =
        ""","rewards":[{"reward_id":"synthetic-reward","reservation_id":"synthetic-reservation","match_id":"synthetic-match-123",""" +
        """ "world_id":7,"player_id":2,"account_id":"alpha","character_id":1,"kind":"enemy_killed","units":1}]""";

    public static void Run()
    {
        Codec();
        LegacyAndDiscovery().GetAwaiter().GetResult();
        StaleAcknowledgmentsAndFrames().GetAwaiter().GetResult();
    }

    private static void Codec()
    {
        Check(Parses(Response()), "Legacy bridge response rejected");
        Check(Parses(Response(Identity)), "Valid worker identity rejected");
        foreach (var malformed in new[] {
            "null", "{}", """{"worker_id":"alpha"}""",
            """{"worker_id":"alpha","incarnation_id":null}""",
            """{"worker_id":"alpha","incarnation_id":"bad space"}""",
            """{"worker_id":"alpha","worker_id":"alpha","incarnation_id":"start-1"}""",
            """{"worker_id":"alpha","incarnation_id":"start-1","extra":false}""",
            """{"worker_id":"alpha","incarnation_id":"é"}""",
            "{\"worker_id\":\"" + new string('a', 65) + "\",\"incarnation_id\":\"start-1\"}"
        })
            Check(!Parses(Response(malformed)), "Malformed worker metadata accepted");
        Check(!Parses(Response(extra: ""","unknown_field":false""")), "An unknown root field was accepted");
        Check(!Parses(Response(Identity, ",\"worker_identity\":" + Identity)),
            "Duplicate worker identity accepted");
    }

    private static async Task LegacyAndDiscovery()
    {
        using var listener = new Fixture();
        var legacy = listener.CreateBridge();
        var legacyHello = await listener.Exchange(legacy, "hello", null, Response());
        Check(legacyHello.Ok && legacyHello.WorkerIdentity == null && legacy.WorkerIdentity == null,
            "Legacy hello incorrectly required or pinned an identity");
        Check((await listener.Exchange(legacy, "progression_committed", null, Response())).Ok,
            "Legacy progression ACK was rejected");

        var configured = listener.CreateBridge("alpha");
        CheckIdentityFailure(await configured.RequestControlAsync(new { v = 2, request_id = "synthetic", op = "reserve" }));
        await RejectStream(configured, _ => Task.CompletedTask, listener.Token);
        Check(!listener.Pending, "An unbound bridge reached the worker outside of hello");

        var rejected = new[] { Response(), Response(Identity), Response(extra: Capability),
            Response(OtherWorker, Capability), Response(Identity, Capability, error: "discovery_unavailable") };
        foreach (var reply in rejected)
        {
            CheckIdentityFailure(await listener.Exchange(configured, "hello", null, reply));
            Check(configured.WorkerIdentity == null && !configured.HasWorkerIdentityFault,
                "An invalid hello established a binding or prevented discovery");
        }
        var hello = await listener.Exchange(configured, "hello", null, Response(Identity, Capability));
        Check(hello.Ok && hello.SupportsWorkerRegistry && configured.WorkerIdentity?.WorkerId == "alpha" &&
            configured.WorkerIdentity.IncarnationId == "start-1" && hello.WorkerIdentity == configured.WorkerIdentity,
            "Valid hello did not pin the configured worker incarnation");
    }

    private static async Task StaleAcknowledgmentsAndFrames()
    {
        using var listener = new Fixture();
        var bridge = await Bind(listener);
        Check((await listener.Exchange(bridge, "hello", Identity, Response(Identity, Capability))).Ok,
            "A repeated hello did not preserve the original binding");
        var accepted = await listener.Exchange(bridge, "progression_committed", Identity, Response(Identity, Reward, Message));
        Check(accepted.Ok && accepted.Messages.Count == 1 && accepted.Rewards.Count == 1,
            "An authenticated progression ACK lost its message or receipt");
        var refreshed = await listener.Exchange(bridge, "refresh_progression", Identity, Response(Identity));
        Check(refreshed.Ok && refreshed.Messages.Count == 0 && refreshed.Rewards.Count == 0,
            "An authenticated progression refresh did not accept its empty acknowledgment");
        var refused = await listener.Exchange(bridge, "progression_committed", Identity,
            Response(Identity, error: "invalid_progression_snapshot"));
        Check(!refused.Ok && refused.Failure == GameplayBridgeFailure.None && !bridge.HasWorkerIdentityFault,
            "A normal gameplay rejection faulted the worker identity");

        var forged = await bridge.RequestControlAsync(new
        {
            v = 2, request_id = "synthetic", op = "reserve",
            worker_identity = new { worker_id = "alpha", incarnation_id = "start-2" }
        });
        Check(forged.Failure == GameplayBridgeFailure.InvalidRequest && !listener.Pending,
            "Caller-supplied worker identity reached the worker");

        foreach (var (operation, reply) in new[] {
            ("progression_committed", Response()), ("progression_committed", Response(OtherWorker)),
            ("progression_committed", Response(RestartedIdentity, Reward, Message)),
            ("progression_committed", Response(RestartedIdentity, error: "worker_identity_mismatch")),
            ("progression_committed", Response(Identity, error: "worker_identity_required")),
            ("refresh_progression", Response(RestartedIdentity)),
            ("hello", Response(RestartedIdentity, Capability))
        })
        {
            bridge = await Bind(listener);
            CheckIdentityFailure(await listener.Exchange(bridge, operation, Identity, reply));
            Check(bridge.HasWorkerIdentityFault && bridge.WorkerIdentity.IncarnationId == "start-1",
                "A failed identity check did not preserve the fault and original binding");
            CheckIdentityFailure(await bridge.RequestHelloAsync(new { v = 2, request_id = "synthetic", op = "hello" }));
            Check(!listener.Pending, "A permanently faulted bridge attempted rediscovery");
        }

        var delivered = new List<GameplayBridgeResponse>();
        Task Receive(GameplayBridgeResponse reply)
        {
            delivered.Add(reply);
            return Task.CompletedTask;
        }
        bridge = await Bind(listener);
        var server = listener.Serve("subscribe_world", Identity, Response(RestartedIdentity));
        await RejectStream(bridge, Receive, listener.Token);
        await server;
        Check(delivered.Count == 0 && bridge.HasWorkerIdentityFault, "A stale subscription ACK reached delivery or did not fault the bridge");

        bridge = await Bind(listener);
        server = listener.Serve("subscribe_world", Identity,
            Response(Identity), Response(Identity, messages: Message), Response(RestartedIdentity, messages: Message));
        await RejectStream(bridge, Receive, listener.Token);
        await server;
        Check(bridge.HasWorkerIdentityFault && delivered.Count == 2 && delivered[0].Messages.Count == 0 && delivered[1].Messages.Count == 1 &&
            delivered[1].Messages[0].Payload.SequenceEqual(new byte[] { 0xab }),
            "Only the authenticated subscription ACK and tick should reach delivery");
    }

    private static async Task<GameplayBridge> Bind(Fixture listener)
    {
        var bridge = listener.CreateBridge("alpha");
        Check((await listener.Exchange(bridge, "hello", null, Response(Identity, Capability))).Ok, "Binding failed");
        return bridge;
    }

    private static async Task RejectStream(GameplayBridge bridge, Func<GameplayBridgeResponse, Task> receive, CancellationToken token)
    {
        try { await bridge.SubscribeWorldAsync(7, "synthetic", receive, token); }
        catch (GameplayWorkerIdentityException) { return; }
        throw new Exception("The subscription did not fail on an invalid worker identity");
    }

    private static void CheckIdentityFailure(GameplayBridgeResponse reply) =>
        Check(!reply.Ok && reply.Failure == GameplayBridgeFailure.WorkerIdentityMismatch && !reply.IsTransientFailure &&
            reply.WorkerIdentity == null && reply.Messages.Count == 0 && reply.Rewards.Count == 0,
            "An unauthenticated worker reply was not rejected at the bridge");

    private static string Response(string identity = null, string extra = "", string messages = "", string error = null) =>
        "{\"v\":2,\"request_id\":\"synthetic\",\"ok\":" + (error == null ? "true" : "false") +
        ",\"error\":" + JsonSerializer.Serialize(error) + ",\"messages\":[" + messages + "]" +
        (identity == null ? "" : ",\"worker_identity\":" + identity) + extra + "}";

    private static bool Parses(string reply) =>
        GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(reply), "synthetic", true, out _);

    private static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    private sealed class Fixture : IDisposable
    {
        private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        private readonly CancellationTokenSource _deadline = new(TimeSpan.FromSeconds(5));

        public Fixture() => _listener.Start();
        public CancellationToken Token => _deadline.Token;
        public bool Pending => _listener.Pending();

        public GameplayBridge CreateBridge(string expectedWorkerId = null)
        {
            int port = ((IPEndPoint)_listener.LocalEndpoint).Port;
            return expectedWorkerId == null
                ? new GameplayBridge("127.0.0.1", port)
                : new GameplayBridge("127.0.0.1", port, expectedWorkerId);
        }

        public async Task<GameplayBridgeResponse> Exchange(GameplayBridge bridge, string operation, string identity, string reply)
        {
            var server = Serve(operation, identity, reply);
            var response = await (operation switch
            {
                "hello" => bridge.RequestHelloAsync(new { v = 2, request_id = "synthetic", op = operation }),
                "refresh_progression" => bridge.RequestProgressionRefreshAsync(new
                {
                    v = 2, request_id = "synthetic", op = operation, reservation_id = "synthetic-reservation",
                    world_id = 7, player_id = 2, account_id = "alpha", character_id = 1,
                    progression = new { experience = 42, level = 1, skill_points = 0 }
                }),
                _ => bridge.RequestProgressionAsync(new
                {
                    v = 2, request_id = "synthetic", op = operation, reservation_id = "synthetic-reservation",
                    world_id = 7, player_id = 2, reward_id = "synthetic-reward", match_experience = 2,
                    progression = new { experience = 42, level = 1, skill_points = 0 }
                })
            });
            await server;
            return response;
        }

        public async Task Serve(string operation, string identity, params string[] replies)
        {
            using var socket = await _listener.AcceptTcpClientAsync(Token);
            using var stream = socket.GetStream();
            using var reader = new StreamReader(stream);
            using var writer = new StreamWriter(stream) { AutoFlush = true };
            using var document = JsonDocument.Parse(await reader.ReadLineAsync(Token));
            var root = document.RootElement;
            Check(root.GetProperty("op").GetString() == operation && (operation == "hello" || root.GetProperty("world_id").GetUInt32() == 7),
                "Worker request operation or world changed");
            if (operation is "progression_committed" or "refresh_progression")
                Check(root.GetProperty("progression").GetProperty("experience").GetUInt32() == 42,
                    "Worker request lost its progression payload");
            if (operation == "refresh_progression")
                Check(root.GetProperty("account_id").GetString() == "alpha" && root.GetProperty("character_id").GetUInt32() == 1 &&
                    !root.TryGetProperty("reward_id", out _) && !root.TryGetProperty("match_experience", out _),
                    "A progression refresh must carry the current character owner without a reward or match totals");
            bool hasIdentity = root.TryGetProperty("worker_identity", out var sent);
            Check(hasIdentity == (identity != null), "Worker identity presence in request was incorrect");
            if (hasIdentity)
            {
                using var expected = JsonDocument.Parse(identity);
                Check(JsonElement.DeepEquals(sent, expected.RootElement), "Request carried the wrong worker identity");
            }
            foreach (var reply in replies)
            {
                Check(!reply.Contains('\n'), "A synthetic JSONL reply contains a line break");
                await writer.WriteLineAsync(reply.AsMemory(), Token);
            }
        }

        public void Dispose()
        {
            _listener.Stop();
            _deadline.Dispose();
        }
    }
}
