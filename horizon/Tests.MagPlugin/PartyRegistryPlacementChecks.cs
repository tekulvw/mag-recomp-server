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
            VerifyOpposingParties(directory, false).GetAwaiter().GetResult();
            VerifyOpposingParties(directory, true).GetAwaiter().GetResult();
            VerifyOpposingParties(directory, false, twoOnFirstSide: true).GetAwaiter().GetResult();
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    });

    static async Task VerifyPartyRegistry(string directory, bool cancel)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, $"party-{cancel}.json"));
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

}
