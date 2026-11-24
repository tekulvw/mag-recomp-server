using System.Collections;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

/// <summary>Restart adoption with invented loopback workers: a restarted worker's new
/// incarnation is adopted for its own entry only, a same-incarnation stream break is not a
/// restart, a changed contract is refused and the crash-loop cap holds.</summary>
internal static class RegistryAdoptionChecks
{
    public static void Run()
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try
        {
            LeadershipProgressionChecks.RegistryRestartAdoption();
            LeadershipProgressionChecks.RegistryAdoptionRefusal();
            LeadershipProgressionChecks.RegistryAdoptionCapAndSwitch();
            LeadershipProgressionChecks.RegistryLateWorkerStart();
            LeadershipProgressionChecks.RegistryAdoptionPortLeases();
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    }
}

internal static partial class LeadershipProgressionChecks
{
    static readonly TimeSpan AdoptionDeadline = TimeSpan.FromSeconds(30);

    static JsonObject AdoptionWorkerRow(string id, int port) => new()
    {
        ["worker_id"] = id, ["worker_host"] = "127.0.0.1", ["worker_port"] = port, ["minimum_players"] = 2,
        ["maximum_players"] = 2, ["minimum_players_per_team"] = new JsonArray(1, 1), ["draining"] = false
    };

    static MagWorldRegistry AdoptionRegistry(LocalCharacterStore store, JsonObject adoption, params JsonObject[] workers) =>
        AdoptionRegistry(store, adoption, null, workers);

