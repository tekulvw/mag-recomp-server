using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Text.Json.Nodes;
using System.Threading.Channels;
using RT.Models;
using RT.Models.MAG;
using Server.Medius;

internal static class RegistryDmeHostChecks
{
    public static void Run() => LeadershipProgressionChecks.RegistryHostIsolation();
}

internal static partial class LeadershipProgressionChecks
{
    public static void RegistryHostIsolation() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1), ("charlie", (byte)2), ("delta", (byte)0) })
        {
            var create = new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] };
            Check(store.CreateForAccount(create, account).Result == MagNetResult.Success, "Synthetic faction creation failed");
        }
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var tcp = Server.Dme.Program.TcpServer;
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            var firstClock = new RegistryClock();
            using var progressionGate = new SemaphoreSlim(1, 1);
            await using var firstWorker = new RegistryWorker("first");
            await using var secondWorker = new RegistryWorker("second");
            await using var first = new LocalDmeHost(firstWorker.Client, store, Options("first", 1), firstClock, progressionGate);
            await using var second = new LocalDmeHost(secondWorker.Client, store, Options("second", 2), progressionGate: progressionGate);
            first.DurableProgressionCommitted += second.QueueProgressionRefresh;
            second.DurableProgressionCommitted += first.QueueProgressionRefresh;
            var drainingWorkers = new HashSet<string>(StringComparer.Ordinal);
            first.SetDrainPolicy(() => Volatile.Read(ref drainingWorkers).Contains("first"));
            second.SetDrainPolicy(() => Volatile.Read(ref drainingWorkers).Contains("second"));
            await Task.WhenAll(first.StartInRegistryAsync(manager), second.StartInRegistryAsync(manager));

            Check(first.World.WorldId != second.World.WorldId && first.World.Manager == manager && second.World.Manager == manager,
                "Registry workers did not create distinct registered worlds under the supplied manager");
            Check(first.GetPlacement().Profiles.Select(p => p.ProfileId).SequenceEqual(new[] { "profile-0-1", "profile-1-0" }),
                "Worker profiles did not respect configured factions");
            using var batch = first.BeginRegistryBatch();
            Check(second.GetPlacement().CanForm && !first.TryStageRegistryFormation("profile-0-2") &&
                first.TryStageRegistryFormation("profile-0-1"), "Registry accepted an unadvertised profile or declined a feasible formation");

            MagMlsSession Session(string account)
            {
                var settings = Server.Medius.Program.Settings;
                int previous = settings.MagApplicationId;
                settings.MagApplicationId = 20231; // The login fixture issues tickets for the active application.
                try { return MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id); }
                finally { settings.MagApplicationId = previous; }
            }
            var alpha = Session("alpha");
            int transaction = 0;
            MagJoinGameRequest Request() => new MagRetailJoinGameRequest { TransactionId = ++transaction, Preference = 1, Value = 2 };
            Task<MagJoinGameResponse> Queue(LocalDmeHost host, MagMlsSession session, MagJoinGameRequest request = null)
            {
                var reply = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                host.QueueJoin(session, request ?? Request(),
                    m => { if (m is MagJoinGameResponse response) reply.TrySetResult(response); }, () => true);
                return reply.Task;
            }

            var oldAlphaRequest = Request();
            var supersededAlpha = Queue(first, alpha, oldAlphaRequest);
            var alphaFirst = Queue(first, alpha, Request());
            first.CancelQueuedJoin(alpha, oldAlphaRequest);
            await first.TickWorldAsync();
            var pump = (Task)typeof(LocalDmeHost).GetField("_matchmakingWorker", HostInternals)!.GetValue(first)!;
            await pump.WaitAsync(AdmissionTimeout);
            Check(firstWorker.SelectedProfile == null && first.GetAccountOwnership("alpha") == LocalDmeAccountOwnership.Queued,
                "A concurrent tick consumed a partially forwarded registry formation");
            var bravo = Queue(first, Session("bravo"));
            Check(first.GetPlacement().PendingQueueCount == 2 && first.GetAccountOwnership("alpha") == LocalDmeAccountOwnership.Queued,
                "Forwarded tickets formed before the registry's worker tick or lacked ownership");
            Check(second.GetAccountOwnership("alpha") == LocalDmeAccountOwnership.None, "Queued ownership leaked across workers");
            batch.Dispose();
            await first.TickWorldAsync();
            var initial = await Task.WhenAll(alphaFirst, bravo).WaitAsync(AdmissionTimeout);
            Check(!supersededAlpha.IsCompleted && initial.All(join => join.Result == 0 && join.WorldId == (uint)first.World.WorldId && join.ServerPort == 12073) &&
                initial[0].ServerAddress.TakeWhile(b => b != 0).SequenceEqual("192.0.2.9"u8.ToArray()),
                "A superseded ticket replied or the current reply lost its actual world and configured public TCP endpoint");
            Check(firstWorker.SelectedProfile == "profile-0-1" && first.GetPlacement().LiveTeamCounts.SequenceEqual(new[] { 1, 1, 0 }),
                "Worker chose a different profile or failed to report reserved faction occupancy");
            Check(second.TryStageRegistryFormation("profile-0-2"), "Second worker could not stage a simultaneous match");
            var charlie = Queue(second, Session("charlie"));
            var deltaSession = Session("delta");
            var deltaRequest = Request();
            var delta = Queue(second, deltaSession, deltaRequest);
            await second.TickWorldAsync();
            var other = await Task.WhenAll(charlie, delta).WaitAsync(AdmissionTimeout);
            Check(other.All(join => join.Result == 0 && join.WorldId == (uint)second.World.WorldId) &&
                first.GetPlacement().LiveCount == 2 && second.GetPlacement().LiveTeamCounts.SequenceEqual(new[] { 1, 0, 1 }),
                "The other worker did not reserve an independent quorum while the first world was live");

            first.CancelQueuedJoin(alpha, oldAlphaRequest);
            Check(manager.GetClientBySessionKey(alpha.SessionKey) != null,
                "An old request cancelled a newer acknowledged reservation for the same lobby session");
            second.CancelQueuedJoin(deltaSession, deltaRequest);
            await WaitForRegistryHostAsync(() => manager.GetClientBySessionKey(deltaSession.SessionKey) == null);
            Check((second.GetAccountOwnership("delta") & LocalDmeAccountOwnership.Live) == 0,
                "The exact unclaimed acknowledged reservation survived cancellation");
            await second.TickWorldAsync();
            await WaitForRegistryHostAsync(() => second.GetAccountOwnership("delta") == LocalDmeAccountOwnership.None);

            var reserved = firstWorker.Reservations.ToArray();
            var oldAlpha = reserved.Single(r => r["account_id"]!.GetValue<uint>() == alpha.CharacterId);
            var client = manager.GetClientByAccessToken(oldAlpha["access_key"]!.GetValue<string>());
            Check(client != null && !second.TryBind(client, new RT_MSG_CLIENT_CONNECT_TCP_AUX_UDP
            {
                AppId = 20231, ARG1 = (uint)second.World.WorldId, SessionKey = client.SessionKey, AccessToken = client.Token
            }), "Another worker could claim a valid token for the wrong world");
            client = ConnectFixture(first, oldAlpha);
            Volatile.Write(ref drainingWorkers, new HashSet<string>(StringComparer.Ordinal) { "first" });
            Check(first.GetPlacement().Draining && !first.GetPlacement().CanForm && first.CanUse(client) && !second.GetPlacement().Draining,
                "Draining a worker disrupted an accepted member or left fresh placement enabled");

            await firstWorker.PublishAsync(TerminalReply("synthetic-first-match", reserved));
            await WaitForRegistryHostAsync(() => first.GetAccountOwnership("alpha") == LocalDmeAccountOwnership.Terminal);
            await first.ReleaseTerminalAccountAsync("alpha").WaitAsync(AdmissionTimeout);
            Check(manager.GetClientByAccessToken(client.Token) == null && client.IsDestroyed &&
                first.GetAccountOwnership("alpha") == LocalDmeAccountOwnership.Terminal,
                "Terminal transfer did not close the old token locally while retaining its release ownership");

            var alphaSecond = Queue(second, alpha);
            await second.TickWorldAsync();
            var successor = await alphaSecond.WaitAsync(AdmissionTimeout);
            Check(successor.Result == 0 && successor.WorldId == (uint)second.World.WorldId &&
                second.GetAccountOwnership("alpha") == LocalDmeAccountOwnership.Live &&
                manager.GetClientBySessionKey(alpha.SessionKey)?.DmeWorld == second.World,
                "A terminal session could not take a new reservation in the other worker's world");

            var newAlpha = secondWorker.Reservations.Single(r => r["account_id"]!.GetValue<uint>() == alpha.CharacterId);
            JsonObject OldReward()
            {
                var response = RewardReply("enemy_killed", "");
                var reward = response["rewards"]![0]!;
                foreach (string key in new[] { "reservation_id", "world_id", "player_id" }) reward[key] = oldAlpha[key]!.DeepClone();
                reward["match_id"] = "synthetic-first-match";
                return response;
            }
            async Task DriveReward(int acknowledgementCount)
            {
                await firstWorker.PublishAsync(OldReward());
                using var deadline = new CancellationTokenSource(AdmissionTimeout);
                while (firstWorker.Requests.Count(r => r["op"]!.GetValue<string>() == "progression_committed") < acknowledgementCount ||
                    !secondWorker.Requests.Any(r => r["op"]!.GetValue<string>() == "refresh_progression"))
                {
                    await first.TickWorldAsync();
                    await second.TickWorldAsync();
                    await Task.Delay(10, deadline.Token);
                }
            }
            await DriveReward(1);
            await DriveReward(2);
            var refresh = secondWorker.Requests.First(r => r["op"]!.GetValue<string>() == "refresh_progression");
            Check(store.ReadForAccount("alpha").Single().Experience == 7 && refresh["progression"]!["experience"]!.GetValue<uint>() == 7 &&
                refresh["reservation_id"]!.GetValue<string>() == newAlpha["reservation_id"]!.GetValue<string>() &&
                refresh["world_id"]!.GetValue<uint>() == (uint)second.World.WorldId && !refresh.ContainsKey("match_experience") &&
                firstWorker.Requests.Where(r => r["op"]!.GetValue<string>() == "progression_committed")
                    .All(r => r["reservation_id"]!.GetValue<string>() == oldAlpha["reservation_id"]!.GetValue<string>()),
                "A replayed old reward did not settle exactly once and refresh the exact new world without carrying match XP");

            Volatile.Write(ref drainingWorkers, new HashSet<string>(StringComparer.Ordinal));
            firstWorker.PauseHeartbeat = true;
            firstClock.ExpireLease();
            Check(!first.GetPlacement().Ready && second.GetPlacement().Ready, "A stale worker lease affected unrelated workers or admitted fresh work");
            firstWorker.IncarnationId = "replacement";
            await firstWorker.PublishAsync(CohortReply());
            await WaitForRegistryHostAsync(() => first.GetPlacement().Draining);
            first.SetDraining(false);
            Check(first.GetPlacement().Draining && !first.GetPlacement().Ready,
                "Clearing operator drain allowed a mismatched worker incarnation to resume");
            await first.DisposeAsync();
            Check(Server.Dme.Program.TcpServer == tcp && second.GetPlacement().Ready,
                "Disposing one registry worker affected the shared TCP server or another worker");
        }

        static LocalDmeHostOptions Options(string name, int opposingFaction) =>
            new(20231, 2, 2, 8, new[] { 1, 1 }, new[] { 0, opposingFaction }, IPAddress.Parse("192.0.2.9"), 12073, name);
    });

    private static async Task WaitForRegistryHostAsync(Func<bool> ready)
    {
        using var deadline = new CancellationTokenSource(AdmissionTimeout);
        while (!ready()) await Task.Delay(10, deadline.Token);
    }

    private sealed class RegistryClock : TimeProvider
    {
        private long _delay;
        public override long GetTimestamp() => base.GetTimestamp() + Interlocked.Read(ref _delay);
        public void ExpireLease() => Interlocked.Add(ref _delay, 4 * TimestampFrequency);
    }

    private sealed class RegistryWorker : IAsyncDisposable
    {
        private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        private readonly CancellationTokenSource _stop = new();
        private readonly Channel<JsonObject> _frames = Channel.CreateUnbounded<JsonObject>();
        private readonly string _workerId;
        private readonly Task _loop;
        public GameplayBridge Client { get; }
        public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;
        public ConcurrentQueue<JsonObject> Requests { get; } = new();
        public ConcurrentQueue<JsonObject> Reservations { get; } = new();
        public string SelectedProfile { get; private set; }
        public volatile bool PauseHeartbeat;
        public volatile string IncarnationId = "fixture-1";

        private static readonly JsonObject[] Profiles = (from byte attacker in new byte[] { 0, 1, 2 }
            from byte defender in new byte[] { 0, 1, 2 }
            where attacker != defender
            select new JsonObject { ["profile_id"] = $"profile-{attacker}-{defender}", ["game_type"] = 2,
                ["attacking_faction"] = attacker, ["defending_faction"] = defender,
                ["maximum_players"] = 8, ["team_capacity"] = 4 }).ToArray();

        public RegistryWorker(string workerId)
        {
            _workerId = workerId;
            _listener.Start();
            Client = new GameplayBridge("127.0.0.1", Port, workerId);
            _loop = RunAsync();
        }

        public ValueTask PublishAsync(JsonObject frame) => _frames.Writer.WriteAsync(frame, _stop.Token);

        private async Task RunAsync()
        {
            var connections = new List<Task>();
            try
            {
                while (true)
                {
                    var socket = await _listener.AcceptTcpClientAsync(_stop.Token);
                    connections.RemoveAll(task => task.IsCompletedSuccessfully);
                    connections.Add(AnswerAsync(socket));
                }
            }
            catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            finally { await Task.WhenAll(connections); }
        }

        private async Task AnswerAsync(TcpClient socket)
        {
            using (socket)
            using (var stream = socket.GetStream())
            using (var reader = new StreamReader(stream))
            using (var writer = new StreamWriter(stream) { AutoFlush = true })
            {
                try
                {
                    var request = JsonNode.Parse(await reader.ReadLineAsync(_stop.Token))!.AsObject();
                    Requests.Enqueue(request);
                    var response = CohortReply();
                    string op = request["op"]!.GetValue<string>();
                    async Task ReplyAsync(JsonObject reply)
                    {
                        reply["request_id"] = request["request_id"]!.DeepClone();
                        reply["worker_identity"] ??= new JsonObject { ["worker_id"] = _workerId, ["incarnation_id"] = IncarnationId };
                        await writer.WriteLineAsync(reply.ToJsonString().AsMemory(), _stop.Token);
                    }
                    switch (op)
                    {
                        case "hello":
                            response["capabilities"] = new JsonArray("committed_loadout_selection_v1", "active_loadout_refresh_v1",
                                "match_profiles_v1", "independent_match_cohorts_v1", "worker_registry_v1");
                            response["match_profiles"] = new JsonArray(Profiles.Select(p => p.DeepClone()).ToArray());
                            if (PendingReward is JsonNode reward) response["rewards"] = new JsonArray(reward.DeepClone());
                            break;
                        case "progression_committed":
                            PendingReward = null;
                            break;
                        case "select_match_profile":
                            SelectedProfile = request["profile_id"]!.GetValue<string>();
                            var selected = Profiles.Single(p => p["profile_id"]!.GetValue<string>() == SelectedProfile).DeepClone().AsObject();
                            selected.Remove("maximum_players");
                            selected.Remove("team_capacity");
                            selected["formation_id"] = request["formation_id"]!.DeepClone();
                            response["selected_match"] = selected;
                            break;
                        case "reserve":
                            Reservations.Enqueue(request);
                            break;
                    }
                    await ReplyAsync(response);
                    if (op == "subscribe_world")
                        await StreamFramesAsync(ReplyAsync);
                }
                catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            }
        }

        private volatile JsonNode PendingReward;
        private async Task StreamFramesAsync(Func<JsonObject, Task> reply)
        {
            using var timer = new PeriodicTimer(TimeSpan.FromMilliseconds(500));
            var pulse = timer.WaitForNextTickAsync(_stop.Token).AsTask();
            var ready = _frames.Reader.WaitToReadAsync(_stop.Token).AsTask();
            try
            {
                while (true)
                {
                    if (await Task.WhenAny(ready, pulse) == ready)
                    {
                        if (!await ready) return;
                        while (_frames.Reader.TryRead(out var frame))
                        {
                            if (frame["rewards"]?[0] is JsonNode reward) PendingReward = reward.DeepClone();
                            await reply(frame);
                        }
                        ready = _frames.Reader.WaitToReadAsync(_stop.Token).AsTask();
                    }
                    else
                    {
                        if (!await pulse) return;
                        if (!PauseHeartbeat) await reply(CohortReply());
                        pulse = timer.WaitForNextTickAsync(_stop.Token).AsTask();
                    }
                }
            }
            catch (IOException) { } // The host may close this fixture subscription on shutdown or a fenced incarnation.
        }

        public async ValueTask DisposeAsync()
        {
            _stop.Cancel();
            try { await _loop.WaitAsync(AdmissionTimeout); }
            finally { _listener.Stop(); _stop.Dispose(); }
        }
    }
}
