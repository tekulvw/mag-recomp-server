using System.Collections;
using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Text.Json.Nodes;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    private static readonly TimeSpan AdmissionTimeout = TimeSpan.FromSeconds(5);
    private const BindingFlags HostInternals = BindingFlags.Instance | BindingFlags.NonPublic;

    private sealed class FixtureBridge : IAsyncDisposable
    {
        private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        private readonly CancellationTokenSource _stop = new();
        private readonly Task _worker;
        public ConcurrentQueue<JsonObject> Requests { get; } = new();
        public GameplayBridge Client { get; }

        public FixtureBridge()
        {
            _listener.Start();
            Client = new GameplayBridge("127.0.0.1", ((IPEndPoint)_listener.LocalEndpoint).Port);
            _worker = AnswerAsync();
        }

        public JsonObject Latest(string operation) => Requests.Last(request => request["op"]!.GetValue<string>() == operation);

        private async Task AnswerAsync()
        {
            while (!_stop.IsCancellationRequested)
            {
                using var socket = await _listener.AcceptTcpClientAsync(_stop.Token);
                using var stream = socket.GetStream();
                using var reader = new StreamReader(stream);
                using var writer = new StreamWriter(stream) { AutoFlush = true };
                var request = JsonNode.Parse((await reader.ReadLineAsync(_stop.Token))!)!.AsObject();
                Requests.Enqueue(request);
                var response = new JsonObject {
                    ["v"] = 2, ["request_id"] = request["request_id"]!.DeepClone(), ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray()
                };
                await writer.WriteLineAsync(response.ToJsonString().AsMemory(), _stop.Token);
            }
        }

        public async ValueTask DisposeAsync()
        {
            _stop.Cancel();
            try { await _worker.WaitAsync(AdmissionTimeout); }
            catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            finally { _listener.Stop(); _stop.Dispose(); }
        }
    }

    private sealed class CoordinatedCommitter : ICharacterStoreCommitter, IDisposable
    {
        private readonly AtomicCharacterStoreCommitter _inner = new();
        private readonly ManualResetEventSlim _proceed = new();
        public TaskCompletionSource Entered { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public bool Pause { get; set; }
        public void Resume() => _proceed.Set();
        public void Dispose() => _proceed.Dispose();

        public void WriteAtomically(string path, byte[] contents)
        {
            if (Pause)
            {
                Entered.TrySetResult();
                if (!_proceed.Wait(AdmissionTimeout)) throw new IOException("Synthetic reward commit timed out");
            }
            _inner.WriteAtomically(path, contents);
        }
    }

    private static async Task WithAdmission(LocalCharacterStore store, Func<LocalDmeHost, FixtureBridge, MagMlsSession, Task> test)
    {
        var settings = Server.Medius.Program.Settings;
        int previousApp = settings.MagApplicationId;
        settings.MagApplicationId = 20231;
        try
        {
            await using var bridge = new FixtureBridge();
            await using var host = new LocalDmeHost(bridge.Client, store);
            // Supply only an in-memory world and readiness. StartAsync would open production listeners.
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            typeof(LocalDmeHost).GetProperty(nameof(LocalDmeHost.World))!.SetValue(host, new Server.Dme.Models.World(manager, 20231, 8));
            typeof(LocalDmeHost).GetField("_ready", HostInternals)!.SetValue(host, true);
            typeof(LocalDmeHost).GetField("_worldStreamHealthy", HostInternals)!.SetValue(host, true);

            var now = DateTime.UtcNow;
            var registry = new MagLoginRegistry();
            var ticket = registry.IssueForAccount("alpha", 1, 1, now);
            var session = new MagMlsSession { CharacterStore = store };
            var connect = new RT_MSG_CLIENT_CONNECT_TCP {
                AppId = 20231, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey
            };
            Check(session.TryBind(connect, registry, "synthetic-channel", now) && session.Accept(now) && session.CompleteTransport(now),
                "Synthetic MLS transport setup failed");
            foreach (var message in new MagMessage[] { new MagHello(), new MagProtocolInfo { ProtocolVersion = settings.MagProtocolVersion } })
            {
                byte[] payload = MagMessageCodec.SerializeForApplication(message, 20231);
                using var buffer = new MemoryStream();
                using (var writer = new BinaryWriter(buffer, Encoding.UTF8, leaveOpen: true))
                { writer.Write((ushort)payload.Length); writer.Write((byte)0); writer.Write(payload); }
                buffer.Position = 0;
                using var reader = new MessageReader(buffer);
                var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN(); envelope.Deserialize(reader);
                session.HandleMany(envelope, now);
            }
            Check(session.State == MagMlsSession.SessionState.Ready, "Synthetic authenticated MLS handshake failed");
            await test(host, bridge, session);
        }
        finally { settings.MagApplicationId = previousApp; }
    }

    private static Task<MagJoinGameResponse> BeginJoin(LocalDmeHost host, MagMlsSession session) =>
        host.JoinAsync(session, new MagRetailJoinGameRequest { Preference = 1, Value = (uint)Server.Medius.Program.Settings.MagGameType });

    private static Task InvokeHost(LocalDmeHost host, string method, object argument) =>
        (Task)typeof(LocalDmeHost).GetMethod(method, HostInternals)!.Invoke(host, new[] { argument })!;

    private static Task Release(LocalDmeHost host, JsonObject previous)
    {
        var owners = (IDictionary)typeof(LocalDmeHost).GetField("_reservations", HostInternals)!.GetValue(host)!;
        return InvokeHost(host, "ReleaseAsync", owners[previous["reservation_id"]!.GetValue<string>()]!);
    }

    public static void AdmissionCapability() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Legacy synthetic admission failed");
            var legacy = bridge.Latest("reserve");
            Check(legacy["character"]!["leadership_progression_enabled"]?.GetValue<bool>() == false, "XP-only catalog did not advertise disabled leadership progression");
            await Release(host, legacy).WaitAsync(AdmissionTimeout);
            store.ProgressionCatalog = Load(directory, Catalog());
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Complete-catalog synthetic admission failed");
            Check(bridge.Latest("reserve")["character"]!["leadership_progression_enabled"]?.GetValue<bool>() == true,
                "Complete authored catalog did not enable leadership progression on admission");
        }).GetAwaiter().GetResult();
    });

    public static void AdmissionRewardRace() => WithDirectory(directory =>
    {
        var fixture = Catalog();
        fixture["table"]!["Awards"]!["VictoryModifier"]!["MinorVictory"] = 1;
        using var committer = new CoordinatedCommitter();
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), committer) { ProgressionCatalog = Load(directory, fixture) };
        Create(store, "alpha", "Alpha");
        WithAdmission(store, async (host, bridge, session) =>
        {
            string match = $"world-{host.World.WorldId}-synthetic-match-123";
            Check(store.ApplyRewardForAccount("alpha", 1, "first-action", match, "enemy_killed", 1).Experience == 7, "Synthetic action XP missing");
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Initial synthetic admission failed");
            var initial = bridge.Latest("reserve");
            await Release(host, initial).WaitAsync(AdmissionTimeout);
            var response = RewardReply("match_minor_victory", """
              ,"leadership":{"role":1,"baseline":{"leadership_level":1,"nominations":1,"matches_since_last_command":0}}
              """);
            var reward = response["rewards"]![0]!;
            foreach (string key in new[] { "reservation_id", "world_id", "player_id" }) reward[key] = initial[key]!.DeepClone();
            Check(Parse(response, out var receipt), "Synthetic old-reservation reward rejected by bridge codec");
            committer.Pause = true;
            Task settling = Task.Run(() => InvokeHost(host, "SettleRewardsAsync", receipt));
            Task<MagJoinGameResponse> replacing;
            try
            {
                await committer.Entered.Task.WaitAsync(AdmissionTimeout);
                // The old join read before yielding for world allocation. The saved reward must win here.
                replacing = BeginJoin(host, session);
            }
            finally { committer.Resume(); }
            await settling.WaitAsync(AdmissionTimeout);
            Check((await replacing.WaitAsync(AdmissionTimeout)).Result == 0, "Replacement synthetic admission failed");
            var ack = bridge.Latest("progression_committed");
            Check(ack["progression"]!["experience"]!.GetValue<int>() == 14 && ack["leadership"]!["leadership_points"]!.GetValue<int>() == 35,
                "Synthetic reward did not commit and ACK both XP and leadership points");
            var replacement = bridge.Latest("reserve")["character"]!;
            Check(replacement["progression"]!["experience"]!.GetValue<int>() == 14 && replacement["leadership"]!["leadership_points"]!.GetValue<int>() == 35,
                "Reconnect reserve carried stale XP or leadership points across an in-flight settlement");
        }).GetAwaiter().GetResult();
    });
}
