using System.Collections;
using System.Collections.Concurrent;
using System.Text;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// Synthetic DME port capacity checks: invented accounts, loopback worker
// fixtures and in-memory transports. No UDP port is bound.
internal static class DmePortCapacityChecks
{
    public static void Budget()
    {
        static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }
        static Func<string, string> Env(string end, string capacity) => name =>
            name == MagDmePortBudget.PortEndVariable ? end : name == MagDmePortBudget.CapacityVariable ? capacity : null;
        static void Refused(string end, string capacity)
        {
            try { MagDmePortBudget.Load(Env(end, capacity)); }
            catch (ArgumentException) { return; }
            throw new Exception($"Invalid DME port configuration accepted: end={end} capacity={capacity}");
        }

        var open = MagDmePortBudget.Load(Env(null, null));
        Check(!open.Bounded && open.Capacity == null && open.LastPort == null && open.Available == int.MaxValue && open.Source == "unbounded",
            "Unset port configuration did not keep the open-ended DME allocation");
        Check(open.TryAcquire(64)?.Length == 64 && open.InUse == 64, "Unbounded budget refused leases");

        var range = MagDmePortBudget.Load(Env("10505", null));
        Check(range.Capacity == 32 && range.LastPort == 10505 && range.Source == "port_range",
            "Forwarded range 10474-10505 did not become a 32-human cap");
        var single = MagDmePortBudget.Load(Env("10474", null));
        Check(single.Capacity == 1 && single.LastPort == 10474, "One forwarded port did not become a one-human cap");
        var lowered = MagDmePortBudget.Load(Env("10505", "20"));
        Check(lowered.Capacity == 20 && lowered.LastPort == 10505 && lowered.Source == "override",
            "Explicit capacity did not override the range size");
        var onlyCapacity = MagDmePortBudget.Load(Env(null, "8"));
        Check(onlyCapacity.Capacity == 8 && onlyCapacity.LastPort == 10481, "A capacity alone did not bound the DME ports");
        foreach (var (end, capacity) in new[] { ("10473", null), ("65536", null), ("abc", null), ("-1", null), (" 10505", null),
                     ("10505", "0"), ("10505", "33"), (null, "-3"), ("10474", "2") })
            Refused(end, capacity);

        var budget = new MagDmePortBudget(10477, null);
        var three = budget.TryAcquire(3);
        Check(three?.Length == 3 && budget.InUse == 3 && budget.Available == 1, "Group lease was not counted");
        Check(budget.TryAcquire(2) == null && budget.InUse == 3, "A group that does not fit took part of the remaining ports");
        var last = budget.TryAcquire();
        Check(last != null && budget.Available == 0 && budget.TryAcquire() == null, "Full budget handed out another port");
        three[0].Dispose(); three[0].Dispose();
        Check(budget.InUse == 3 && !three[0].Held && three[1].Held, "A lease returned twice or not at all");
        Check(budget.Describe().Contains("in_use=3 capacity=4 ports=10474-10477"), "Port status line lost its fields");
    }

    public static void Placement()
    {
        static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }
        var profile = new MagMatchProfile("a-b", 1, 0, 1, 8, 4, SquadCapacity: 4, SquadCounts: new[] { 2, 2 });
        var profiles = new[] { profile, new MagMatchProfile("a-c", 1, 0, 2, 8, 4, 4, new[] { 2, 2 }),
            new MagMatchProfile("b-c", 1, 1, 2, 8, 4, 4, new[] { 2, 2 }) };
        MagWorldPlacementState Running(int live) => new("live", true, false, 2, 8, new[] { 1, 1 }, profiles, profile,
            new[] { 0, live, 0 }, new[] { 0, 0, 0 }, live, 0, false, AcceptsLateParties: true);
        MagWorldPlacementState Idle() => new("idle", true, false, 2, 8, new[] { 1, 1 }, profiles, null,
            new[] { 0, 0, 0 }, new[] { 0, 0, 0 }, 0, 0, true);
        var solos = new[] { new MagQueuedHuman("s0", 1, 1, 0), new MagQueuedHuman("s1", 1, 1, 1), new MagQueuedHuman("s2", 1, 0, 2) };

        Check(MagWorldPlacement.PlanNext(solos, new[] { Running(1) }, n => 0)?.Players.Count == 3,
            "Unbounded placement changed");
        Check(MagWorldPlacement.PlanNext(solos, new[] { Running(1), Idle() }, n => 0, humanRoom: 0) == null,
            "Placement admitted a human with no free DME port");
        var late = MagWorldPlacement.PlanNext(solos, new[] { Running(1) }, n => 0, humanRoom: 1);
        Check(late != null && late.Players.Select(p => p.TicketId).SequenceEqual(new[] { "s0" }),
            "Late join exceeded the free DME ports or skipped the oldest human");

        var opposed = new[] { new MagQueuedHuman("a", 1, 0, 0), new MagQueuedHuman("b", 1, 1, 1),
            new MagQueuedHuman("c", 1, 0, 2), new MagQueuedHuman("d", 1, 1, 3) };
        Check(MagWorldPlacement.PlanNext(opposed, new[] { Idle() }, n => 0, humanRoom: 1) == null,
            "A new match formed below its minimum to fit the free ports");
        var formed = MagWorldPlacement.PlanNext(opposed, new[] { Idle() }, n => 0, humanRoom: 2);
        Check(formed != null && formed.NewFormation && formed.Players.Select(p => p.TicketId).SequenceEqual(new[] { "a", "b" }),
            "New formation exceeded the free DME ports");

        // A party of three behind one older solo and ahead of a younger one.
        var queue = new[] { new MagQueuedHuman("older", 1, 0, 0), new MagQueuedHuman("party", 1, 0, 1, 3),
            new MagQueuedHuman("younger", 1, 0, 2) };
        var held = MagWorldPlacement.PlanNext(queue, new[] { Running(1) }, n => 0, humanRoom: 2);
        Check(held != null && held.Players.Select(p => p.TicketId).SequenceEqual(new[] { "older" }),
            "Port-blocked party let a younger human take its ports, or held an older one");
        Check(MagWorldPlacement.PlanNext(queue.Skip(1).ToArray(), new[] { Running(2) }, n => 0, humanRoom: 2) == null,
            "Younger solo took a port while an older party waited for room");
        var party = MagWorldPlacement.PlanNext(queue.Skip(1).ToArray(), new[] { Running(2) }, n => 0, humanRoom: 3);
        Check(party != null && party.Players.Single().TicketId == "party", "Party did not enter once its ports were free");
        Check(MagWorldPlacement.PlanNext(queue.Skip(1).ToArray(), new[] { Running(2) }, n => 0)?.Players.Single().TicketId == "party",
            "Unbounded late party placement changed");
    }

    public static void Admission() => LeadershipProgressionChecks.DmePortHostAdmission();
    public static void Registry() => LeadershipProgressionChecks.DmePortRegistry();
    public static void Parties() => LeadershipProgressionChecks.DmePortRegistryParties();
}

