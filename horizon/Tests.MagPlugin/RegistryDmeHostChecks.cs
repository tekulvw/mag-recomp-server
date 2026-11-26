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
    public static void Run()
    {
        LeadershipProgressionChecks.RegistryHostIsolation();
        LeadershipProgressionChecks.RegistryBotFillStart();
        LeadershipProgressionChecks.RegistryLargeWorldStart();
    }
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

    /// <summary>A worker advertising bot fill forms from one queued human; an
    /// otherwise identical worker without it still waits for both sides.</summary>
    public static void RegistryBotFillStart() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        foreach (var account in new[] { "solo-alpha", "solo-bravo" })
        {
            var create = new MagCreateCharacterRequest { AccountName = account, Faction = 0, UniverseId = 26, NpId = new byte[36] };
            Check(store.CreateForAccount(create, account).Result == MagNetResult.Success, "Synthetic solo character creation failed");
        }
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            await using var botWorker = new RegistryWorker("bots") { SupportsBotFill = true };
            await using var plainWorker = new RegistryWorker("plain");
            await using var bots = new LocalDmeHost(botWorker.Client, store, SoloOptions("bots"));
            await using var plain = new LocalDmeHost(plainWorker.Client, store, SoloOptions("plain"));
            await Task.WhenAll(bots.StartInRegistryAsync(manager), plain.StartInRegistryAsync(manager));

            var botPlacement = bots.GetPlacement();
            var plainPlacement = plain.GetPlacement();
            Check(botPlacement.Profiles.All(p => p.BotFillTotal == 8) && botPlacement.StartupPolicy.MinimumPlayers == 1 &&
                botPlacement.MinimumPlayersPerTeam.SequenceEqual(new[] { 0, 0 }),
                "A bot-filled worker did not keep the operator's one-human startup policy");
            Check(plainPlacement.Profiles.All(p => !p.BotFill) && plainPlacement.StartupPolicy.MinimumPlayers == 2 &&
                plainPlacement.MinimumPlayersPerTeam.SequenceEqual(new[] { 1, 1 }),
                "A worker without bot fill lost its two-human, one-per-side floor");

            // Pure placement sends a lone human to the worker that fills with bots.
            MagWorldPlacementState State(string id, LocalDmeWorkerPlacement s) => new(id, s.Ready, s.Draining,
                s.StartupPolicy.MinimumPlayers, s.StartupPolicy.MaximumPlayers, s.MinimumPlayersPerTeam.ToArray(), s.Profiles,
                s.CurrentProfile, s.LiveTeamCounts, s.PendingTeamCounts, s.LiveCount, s.PendingTeamCounts.Sum(), s.CanForm);
            var lone = new[] { new MagQueuedHuman("solo", 2, 0, 0) };
            var plan = MagWorldPlacement.PlanNext(lone, new[] { State("plain", plainPlacement), State("bots", botPlacement) }, n => 0);
            Check(plan != null && plan.WorkerId == "bots" && plan.NewFormation && plan.Players.Count == 1,
                "One human did not form a new match on the bot-filled worker");
            Check(MagWorldPlacement.PlanNext(lone, new[] { State("plain", plainPlacement) }, n => 0) == null,
                "One human formed a match on a worker without bot fill");

            MagMlsSession Session(string account)
            {
                var settings = Server.Medius.Program.Settings;
                int previous = settings.MagApplicationId;
                settings.MagApplicationId = 20231;
                try { return MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id); }
                finally { settings.MagApplicationId = previous; }
            }
            int transaction = 0;
            Task<MagJoinGameResponse> Queue(LocalDmeHost host, MagMlsSession session)
            {
                var reply = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                host.QueueJoin(session, new MagRetailJoinGameRequest { TransactionId = ++transaction, Preference = 1, Value = 2 },
                    m => { if (m is MagJoinGameResponse response) reply.TrySetResult(response); }, () => true);
                return reply.Task;
            }

            Check(bots.TryStageRegistryFormation("profile-0-1"), "Bot-filled worker declined a one-human formation");
            var solo = Queue(bots, Session("solo-alpha"));
            await bots.TickWorldAsync();
            var admitted = await solo.WaitAsync(AdmissionTimeout);
            var configure = botWorker.Requests.Single(r => r["op"]!.GetValue<string>() == "configure_match");
            Check(admitted.Result == 0 && botWorker.SelectedProfile == "profile-0-1" && botWorker.Reservations.Count == 1 &&
                configure["minimum_players"]!.GetValue<int>() == 1 &&
                configure["minimum_players_per_team"]!.AsArray().Select(n => n!.GetValue<int>()).SequenceEqual(new[] { 0, 0 }),
                "A lone human was not admitted under a one-human bot-filled quorum");

            Check(plain.TryStageRegistryFormation("profile-0-1"), "Plain worker declined a staged formation");
            var waiting = Queue(plain, Session("solo-bravo"));
            await plain.TickWorldAsync();
            var refused = await waiting.WaitAsync(AdmissionTimeout);
            Check(refused.Result == 6 && plainWorker.Reservations.IsEmpty,
                "A worker without bot fill admitted one human to a new formation");
        }

        static LocalDmeHostOptions SoloOptions(string name) =>
            new(20231, 2, 1, 8, new[] { 0, 0 }, new[] { 0, 1 }, IPAddress.Parse("192.0.2.9"), 12073, name);
    });

    /// <summary>A 128-Character world is offered only by a host whose world capacity allows it,
    /// and configures and admits humans against its human caps, never its world size.</summary>
    public static void RegistryLargeWorldStart() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        var create = new MagCreateCharacterRequest { AccountName = "large-alpha", Faction = 0, UniverseId = 26, NpId = new byte[36] };
        Check(store.CreateForAccount(create, "large-alpha").Result == MagNetResult.Success, "Synthetic large-world character creation failed");
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            await using var largeWorker = new RegistryWorker("large") { SupportsBotFill = true, LargeWorld = true };
            await using var narrowWorker = new RegistryWorker("narrow") { SupportsBotFill = true, LargeWorld = true };
            await using var large = new LocalDmeHost(largeWorker.Client, store, LargeOptions("large", 128));
            await using var narrow = new LocalDmeHost(narrowWorker.Client, store, LargeOptions("narrow", 64));
            await large.StartInRegistryAsync(manager);
            bool refused = false;
            try { await narrow.StartInRegistryAsync(manager); } catch (IOException) { refused = true; }
            Check(refused, "A host limited to 64-Character worlds offered 128-Character profiles");

            var placement = large.GetPlacement();
            Check(placement.Profiles.Count == 2 && placement.Profiles.All(p => p.MaximumPlayers == 128 && p.TeamCapacity == 64 &&
                p.HumanCapacity == 32 && p.HumanTeamCapacity == 16 && p.BotFillTotal == 128) &&
                placement.StartupPolicy.MaximumPlayers == 64,
                "A 128-Character profile lost its human caps, or the host's human maximum followed world size");

            MagWorldPlacementState State(LocalDmeWorkerPlacement s) => new("large", s.Ready, s.Draining,
                s.StartupPolicy.MinimumPlayers, s.StartupPolicy.MaximumPlayers, s.MinimumPlayersPerTeam.ToArray(), s.Profiles,
                s.CurrentProfile, s.LiveTeamCounts, s.PendingTeamCounts, s.LiveCount, s.PendingTeamCounts.Sum(), s.CanForm);
            var crowd = Enumerable.Range(0, 40).Select(i => new MagQueuedHuman($"crowd-{i:00}", 2, 0, i)).ToArray();
            var plan = MagWorldPlacement.PlanNext(crowd, new[] { State(placement) }, n => 0);
            Check(plan != null && plan.NewFormation && plan.Players.Count == 16,
                "A new 128-Character formation took more than 16 humans on one side");

            var settings = Server.Medius.Program.Settings;
            int previous = settings.MagApplicationId;
            settings.MagApplicationId = 20231;
            MagMlsSession session;
            try { session = MakeSession(store, 20231, "large-alpha", store.ReadForAccount("large-alpha").Single().Id); }
            finally { settings.MagApplicationId = previous; }
            Check(large.TryStageRegistryFormation("profile-0-1"), "The large-world host declined a one-human formation");
            var reply = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
            large.QueueJoin(session, new MagRetailJoinGameRequest { TransactionId = 1, Preference = 1, Value = 2 },
                m => { if (m is MagJoinGameResponse response) reply.TrySetResult(response); }, () => true);
            await large.TickWorldAsync();
            var admitted = await reply.Task.WaitAsync(AdmissionTimeout);
            var configure = largeWorker.Requests.Single(r => r["op"]!.GetValue<string>() == "configure_match");
            Check(admitted.Result == 0 && configure["maximum_players"]!.GetValue<int>() == 32,
                "A 128-Character world was configured with its world size as the human maximum");
        }

        static LocalDmeHostOptions LargeOptions(string name, int worldCapacity) =>
            new(20231, 2, 1, 64, new[] { 0, 0 }, new[] { 0, 1 }, IPAddress.Parse("192.0.2.9"), 12073, name, worldCapacity);
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
        public bool SupportsParties;
        public bool SupportsLateParties;
        public bool SupportsBotFill;
        // Advertise control_plane_rebind_v1 and worker_drain_v1.
        public bool SupportsControlPlane;
        public bool SupportsDrain;
        public Func<JsonObject, Task> BeforePartyReply;
        public volatile bool DropNextPartyReply;
        // Reserve_party replies still to drop after recording them (int.MaxValue: never answer).
        public int DropPartyReplies;
        public Func<JsonObject, bool> RefuseParty;
        readonly ConcurrentDictionary<string, string[]> _partyReservations = new(StringComparer.Ordinal);
        public volatile string IncarnationId = "fixture-1";
        // Restart simulation (all off by default). Fence answers a request fenced for another
        // incarnation as the real worker does; Offline closes every connection unanswered, like a
        // stopped process; BreakStreams ends open world streams without changing the incarnation.
        public volatile bool Fence;
        public volatile bool Offline;
        public volatile bool WithholdPolledRewards;
        public volatile int ProfileTeamCapacity = 4;
        // Advertise world_capacity_v1 and 128-Character profiles (64 per side) with human caps 32/16.
        public bool LargeWorld;
        private CancellationTokenSource _streams = new();
        public void BreakStreams() => Interlocked.Exchange(ref _streams, new CancellationTokenSource()).Cancel();

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
                    if (Offline) return;
                    var line = await reader.ReadLineAsync(_stop.Token);
                    if (line == null || Offline) return;
                    var request = JsonNode.Parse(line)!.AsObject();
                    Requests.Enqueue(request);
                    var response = CohortReply();
                    string op = request["op"]!.GetValue<string>();
                    string incarnation = IncarnationId;
                    var streams = Volatile.Read(ref _streams);
                    async Task ReplyAsync(JsonObject reply)
                    {
                        reply["request_id"] = request["request_id"]!.DeepClone();
                        reply["worker_identity"] ??= new JsonObject { ["worker_id"] = _workerId, ["incarnation_id"] = incarnation };
                        await writer.WriteLineAsync(reply.ToJsonString().AsMemory(), _stop.Token);
                    }
                    if (Fence && (request["worker_identity"] is JsonObject fence
                        ? fence["incarnation_id"]?.GetValue<string>() != incarnation || fence["worker_id"]?.GetValue<string>() != _workerId
                        : op != "hello"))
                    {
                        response["ok"] = false;
                        response["error"] = request.ContainsKey("worker_identity") ? "worker_identity_mismatch" : "worker_identity_required";
                        await ReplyAsync(response);
                        return;
                    }
                    switch (op)
                    {
                        case "hello":
                            response["capabilities"] = new JsonArray("committed_loadout_selection_v1", "active_loadout_refresh_v1",
                                "match_profiles_v1", "independent_match_cohorts_v1", "worker_registry_v1");
                            response["match_profiles"] = new JsonArray(Profiles.Select(p => p.DeepClone()).ToArray());
                            foreach (var profile in response["match_profiles"]!.AsArray()) profile!["team_capacity"] = ProfileTeamCapacity;
                            if (LargeWorld)
                            {
                                response["capabilities"]!.AsArray().Add("world_capacity_v1");
                                foreach (var profile in response["match_profiles"]!.AsArray())
                                {
                                    profile!["maximum_players"] = 128; profile["team_capacity"] = 64;
                                    profile["human_capacity"] = 32; profile["human_team_capacity"] = 16;
                                }
                            }
                            if (SupportsControlPlane) response["capabilities"]!.AsArray().Add("control_plane_rebind_v1");
                            if (SupportsDrain) response["capabilities"]!.AsArray().Add("worker_drain_v1");
                            if (SupportsBotFill)
                            {
                                response["capabilities"]!.AsArray().Add("bot_fill_v1");
                                foreach (var profile in response["match_profiles"]!.AsArray()) profile["bot_fill_total"] = LargeWorld ? 128 : 8;
                            }
                            if (SupportsParties)
                            {
                                response["capabilities"]!.AsArray().Add("party_reservation_v1");
                                if (SupportsLateParties) response["capabilities"]!.AsArray().Add("party_late_entry_v1");
                                foreach (var profile in response["match_profiles"]!.AsArray())
                                {
                                    profile["squad_capacity"] = 2;
                                    profile["squad_counts"] = new JsonArray(2, 2);
                                }
                            }
                            if (PendingReward is JsonNode reward && !WithholdPolledRewards) response["rewards"] = new JsonArray(reward.DeepClone());
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
                            if (SupportsParties) selected["admission_epoch"] = 1;
                            response["selected_match"] = selected;
                            break;
                        case "reserve":
                            Reservations.Enqueue(request);
                            break;
                        case "reserve_party":
                        case "release_party":
                            if (BeforePartyReply != null) await BeforePartyReply(request);
                            if (op == "reserve_party" && RefuseParty?.Invoke(request) == true)
                            {
                                response["ok"] = false; response["error"] = "synthetic_party_refusal";
                                break;
                            }
                            var party = new JsonObject();
                            foreach (var field in new[] { "formation_id", "profile_id", "cohort_id", "world_id", "admission_epoch" })
                                party[field] = request[field]!.DeepClone();
                            party["status"] = op == "reserve_party" ? "reserved" : "released";
                            party["reservation_ids"] = new JsonArray();
                            if (op == "reserve_party")
                            {
                                var members = request["members"]!.AsArray();
                                var ids = members.Select(member => member["reservation_id"]!.GetValue<string>()).ToArray();
                                _partyReservations.TryAdd(request["cohort_id"]!.GetValue<string>(), ids);
                                foreach (var reservation in ids) party["reservation_ids"]!.AsArray().Add(reservation);
                                var faction = members[0]["character"]!["faction"]!.GetValue<byte>();
                                int team = faction == byte.Parse(SelectedProfile.Split('-')[1]) ? 0 : 1;
                                party["team"] = team;
                                var draws = Requests.Where(call => call["op"]!.GetValue<string>() == "reserve_party" &&
                                    call["formation_id"]!.GetValue<string>() == request["formation_id"]!.GetValue<string>() &&
                                    call["members"]![0]!["character"]!["faction"]!.GetValue<byte>() == faction)
                                    .Select(call => call["cohort_id"]!.GetValue<string>()).Distinct().ToList();
                                party["squad"] = 16 * team + draws.IndexOf(request["cohort_id"]!.GetValue<string>());
                                if (DropNextPartyReply) { DropNextPartyReply = false; return; }
                                if (Interlocked.Decrement(ref DropPartyReplies) >= 0) return;
                                Interlocked.Exchange(ref DropPartyReplies, 0);
                            }
                            else if (_partyReservations.TryGetValue(request["cohort_id"]!.GetValue<string>(), out var released))
                                foreach (var reservation in released) party["reservation_ids"]!.AsArray().Add(reservation);
                            response["party_reservation"] = party;
                            break;
                    }
                    await ReplyAsync(response);
                    if (op == "subscribe_world")
                        await StreamFramesAsync(ReplyAsync, streams.Token);
                }
                catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            }
        }

        private volatile JsonNode PendingReward;
        private async Task StreamFramesAsync(Func<JsonObject, Task> reply, CancellationToken streams)
        {
            using var linked = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token, streams);
            using var timer = new PeriodicTimer(TimeSpan.FromMilliseconds(500));
            var pulse = timer.WaitForNextTickAsync(linked.Token).AsTask();
            var ready = _frames.Reader.WaitToReadAsync(linked.Token).AsTask();
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
                        ready = _frames.Reader.WaitToReadAsync(linked.Token).AsTask();
                    }
                    else
                    {
                        if (!await pulse) return;
                        if (!PauseHeartbeat) await reply(CohortReply());
                        pulse = timer.WaitForNextTickAsync(linked.Token).AsTask();
                    }
                }
            }
            catch (IOException) { } // The host may close this fixture subscription on shutdown or a fenced incarnation.
            catch (OperationCanceledException) when (streams.IsCancellationRequested && !_stop.IsCancellationRequested) { } // BreakStreams.
        }

        public async ValueTask DisposeAsync()
        {
            _stop.Cancel();
            try { await _loop.WaitAsync(AdmissionTimeout); }
            finally { _listener.Stop(); _stop.Dispose(); }
        }
    }
}