    static MagWorldRegistry AdoptionRegistry(LocalCharacterStore store, JsonObject adoption, MagDmePortBudget ports, params JsonObject[] workers)
    {
        var config = new JsonObject
        {
            ["format"] = "mag_world_registry_v1", ["application_id"] = 20231, ["client_address"] = "192.0.2.9",
            ["client_tcp_port"] = 12073, ["workers"] = new JsonArray(workers)
        };
        if (adoption != null) config["worker_restart_adoption"] = adoption;
        return new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store, portBudget: ports);
    }

    static LocalCharacterStore AdoptionStore(string directory, string name, params (string Account, byte Faction)[] accounts)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, name + ".json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        foreach (var (account, faction) in accounts)
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] },
                account).Result == MagNetResult.Success, "Synthetic adoption character creation failed");
        return store;
    }

    static async Task TickUntil(MagWorldRegistry registry, Func<bool> condition, string failure)
    {
        using var deadline = new CancellationTokenSource(AdoptionDeadline);
        try { while (!condition()) { await registry.TickAsync(); await Task.Delay(20, deadline.Token); } }
        catch (OperationCanceledException) { throw new Exception(failure); }
    }

    static async Task TickFor(MagWorldRegistry registry, TimeSpan duration)
    {
        var until = DateTime.UtcNow + duration;
        while (DateTime.UtcNow < until) { await registry.TickAsync(); await Task.Delay(20); }
    }

    static LocalDmeHost RegistryHost(MagWorldRegistry registry, string workerId)
    {
        foreach (var slot in ((IEnumerable)typeof(MagWorldRegistry).GetField("_slots", HostInternals)!.GetValue(registry)!).Cast<object>())
            if (((MagConfiguredWorker)slot.GetType().GetField("Config")!.GetValue(slot)!).WorkerId == workerId)
                return (LocalDmeHost)slot.GetType().GetField("Host")!.GetValue(slot)!;
        throw new InvalidOperationException("Unknown synthetic worker.");
    }

    static MagRegisteredWorld World(MagWorldRegistry registry, string workerId) => registry.Worlds.Single(w => w.WorkerId == workerId);
    static MagWorkerAdoptionCounters Counters(MagWorldRegistry registry, string workerId) => registry.AdoptionCounters.Single(c => c.WorkerId == workerId);

    /// <summary>Stop a synthetic worker, wait until its host noticed, then start it again as a new incarnation.</summary>
    static async Task RestartWorker(MagWorldRegistry registry, RegistryWorker worker, string workerId, string incarnation, int teamCapacity = 4)
    {
        worker.Offline = true;
        worker.BreakStreams();
        await TickUntil(registry, () => !World(registry, workerId).Ready, "A stopped worker still looked ready");
        worker.ProfileTeamCapacity = teamCapacity;
        worker.IncarnationId = incarnation;
        worker.Offline = false;
    }

    static Task<MagJoinGameResponse> QueueRegistryJoin(MagWorldRegistry registry, LocalCharacterStore store, string account, int transaction, uint mode = 2)
    {
        var session = MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id);
        var reply = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
        registry.QueueJoin(session, new MagRetailJoinGameRequest { TransactionId = transaction, Preference = 1, Value = mode },
            message => { if (message is MagJoinGameResponse response) reply.TrySetResult(response); }, () => true);
        return reply.Task;
    }

    public static void RegistryRestartAdoption() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "restart", ("alpha", 0), ("bravo", 1), ("charlie", 0), ("delta", 1));
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            await using var first = new RegistryWorker("first") { Fence = true, WithholdPolledRewards = true };
            await using var second = new RegistryWorker("second") { Fence = true, WithholdPolledRewards = true };
            await using var registry = AdoptionRegistry(store, null, AdoptionWorkerRow("first", first.Port), AdoptionWorkerRow("second", second.Port));
            Check(registry.AdoptionPolicy == MagWorkerAdoptionPolicy.Default && registry.AdoptionPolicy.Enabled,
                "Restart adoption was not on by default");
            await registry.InitializeAsync();
            int transaction = 0;
            async Task<MagJoinGameResponse[]> Admit(params string[] accounts)
            {
                var queued = accounts.Select(account => QueueRegistryJoin(registry, store, account, ++transaction)).ToArray();
                await TickUntil(registry, () => queued.All(t => t.IsCompleted), "Synthetic registry admission did not finish");
                return await Task.WhenAll(queued);
            }

            var firstPair = await Admit("alpha", "bravo");
            var secondPair = await Admit("charlie", "delta");
            Check(firstPair.Concat(secondPair).All(reply => reply.Result == 0), "Synthetic admission failed before the restart");
            string victimId = registry.Worlds.Single(w => w.WorldId == firstPair[0].WorldId).WorkerId;
            string survivorId = victimId == "first" ? "second" : "first";
            var victim = victimId == "first" ? first : second;
            var survivor = victimId == "first" ? second : first;
            Check(firstPair.All(r => r.WorldId == firstPair[0].WorldId) && secondPair.All(r => r.WorldId == World(registry, survivorId).WorldId),
                "Each synthetic pair did not fill its own worker");
            var before = World(registry, victimId);
            var survivorBefore = World(registry, survivorId);

            // Alpha is connected; a reward reaches Horizon in a frame but is never confirmed.
            uint alphaCharacter = store.ReadForAccount("alpha").Single().Id;
            var alphaReservation = victim.Reservations.Single(r => r["account_id"]!.GetValue<uint>() == alphaCharacter);
            var alphaClient = ConnectFixture(RegistryHost(registry, victimId), alphaReservation);
            var reward = RewardReply("enemy_killed", "");
            var row = reward["rewards"]![0]!;
            foreach (string key in new[] { "reservation_id", "world_id", "player_id" }) row[key] = alphaReservation[key]!.DeepClone();
            row["reward_id"] = "synthetic-unconfirmed-reward";
            row["character_id"] = alphaCharacter;
            var victimHost = RegistryHost(registry, victimId);
            var ledger = typeof(LocalDmeHost).GetField("_receipts", HostInternals)!.GetValue(victimHost)!;
            await victim.PublishAsync(reward);
            await TickUntil(registry, () => (int)ledger.GetType().GetProperty("Count")!.GetValue(ledger)! == 1,
                "Horizon did not record the synthetic reward as unconfirmed");

            await RestartWorker(registry, victim, victimId, "fixture-restarted");
            await TickUntil(registry, () => World(registry, victimId) is { Ready: true, IncarnationId: "fixture-restarted" },
                "The restarted worker's new incarnation was not adopted");
            var adopted = World(registry, victimId);
            var counters = Counters(registry, victimId);
            Check(adopted.WorldId != before.WorldId && adopted.ReservedPlayers == 0 && counters.Adoptions == 1 && counters.WriteOffs == 1 &&
                counters.LostRewards == 1 && counters.ReleasedPlayers + counters.DroppedReleases >= 2,
                "Adoption did not reopen a fresh world after writing off the old incarnation and its unconfirmed reward");
            Check(alphaClient.IsDestroyed && (RegistryHost(registry, victimId).GetAccountOwnership("alpha") |
                RegistryHost(registry, survivorId).GetAccountOwnership("alpha")) == LocalDmeAccountOwnership.None,
                "The old incarnation's connected player kept its transport or ownership");
            var survivorAfter = World(registry, survivorId);
            Check(survivorAfter == survivorBefore && survivor.Requests.All(r => r["op"]!.GetValue<string>() is not ("release" or "release_party")) &&
                Counters(registry, survivorId) == new MagWorkerAdoptionCounters(survivorId, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                "Adopting one entry disturbed the other worker's match");

            // The released players form a new match in the adopted world.
            var again = await Admit("alpha", "bravo");
            Check(again.All(reply => reply.Result == 0 && reply.WorldId == adopted.WorldId) && World(registry, victimId).ReservedPlayers == 2,
                "A new match did not form in the adopted world");

            // A same-incarnation stream break is not a restart: no write-off and no adoption.
            int subscriptions = survivor.Requests.Count(r => r["op"]!.GetValue<string>() == "subscribe_world");
            survivor.BreakStreams();
            await TickUntil(registry, () => survivor.Requests.Count(r => r["op"]!.GetValue<string>() == "subscribe_world") > subscriptions &&
                World(registry, survivorId).Ready, "A same-incarnation stream break did not resubscribe");
            await TickFor(registry, TimeSpan.FromSeconds(2));
            var resumed = World(registry, survivorId);
            Check(resumed.Ready && resumed.WorldId == survivorBefore.WorldId && resumed.IncarnationId == survivorBefore.IncarnationId &&
                Counters(registry, survivorId).WriteOffs == 0 && Counters(registry, survivorId).Adoptions == 0 &&
                Counters(registry, victimId).Adoptions == 1, "A same-incarnation stream break was treated as a restart");
        }
    });

    public static void RegistryAdoptionRefusal() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "refusal", ("alpha", 0), ("bravo", 1));
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            await using var worker = new RegistryWorker("first") { Fence = true };
            await using var registry = AdoptionRegistry(store, null, AdoptionWorkerRow("first", worker.Port));
            await registry.InitializeAsync();
            string original = World(registry, "first").IncarnationId;
            await RestartWorker(registry, worker, "first", "fixture-changed", teamCapacity: 5);
            await TickUntil(registry, () => Counters(registry, "first").Refusals == 1, "A changed profile matrix was not refused");
            await TickFor(registry, TimeSpan.FromSeconds(3));
            var refused = Counters(registry, "first");
            Check(refused.Refusals == 1 && refused.WriteOffs == 1 && refused.Adoptions == 0 && !World(registry, "first").Ready &&
                World(registry, "first").IncarnationId == original, "A mismatched incarnation was adopted or refused repeatedly");
            var join = QueueRegistryJoin(registry, store, "alpha", 1);
            await TickFor(registry, TimeSpan.FromMilliseconds(500));
            Check(!join.IsCompleted, "A refused entry admitted a player");

            // A later incarnation with the established contract is adopted.
            await RestartWorker(registry, worker, "first", "fixture-matching");
            await TickUntil(registry, () => World(registry, "first") is { Ready: true, IncarnationId: "fixture-matching" },
                "A matching incarnation after a refusal was not adopted");
            var late = QueueRegistryJoin(registry, store, "bravo", 2);
            await TickUntil(registry, () => join.IsCompleted && late.IsCompleted, "Queued players did not form a match after adoption");
            Check(join.Result.Result == 0 && late.Result.Result == 0 && Counters(registry, "first").Adoptions == 1,
                "Queued players were not admitted into the adopted world");
        }
    });

    /// <summary>With every DME port leased by two live matches, restarting one worker mid-match
    /// returns its players' port leases exactly once, and the waiting players use those ports
    /// for the next match in the adopted world, whose world ID has changed.</summary>
    public static void RegistryAdoptionPortLeases() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "ports", ("alpha", 0), ("bravo", 1), ("charlie", 0), ("delta", 1), ("echo", 0), ("foxtrot", 1));
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var ports = new MagDmePortBudget(MagDmePortBudget.FirstPort + 3, null);
            await using var first = new RegistryWorker("first") { Fence = true };
            await using var second = new RegistryWorker("second") { Fence = true };
            await using var registry = AdoptionRegistry(store, null, ports, AdoptionWorkerRow("first", first.Port), AdoptionWorkerRow("second", second.Port));
            await registry.InitializeAsync();
            int transaction = 0;
            Task<MagJoinGameResponse>[] Queue(params string[] accounts) =>
                accounts.Select(account => QueueRegistryJoin(registry, store, account, ++transaction)).ToArray();
            async Task<MagJoinGameResponse[]> Admit(Task<MagJoinGameResponse>[] queued)
            {
                await TickUntil(registry, () => queued.All(t => t.IsCompleted), "Synthetic admission did not finish");
                return await Task.WhenAll(queued);
            }
            var victimPair = await Admit(Queue("alpha", "bravo"));
            var survivorPair = await Admit(Queue("charlie", "delta"));
            Check(victimPair.Concat(survivorPair).All(r => r.Result == 0) && ports.InUse == 4 && ports.Available == 0,
                "Two matches did not lease every DME port");
            string victimId = registry.Worlds.Single(w => w.WorldId == victimPair[0].WorldId).WorkerId;
            string survivorId = victimId == "first" ? "second" : "first";
            var victim = victimId == "first" ? first : second;
            var oldWorld = World(registry, victimId).WorldId;
            var survivorBefore = World(registry, survivorId);
            var alphaReservation = victim.Reservations.Single(r => r["account_id"]!.GetValue<uint>() == store.ReadForAccount("alpha").Single().Id);
            var alphaClient = ConnectFixture(RegistryHost(registry, victimId), alphaReservation);

            // Two more players wait only for ports.
            var waiting = Queue("echo", "foxtrot");
            await TickFor(registry, TimeSpan.FromMilliseconds(500));
            Check(waiting.All(t => !t.IsCompleted) && ports.InUse == 4, "Players were admitted past the DME port capacity");

            await RestartWorker(registry, victim, victimId, "fixture-restarted");
            var admitted = await Admit(waiting);
            var adopted = World(registry, victimId);
            Check(adopted.IncarnationId == "fixture-restarted" && adopted.WorldId != oldWorld && alphaClient.IsDestroyed &&
                admitted.All(r => r.Result == 0 && r.WorldId == adopted.WorldId),
                "The waiting players did not take the freed ports in the adopted world");
            Check(ports.InUse == 4 && World(registry, survivorId) == survivorBefore &&
                Counters(registry, victimId) is { Adoptions: 1, WriteOffs: 1 } c && c.ReleasedPlayers + c.DroppedReleases >= 2,
                "Port leases were not returned exactly once by the restart, or the other match changed");

            // The released players now wait for ports rather than over-admitting.
            var requeued = Queue("alpha", "bravo");
            await TickFor(registry, TimeSpan.FromMilliseconds(500));
            Check(requeued.All(t => !t.IsCompleted) && ports.InUse == 4, "Released players were admitted past the DME port capacity");
        }
    });

    /// <summary>A worker unreachable when Horizon started is started like any other once it answers.</summary>
    public static void RegistryLateWorkerStart() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "late", ("alpha", 0), ("bravo", 1));
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            await using var ready = new RegistryWorker("first") { Fence = true };
            await using var late = new RegistryWorker("second") { Fence = true, Offline = true };
            await using var registry = AdoptionRegistry(store, null, AdoptionWorkerRow("first", ready.Port), AdoptionWorkerRow("second", late.Port));
            await registry.InitializeAsync();
            var before = World(registry, "first");
            Check(before.Ready && !World(registry, "second").Ready && World(registry, "second").IncarnationId == null,
                "An unreachable worker looked ready at startup");
            late.Offline = false;
            await TickUntil(registry, () => World(registry, "second") is { Ready: true, IncarnationId: "fixture-1" },
                "A worker that came up after startup was not started");
            Check(World(registry, "first") == before && Counters(registry, "second") is { Adoptions: 1, WriteOffs: 0 },
                "A late worker start disturbed the ready worker or reported a write-off");
        }
    });

    public static void RegistryAdoptionCapAndSwitch() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "cap");
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            await using (var worker = new RegistryWorker("first") { Fence = true })
            await using (var registry = AdoptionRegistry(store, new JsonObject { ["max_adoptions"] = 1, ["window_seconds"] = 600 },
                AdoptionWorkerRow("first", worker.Port)))
            {
                await registry.InitializeAsync();
                Check(registry.AdoptionPolicy == new MagWorkerAdoptionPolicy(true, 1, TimeSpan.FromSeconds(600)), "Adoption limit was not read");
                await RestartWorker(registry, worker, "first", "fixture-loop-1");
                await TickUntil(registry, () => World(registry, "first") is { Ready: true, IncarnationId: "fixture-loop-1" },
                    "The first restart within the cap was not adopted");
                await RestartWorker(registry, worker, "first", "fixture-loop-2");
                await TickUntil(registry, () => Counters(registry, "first").CapHits == 1, "The adoption cap was not reported");
                await TickFor(registry, TimeSpan.FromSeconds(2));
                var capped = Counters(registry, "first");
                Check(capped.Adoptions == 1 && capped.WriteOffs == 2 && capped.CapHits == 1 && !World(registry, "first").Ready &&
                    World(registry, "first").IncarnationId == "fixture-loop-1", "A restart beyond the cap was adopted");
            }

            await using (var worker = new RegistryWorker("first") { Fence = true })
            await using (var registry = AdoptionRegistry(store, new JsonObject { ["enabled"] = false }, AdoptionWorkerRow("first", worker.Port)))
            {
                await registry.InitializeAsync();
                await RestartWorker(registry, worker, "first", "fixture-disabled");
                await TickFor(registry, TimeSpan.FromSeconds(3));
                var world = World(registry, "first");
                Check(!world.Ready && world.IncarnationId == "fixture-1" &&
                    Counters(registry, "first") == new MagWorkerAdoptionCounters("first", 0, 0, 0, 0, 0, 0, 0, 0, 0),
                    "Disabled restart adoption still wrote off or adopted a worker");
            }
        }
    });
}
