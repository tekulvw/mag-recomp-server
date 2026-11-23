using System.Collections;
using System.Collections.Concurrent;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    public static void PartyRegistryPlacement() => WithDirectory(directory =>
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try
        {
            VerifyPartyRegistry(directory, true).GetAwaiter().GetResult();
            VerifyPartyRegistry(directory, false).GetAwaiter().GetResult();
            VerifyPartyRegistry(directory, false, claimOne: true).GetAwaiter().GetResult();
            VerifyOpposingParties(directory, false).GetAwaiter().GetResult();
            VerifyOpposingParties(directory, true).GetAwaiter().GetResult();
            VerifyOpposingParties(directory, false, twoOnFirstSide: true).GetAwaiter().GetResult();
            VerifyMissingForwardedParty(directory).GetAwaiter().GetResult();
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    });

    public static void PartyTerminalRequeue() => WithDirectory(directory =>
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try { VerifyPartyTerminalRequeue(directory).GetAwaiter().GetResult(); }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    });

    static async Task VerifyPartyTerminalRequeue(string directory)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "party-terminal.json"));
        string[] accounts = { "alpha", "bravo", "charlie", "delta" };
        for (int i = 0; i < accounts.Length; i++)
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = accounts[i],
                Faction = (byte)(i < 2 ? 0 : 1), UniverseId = 26, NpId = new byte[36] }, accounts[i]).Result
                == MagNetResult.Success, "Party terminal fixture character creation");

        await using var worker = new RegistryWorker("first") { SupportsParties = true };
        await using var spare = new RegistryWorker("second");
        JsonObject Worker(string id, int port, bool draining) => new() { ["worker_id"] = id,
            ["worker_host"] = "127.0.0.1", ["worker_port"] = port, ["minimum_players"] = 3,
            ["maximum_players"] = 3, ["minimum_players_per_team"] = new JsonArray(1, 1), ["draining"] = draining };
        var config = new JsonObject { ["format"] = "mag_world_registry_v1", ["application_id"] = 20231,
            ["client_address"] = "192.0.2.9", ["client_tcp_port"] = 12073, ["party_invite_seconds"] = 30,
            ["workers"] = new JsonArray(Worker("first", worker.Port, false), Worker("second", spare.Port, true)) };
        await using var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store);
        await registry.InitializeAsync();
        var sessions = accounts.Select(account => MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id)).ToArray();
        var output = sessions.Select(_ => new ConcurrentQueue<MagMessage>()).ToArray();
        var leader = sessions[0]; var member = sessions[1];
        registry.RegisterPartySession(leader, () => true, output[0].Enqueue, null);
        registry.RegisterPartySession(member, () => true, output[1].Enqueue, null);
        Check(registry.RoutePartyMembership(leader, new MagRetailPartyInvite { Inviter = leader.AccountId, Invitee = member.AccountId }),
            "Terminal fixture invite refused");
        uint partyId = output[1].OfType<MagRetailPartyInvite>().Single().PartyId;
        Check(registry.RoutePartyMembership(member, new MagRetailPartyInviteResponse
            { PartyId = partyId, Inviter = leader.AccountId, Responder = member.AccountId }), "Terminal fixture invite response refused");

        void Queue(int index, int transaction, uint id = 0) => registry.QueueJoin(sessions[index],
            new MagRetailJoinGameRequest { TransactionId = transaction, Preference = 1, Value = 2, PartyId = id },
            output[index].Enqueue, () => true);
        int Successes(int index) => output[index].OfType<MagJoinGameResponse>().Count(reply => reply.Result == 0);
        JsonObject[] Cohorts() => worker.Requests.Where(request => request["op"]!.GetValue<string>() == "reserve_party").ToArray();
        static string Id(JsonObject row, string key) => row[key]!.GetValue<string>();
        static JsonObject[] Owners(JsonObject cohort) => cohort["members"]!.AsArray().Select(node =>
        {
            var owner = node!.DeepClone().AsObject();
            owner["world_id"] = cohort["world_id"]!.DeepClone();
            return owner;
        }).ToArray();
        using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(15));
        async Task Until(Func<bool> condition)
        {
            while (!condition()) { await registry.TickAsync(); await Task.Delay(20, deadline.Token); }
        }

        Queue(0, 2, partyId);
        Queue(2, 3);
        await Until(() => Successes(0) == 1 && Successes(1) == 1 && Successes(2) == 1);
        var first = Cohorts().Single();
        var firstOwners = Owners(first);
        var oldOpponent = worker.Reservations.Single();
        var slot = ((IEnumerable)typeof(MagWorldRegistry).GetField("_slots", HostInternals)!.GetValue(registry)!).Cast<object>().First();
        var host = (LocalDmeHost)slot.GetType().GetField("Host")!.GetValue(slot)!;
        var claimed = firstOwners.Append(oldOpponent).Select(owner => ConnectFixture(host, owner)).ToArray();
        await Until(() => claimed.All(registry.CanUse));

        Queue(3, 4);
        await registry.TickAsync();
        Check(registry.Status(1).QueueSizes[7] == 1 && Successes(3) == 0 && Cohorts().Length == 1,
            "An opposite-faction waiter entered a full live match");
        Check(!registry.CancelPartyQueue(leader, partyId) && claimed.All(registry.CanUse),
            "Published cancellation evicted a claimed player");

        RouteFixture(host, TerminalReply("party-terminal-round", firstOwners.Append(oldOpponent).ToArray()));
        Volatile.Write(ref worker.AdmissionEpoch, 2);
        claimed[0].OnTcpDisconnected();
        await Until(() => worker.Requests.Any(request => request["op"]!.GetValue<string>() == "release" &&
            Id(request, "reservation_id") == Id(firstOwners[0], "reservation_id")));
        Check(registry.CanUse(claimed[1]) && registry.CanUse(claimed[2]) && Successes(3) == 0,
            "Leaving results evicted another claimed player or consumed the waiting ticket");

        Queue(0, 5, partyId);
        await Until(() => Successes(0) == 2 && Successes(1) == 2 && Successes(3) == 1);
        var second = Cohorts().Last();
        var secondOwners = Owners(second);
        Check(Cohorts().Length == 2 && first["admission_epoch"]!.GetValue<int>() == 1 && second["admission_epoch"]!.GetValue<int>() == 2 &&
            Id(first, "formation_id") != Id(second, "formation_id") && Id(first, "cohort_id") != Id(second, "cohort_id"),
            "Requeue reused a prior formation, cohort, or admission epoch");
        Check(firstOwners.Select(owner => owner["account_id"]!.GetValue<uint>())
                .SequenceEqual(secondOwners.Select(owner => owner["account_id"]!.GetValue<uint>())) &&
            firstOwners.Zip(secondOwners).All(pair => Id(pair.First, "reservation_id") != Id(pair.Second, "reservation_id") &&
                Id(pair.First, "access_key") != Id(pair.Second, "access_key")),
            "Requeue split the party, changed the roster, or reused transport identity");
        var roster = output[0].OfType<MagRetailPartyRoster>().Last();
        Check(roster.PartyId == partyId && roster.Status == 3 && roster.Members.SequenceEqual(new[] { leader.AccountId, member.AccountId }) &&
            registry.CanUse(claimed[2]) && !registry.CanUse(claimed[1]),
            "Second admission failed to preserve the party or released the unrelated results transport");
    }

    static async Task VerifyPartyRegistry(string directory, bool cancel, bool claimOne = false)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, $"party-{cancel}-{claimOne}.json"));
        foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)0), ("charlie", (byte)1) })
            Check(store.CreateForAccount(new MagCreateCharacterRequest
                { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] }, account).Result
                == MagNetResult.Success, "Party fixture character creation");
        await using var worker = new RegistryWorker("first") { SupportsParties = true };
        await using var spare = new RegistryWorker("second");
        var hold = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var asked = new TaskCompletionSource<JsonObject>(TaskCreationOptions.RunContinuationsAsynchronously);
        worker.DropNextPartyReply = !cancel;
        worker.BeforePartyReply = async request =>
        {
            if (request["op"]!.GetValue<string>() != "reserve_party") return;
            asked.TrySetResult(request);
            await hold.Task.WaitAsync(AdmissionTimeout);
        };

        JsonObject Worker(string id, int port) => new() { ["worker_id"] = id, ["worker_host"] = "127.0.0.1",
            ["worker_port"] = port, ["minimum_players"] = 3, ["maximum_players"] = 3,
            ["minimum_players_per_team"] = new JsonArray(1, 1), ["draining"] = false };
        var config = new JsonObject { ["format"] = "mag_world_registry_v1", ["application_id"] = 20231,
            ["client_address"] = "192.0.2.9", ["client_tcp_port"] = 12073, ["party_invite_seconds"] = 30,
            ["workers"] = new JsonArray(Worker("first", worker.Port), Worker("second", spare.Port)) };
        await using var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store);
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
        registry.QueueJoin(leader, new MagRetailJoinGameRequest { TransactionId = 2, Preference = 1, Value = 2, PartyId = partyId },
            leaderMessages.Enqueue, () => true);
        Check(registry.Status(1).QueueSizes[6] == 2, "Queued party members not counted");
        registry.QueueJoin(solo, new MagRetailJoinGameRequest { TransactionId = 3, Preference = 1, Value = 2 },
            soloMessages.Enqueue, () => true);

        async Task Until(Func<bool> condition)
        {
            using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            while (!condition()) { await registry.TickAsync(); await Task.Delay(20, deadline.Token); }
        }
        bool Success(ConcurrentQueue<MagMessage> queue) => queue.OfType<MagJoinGameResponse>().Any(r => r.Result == 0);
        try
        {
            await Until(() => asked.Task.IsCompleted);
            var request = await asked.Task;
            var members = request["members"]!.AsArray();
            Check(members.Count == 2 && members[0]["account_id"]!.GetValue<uint>() == leader.AccountId &&
                members[1]["account_id"]!.GetValue<uint>() == member.AccountId &&
                request["admission_epoch"]!.GetValue<ulong>() == 1, "Party bridge order or epoch mismatch");
            Check(!Success(leaderMessages) && !Success(memberMessages) && !Success(soloMessages) &&
                registry.Worlds.All(world => world.ReservedPlayers == 0), "Party success escaped before full acknowledgement");
            Check(!registry.CancelPartyQueue(member, partyId), "Party member cancelled the leader's queue");
            if (cancel) Check(registry.CancelPartyQueue(leader, partyId), "Leader could not cancel pending party cohort");
            hold.SetResult();
            if (cancel)
            {
                await Until(() => worker.Requests.Any(r => r["op"]!.GetValue<string>() == "release_party"));
                var rollback = worker.Requests.Single(r => r["op"]!.GetValue<string>() == "release_party");
                Check(rollback["cohort_id"]!.ToJsonString() == request["cohort_id"]!.ToJsonString() &&
                    rollback["admission_epoch"]!.ToJsonString() == request["admission_epoch"]!.ToJsonString(),
                    "Cancellation rolled back a different cohort");
                Check(!Success(leaderMessages) && !Success(memberMessages) && !Success(soloMessages),
                    "A cancelled formation published a success");
            }
            else
            {
                await Until(() => Success(leaderMessages) && Success(memberMessages) && Success(soloMessages));
                var attempts = worker.Requests.Where(r => r["op"]!.GetValue<string>() == "reserve_party").ToArray();
                Check(attempts.Length == 2 && attempts[0]["cohort_id"]!.ToJsonString() == attempts[1]["cohort_id"]!.ToJsonString() &&
                    attempts[0]["members"]!.ToJsonString() == attempts[1]["members"]!.ToJsonString(), "Uncertain party retry changed its immutable payload");
                var replies = new[] { leaderMessages, memberMessages, soloMessages }
                    .Select(queue => queue.OfType<MagJoinGameResponse>().Single(r => r.Result == 0)).ToArray();
                Check(replies.All(reply => reply.WorldId == replies[0].WorldId) &&
                    replies.Select(reply => Convert.ToHexString(reply.AccessKey)).Distinct().Count() == 3 &&
                    worker.Reservations.Count == 1 && spare.Reservations.Count == 0, "Party split into solo reservations or reached another worker");

                var slot = ((IEnumerable)typeof(MagWorldRegistry).GetField("_slots", HostInternals)!.GetValue(registry)!).Cast<object>().First();
                var host = (LocalDmeHost)slot.GetType().GetField("Host")!.GetValue(slot)!;
                Server.Dme.Models.ClientObject claimed = null;
                if (claimOne)
                {
                    var owned = members[1]!.DeepClone().AsObject();
                    owned["world_id"] = request["world_id"]!.DeepClone();
                    claimed = ConnectFixture(host, owned);
                }
                Check(!registry.CancelPartyQueue(member, partyId), "A nonleader cancelled the published party");
                Check(!registry.CancelPartyQueue(leader, partyId + 1), "A wrong party ID cancelled the published party");
                Check(registry.CancelPartyQueue(leader, partyId), "Leader could not cancel a published party with unclaimed members");
                var released = (claimOne ? members.Take(1) : members)
                    .Select(m => m["reservation_id"]!.GetValue<string>()).ToHashSet(StringComparer.Ordinal);
                await Until(() => worker.Requests.Count(r => r["op"]!.GetValue<string>() == "release") >= released.Count);
                Check(worker.Requests.Where(r => r["op"]!.GetValue<string>() == "release")
                    .Select(r => r["reservation_id"]!.GetValue<string>()).ToHashSet().SetEquals(released),
                    "Published party cancel released a claimed or unrelated owner");
                Check(registry.Worlds[0].ReservedPlayers == (claimOne ? 2 : 1) &&
                    (!claimOne || registry.CanUse(claimed)), "Published cancellation removed an active party member or the solo owner");
            }
        }
        finally { hold.TrySetResult(); }
    }

    static async Task VerifyOpposingParties(string directory, bool refuseSecond, bool twoOnFirstSide = false)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, $"opposing-{refuseSecond}-{twoOnFirstSide}.json"));
        string[] accounts = twoOnFirstSide ? new[] { "alpha", "bravo", "echo", "foxtrot", "charlie", "delta" } :
            new[] { "alpha", "bravo", "charlie", "delta" };
        int groupCount = accounts.Length / 2;
        for (int i = 0; i < accounts.Length; i++)
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = accounts[i],
                Faction = (byte)(i >= accounts.Length - 2 ? 1 : 0), UniverseId = 26, NpId = new byte[36] }, accounts[i]).Result
                == MagNetResult.Success, "Opposing party fixture character creation");
        await using var worker = new RegistryWorker("first") { SupportsParties = true };
        await using var spare = new RegistryWorker("second");
        var secondAsked = new TaskCompletionSource<JsonObject>(TaskCreationOptions.RunContinuationsAsynchronously);
        var finishSecond = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        bool IsSecond(JsonObject request) => request["members"]?[0]?["character"]?["faction"]?.GetValue<int>() == 1;
        worker.BeforePartyReply = async request =>
        {
            if (request["op"]!.GetValue<string>() != "reserve_party" || !IsSecond(request)) return;
            secondAsked.TrySetResult(request);
            await finishSecond.Task.WaitAsync(AdmissionTimeout);
        };
        worker.RefuseParty = request => refuseSecond && IsSecond(request);

        JsonObject Worker(string id, int port) => new() { ["worker_id"] = id, ["worker_host"] = "127.0.0.1",
            ["worker_port"] = port, ["minimum_players"] = accounts.Length, ["maximum_players"] = accounts.Length,
            ["minimum_players_per_team"] = new JsonArray(2, 2), ["draining"] = false };
        var config = new JsonObject { ["format"] = "mag_world_registry_v1", ["application_id"] = 20231,
            ["client_address"] = "192.0.2.9", ["client_tcp_port"] = 12073, ["party_invite_seconds"] = 30,
            ["workers"] = new JsonArray(Worker("first", worker.Port), Worker("second", spare.Port)) };
        await using var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store);
        await registry.InitializeAsync();
        var sessions = accounts.Select(account => MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id)).ToArray();
        var output = sessions.Select(_ => new ConcurrentQueue<MagMessage>()).ToArray();
        for (int i = 0; i < sessions.Length; i++)
            registry.RegisterPartySession(sessions[i], () => true, output[i].Enqueue, null);
        for (int i = 0; i < sessions.Length; i += 2)
        {
            var leader = sessions[i]; var member = sessions[i + 1];
            Check(registry.RoutePartyMembership(leader, new MagRetailPartyInvite { Inviter = leader.AccountId, Invitee = member.AccountId }),
                "Opposing party invite refused");
            uint partyId = output[i + 1].OfType<MagRetailPartyInvite>().Single().PartyId;
            Check(registry.RoutePartyMembership(member, new MagRetailPartyInviteResponse
                { PartyId = partyId, Inviter = leader.AccountId, Responder = member.AccountId }), "Opposing party response refused");
            registry.QueueJoin(leader, new MagRetailJoinGameRequest { TransactionId = i + 1, Preference = 1, Value = 2, PartyId = partyId },
                output[i].Enqueue, () => true);
        }
        async Task Until(Func<bool> condition)
        {
            using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            while (!condition()) { await registry.TickAsync(); await Task.Delay(20, deadline.Token); }
        }
        bool Success(ConcurrentQueue<MagMessage> queue) => queue.OfType<MagJoinGameResponse>().Any(r => r.Result == 0);
        try
        {
            await Until(() => secondAsked.Task.IsCompleted);
            Check(worker.Requests.Count(r => r["op"]!.GetValue<string>() == "reserve_party") == groupCount &&
                !output.Any(Success) && registry.Worlds.All(world => world.ReservedPlayers == 0),
                "First team published before the second party acknowledgement");
            finishSecond.SetResult();
            if (refuseSecond)
            {
                await Until(() => worker.Requests.Count(r => r["op"]!.GetValue<string>() == "release_party") >= 2);
                var reserve = worker.Requests.Where(r => r["op"]!.GetValue<string>() == "reserve_party").Take(2)
                    .Select(r => r["cohort_id"]!.GetValue<string>()).ToArray();
                var release = worker.Requests.Where(r => r["op"]!.GetValue<string>() == "release_party").Take(2)
                    .Select(r => r["cohort_id"]!.GetValue<string>()).ToArray();
                Check(reserve.Reverse().SequenceEqual(release) && !output.Any(Success), "Second party refusal did not roll back both exact cohorts");
            }
            else
            {
                await Until(() => output.All(Success));
                Check(registry.Worlds[0].ReservedPlayers == accounts.Length && worker.Reservations.IsEmpty && spare.Reservations.IsEmpty,
                    "Opposing parties did not stay grouped on the single selected worker");
            }
        }
        finally { finishSecond.TrySetResult(); }
    }


    static async Task VerifyMissingForwardedParty(string directory)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "missing-party.json"));
        foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1) })
            Check(store.CreateForAccount(new MagCreateCharacterRequest
                { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] }, account).Result
                == MagNetResult.Success, "Missing party fixture character creation");
        await using var worker = new RegistryWorker("first") { SupportsParties = true };
        await using var spare = new RegistryWorker("second");
        JsonObject Worker(string id, int port) => new() { ["worker_id"] = id, ["worker_host"] = "127.0.0.1",
            ["worker_port"] = port, ["minimum_players"] = 2, ["maximum_players"] = 4,
            ["minimum_players_per_team"] = new JsonArray(1, 1), ["draining"] = false };
        var config = new JsonObject { ["format"] = "mag_world_registry_v1", ["application_id"] = 20231,
            ["client_address"] = "192.0.2.9", ["client_tcp_port"] = 12073,
            ["workers"] = new JsonArray(Worker("first", worker.Port), Worker("second", spare.Port)) };
        await using var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store);
        await registry.InitializeAsync();
        var slot = ((IEnumerable)typeof(MagWorldRegistry).GetField("_slots", HostInternals)!.GetValue(registry)!).Cast<object>().First();
        var host = (LocalDmeHost)slot.GetType().GetField("Host")!.GetValue(slot)!;
        var replies = new ConcurrentQueue<MagJoinGameResponse>();
        using (host.BeginRegistryBatch())
        {
            Check(host.TryStageRegistryFormation("profile-0-1", expectedParties: 1),
                "Host did not stage an authored party formation");
            // The selected party vanished before forwarding. Two surviving solos
            // could satisfy startup, so a reduced batch would otherwise publish.
            foreach (string account in new[] { "alpha", "bravo" })
            {
                var session = MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id);
                host.QueueJoin(session, new MagRetailJoinGameRequest { Preference = 1, Value = 2 },
                    message => { if (message is MagJoinGameResponse reply) replies.Enqueue(reply); }, () => true);
            }
        }
        using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(12));
        while (replies.Count < 2)
        {
            await registry.TickAsync().WaitAsync(deadline.Token);
            await Task.Delay(20, deadline.Token);
        }
        Check(replies.All(reply => reply.Result == 6) && worker.Reservations.IsEmpty,
            "Host admitted an incomplete formation after a selected party went missing");
    }

}