internal static partial class LeadershipProgressionChecks
{
    /// <summary>A standalone world refuses a join past the cap with its full-world answer and frees the port on release.</summary>
    public static void DmePortHostAdmission() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        Create(store, "bravo", "Bravo");
        RunAsync().GetAwaiter().GetResult();
        async Task RunAsync()
        {
            var settings = Server.Medius.Program.Settings;
            int previousApp = settings.MagApplicationId;
            settings.MagApplicationId = 20231;
            try
            {
                var budget = new MagDmePortBudget(10474, null);
                await using var bridge = new FixtureBridge(Array.Empty<string>());
                await using var host = new LocalDmeHost(bridge.Client, store, budget);
                await MakeReady(host, bridge, 20231);
                var alpha = MakeSession(store, 20231, "alpha", store.ReadForAccount("alpha").Single().Id);
                var bravo = MakeSession(store, 20231, "bravo", store.ReadForAccount("bravo").Single().Id);
                Check((await BeginJoin(host, alpha).WaitAsync(AdmissionTimeout)).Result == 0 && budget.InUse == 1,
                    "First human did not take the only DME port");
                var first = bridge.Latest("reserve");
                int clients = host.World.Clients.Count;
                Check((await BeginJoin(host, bravo).WaitAsync(AdmissionTimeout)).Result == 6,
                    "Join past the forwarded ports was not refused as a full world");
                Check(bridge.Requests.Count(r => r["op"]!.GetValue<string>() == "reserve") == 1 && host.World.Clients.Count == clients &&
                    budget.InUse == 1, "Refused join still allocated a DME client, reservation or port");
                await Release(host, first).WaitAsync(AdmissionTimeout);
                Check(budget.InUse == 0, "Released transport kept its DME port");
                Check((await BeginJoin(host, bravo).WaitAsync(AdmissionTimeout)).Result == 0 && budget.InUse == 1,
                    "Freed DME port was not reused");
                Check((await BeginJoin(host, alpha).WaitAsync(AdmissionTimeout)).Result == 6 &&
                    bridge.Requests.Count(r => r["op"]!.GetValue<string>() == "reserve") == 2 && budget.InUse == 1,
                    "Reconnect bypassed the DME port cap");
            }
            finally { settings.MagApplicationId = previousApp; }
        }
    });

    /// <summary>Registry placement keeps players queued while every port is in use,
    /// across worlds, and admits them as transports release.</summary>
    public static void DmePortRegistry() => WithDirectory(directory =>
    {
        int app = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try { RunAsync().GetAwaiter().GetResult(); }
        finally { Server.Medius.Program.Settings.MagApplicationId = app; }
        async Task RunAsync()
        {
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1), ("charlie", (byte)0), ("delta", (byte)1) })
                Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] },
                    account).Result == MagNetResult.Success, "Synthetic port-cap character creation failed");
            await using var a = new RegistryWorker("first");
            await using var b = new RegistryWorker("second");
            JsonObject Row(string id, int port) => new() { ["worker_id"] = id, ["worker_host"] = "127.0.0.1", ["worker_port"] = port,
                ["minimum_players"] = 2, ["maximum_players"] = 4, ["minimum_players_per_team"] = new JsonArray(1, 1), ["draining"] = false };
            var config = new JsonObject { ["format"] = "mag_world_registry_v1", ["application_id"] = 20231, ["client_address"] = "192.0.2.9",
                ["client_tcp_port"] = 12073, ["workers"] = new JsonArray(Row("first", a.Port), Row("second", b.Port)) };
            var budget = new MagDmePortBudget(10476, null);
            await using var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store, portBudget: budget);
            await registry.InitializeAsync();
            Check(ReferenceEquals(registry.PortBudget, budget), "Registry did not share its DME port budget");
            var hosts = ((IEnumerable)typeof(MagWorldRegistry).GetField("_slots", HostInternals)!.GetValue(registry)!).Cast<object>()
                .Select(s => (LocalDmeHost)s.GetType().GetField("Host")!.GetValue(s)!).ToArray();
            Check(hosts.All(host => ReferenceEquals(host.PortBudget, budget)), "Worlds did not share one DME port budget");
            MagMlsSession Session(string account) => MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id);
            int transaction = 0;
            Task<MagJoinGameResponse> Queue(MagMlsSession session)
            {
                var completion = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                registry.QueueJoin(session, new MagRetailJoinGameRequest { TransactionId = ++transaction, Preference = 1, Value = 2 },
                    m => { if (m is MagJoinGameResponse r) completion.TrySetResult(r); }, () => true);
                return completion.Task;
            }
            async Task Until(Func<bool> condition)
            {
                using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(10));
                while (!condition()) { await registry.TickAsync(); await Task.Delay(10, deadline.Token); }
            }
            async Task Settle() { for (int i = 0; i < 20; i++) { await registry.TickAsync(); await Task.Delay(10); } }
            JsonObject Reserved(string account) => a.Reservations.Concat(b.Reservations)
                .Last(r => r["account_id"]!.GetValue<uint>() == store.ReadForAccount(account).Single().Id);
            async Task Disconnect(string account)
            {
                var reservation = Reserved(account);
                var host = hosts.Single(h => (uint)h.World.WorldId == reservation["world_id"]!.GetValue<uint>());
                var client = ConnectFixture(host, reservation);
                await client.Stop();
                await Until(() => host.GetAccountOwnership(account) == LocalDmeAccountOwnership.None);
            }

            var alpha = Queue(Session("alpha")); var bravo = Queue(Session("bravo"));
            await Until(() => alpha.IsCompleted && bravo.IsCompleted);
            var charlie = Queue(Session("charlie"));
            await Until(() => charlie.IsCompleted);
            Check((await alpha).Result == 0 && (await bravo).Result == 0 && (await charlie).Result == 0 && budget.InUse == 3,
                "Humans within the forwarded ports were not admitted");

            // The fourth human fits a world (and the idle second worker) but not a port.
            var delta = Queue(Session("delta"));
            await Settle();
            Check(!delta.IsCompleted && budget.InUse == 3 && a.Reservations.Count + b.Reservations.Count == 3 &&
                hosts.All(host => host.GetAccountOwnership("delta") == LocalDmeAccountOwnership.None),
                "A human past the forwarded ports was reserved instead of kept waiting");

            // Releasing a transport returns its port to the next waiting human.
            await Disconnect("bravo");
            await Until(() => delta.IsCompleted);
            Check((await delta).Result == 0 && budget.InUse == 3, "Waiting human did not take the released port");

            // A reconnecting human queues like anyone else and waits for a port.
            var rejoin = Queue(Session("bravo"));
            await Settle();
            Check(!rejoin.IsCompleted && budget.InUse == 3, "Reconnect bypassed the DME port cap");
            await Disconnect("charlie");
            await Until(() => rejoin.IsCompleted);
            Check((await rejoin).Result == 0 && budget.InUse == 3, "Reconnect did not take a freed port");
        }
    });

    /// <summary>A party waits until every member has a port and enters as a unit;
    /// a party larger than every forwarded port is refused.</summary>
    public static void DmePortRegistryParties() => WithDirectory(directory =>
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try
        {
            VerifyPortHeldParty(directory, 3).GetAwaiter().GetResult();
            VerifyPortHeldParty(directory, 1).GetAwaiter().GetResult();
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    });

    static async Task VerifyPortHeldParty(string directory, int capacity)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, $"party-ports-{capacity}.json"));
        foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)0), ("charlie", (byte)1) })
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] },
                account).Result == MagNetResult.Success, "Party port fixture character creation");
        await using var worker = new RegistryWorker("first") { SupportsParties = true };
        await using var spare = new RegistryWorker("second") { SupportsParties = true };
        JsonObject Row(string id, int port) => new() { ["worker_id"] = id, ["worker_host"] = "127.0.0.1", ["worker_port"] = port,
            ["minimum_players"] = 3, ["maximum_players"] = 3, ["minimum_players_per_team"] = new JsonArray(1, 1), ["draining"] = false };
        var config = new JsonObject { ["format"] = "mag_world_registry_v1", ["application_id"] = 20231, ["client_address"] = "192.0.2.9",
            ["client_tcp_port"] = 12073, ["party_invite_seconds"] = 30, ["workers"] = new JsonArray(Row("first", worker.Port), Row("second", spare.Port)) };
        var budget = new MagDmePortBudget(null, capacity);
        await using var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store, portBudget: budget);
        await registry.InitializeAsync();
        MagMlsSession Session(string account) => MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id);
        var leader = Session("alpha"); var member = Session("bravo"); var solo = Session("charlie");
        var leaderMessages = new ConcurrentQueue<MagMessage>();
        var memberMessages = new ConcurrentQueue<MagMessage>();
        var soloMessages = new ConcurrentQueue<MagMessage>();
        registry.RegisterPartySession(leader, () => true, leaderMessages.Enqueue, null);
        registry.RegisterPartySession(member, () => true, memberMessages.Enqueue, null);
        Check(registry.RoutePartyMembership(leader, new MagRetailPartyInvite { Inviter = leader.AccountId, Invitee = member.AccountId }),
            "Registry did not route private invite");
        uint partyId = memberMessages.OfType<MagRetailPartyInvite>().Single().PartyId;
        Check(registry.RoutePartyMembership(member, new MagRetailPartyInviteResponse
            { PartyId = partyId, Inviter = leader.AccountId, Responder = member.AccountId }), "Registry did not route private answer");
        bool Success(ConcurrentQueue<MagMessage> queue) => queue.OfType<MagJoinGameResponse>().Any(r => r.Result == 0);
        async Task Until(Func<bool> condition)
        {
            using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            while (!condition()) { await registry.TickAsync(); await Task.Delay(20, deadline.Token); }
        }

        if (capacity < 2)
        {
            // The group can never fit: refuse it as a unit with the ordinary failure.
            registry.QueueJoin(leader, new MagRetailJoinGameRequest { TransactionId = 2, Preference = 1, Value = 2, PartyId = partyId },
                leaderMessages.Enqueue, () => true);
            Check(leaderMessages.OfType<MagJoinGameResponse>().SingleOrDefault()?.Result == 2 && registry.Status(1).QueueSizes[6] == 0,
                "Party larger than the forwarded ports was queued");
            return;
        }

        // Another world's human holds one of three ports: the party of two and
        // its opponent need three, so nobody is reserved yet.
        var elsewhere = budget.TryAcquire();
        registry.QueueJoin(leader, new MagRetailJoinGameRequest { TransactionId = 2, Preference = 1, Value = 2, PartyId = partyId },
            leaderMessages.Enqueue, () => true);
        registry.QueueJoin(solo, new MagRetailJoinGameRequest { TransactionId = 3, Preference = 1, Value = 2 }, soloMessages.Enqueue, () => true);
        for (int i = 0; i < 20; i++) { await registry.TickAsync(); await Task.Delay(20); }
        Check(!Success(leaderMessages) && !Success(memberMessages) && !Success(soloMessages) &&
            !worker.Requests.Concat(spare.Requests).Any(r => r["op"]!.GetValue<string>() is "reserve_party" or "reserve") &&
            budget.InUse == 1 && registry.Status(1).QueueSizes[6] == 2 && registry.Status(1).QueueSizes[7] == 1,
            "Party or its opponent was reserved without a port for every member");

        elsewhere.Dispose();
        await Until(() => Success(leaderMessages) && Success(memberMessages) && Success(soloMessages));
        Check(budget.InUse == 3 && worker.Requests.Concat(spare.Requests).Count(r => r["op"]!.GetValue<string>() == "reserve_party") == 1,
            "Party did not enter as one unit once its ports were free");
    }
}
