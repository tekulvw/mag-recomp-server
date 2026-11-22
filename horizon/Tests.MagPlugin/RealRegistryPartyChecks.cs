using System.Collections.Concurrent;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    public static void RealRegistryParties(string binary, string catalogs) => WithDirectory(directory =>
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try { VerifyAsync().GetAwaiter().GetResult(); }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }

        async Task VerifyAsync()
        {
            string[] accounts = { "alpha", "bravo", "charlie", "delta", "echo", "foxtrot" };
            var store = new LocalCharacterStore(Path.Combine(directory, "party-characters.json"));
            for (int i = 0; i < accounts.Length; i++)
                Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = accounts[i], NpId = new byte[36],
                    Faction = (byte)(i < 4 ? 0 : 1), UniverseId = 26 }, accounts[i]).Result == MagNetResult.Success,
                    "Synthetic party character setup failed");
            await using var first = await RealRegistryWorkerChecks.Worker.StartAsync("first", directory, binary, catalogs, parties: true);
            await using var second = await RealRegistryWorkerChecks.Worker.StartAsync("second", directory, binary, catalogs);
            JsonObject Worker(RealRegistryWorkerChecks.Worker worker) => new()
            {
                ["worker_id"] = worker.Id, ["worker_host"] = "127.0.0.1", ["worker_port"] = worker.Port,
                ["minimum_players"] = 6, ["maximum_players"] = 6,
                ["minimum_players_per_team"] = new JsonArray(2, 2), ["draining"] = false
            };
            var config = new JsonObject
            {
                ["format"] = "mag_world_registry_v1", ["application_id"] = 20231, ["client_address"] = "192.0.2.9",
                ["client_tcp_port"] = 12073, ["party_invite_seconds"] = 30,
                ["workers"] = new JsonArray(Worker(first), Worker(second))
            };
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
                    "Real party invite refused");
                uint partyId = output[i + 1].OfType<MagRetailPartyInvite>().Single().PartyId;
                Check(registry.RoutePartyMembership(member, new MagRetailPartyInviteResponse
                    { PartyId = partyId, Inviter = leader.AccountId, Responder = member.AccountId }), "Real party invite response refused");
                registry.QueueJoin(leader, new MagRetailJoinGameRequest { TransactionId = i + 1, Preference = 1, Value = 0, PartyId = partyId },
                    output[i].Enqueue, () => true);
            }
            using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(15));
            while (output.Any(queue => !queue.OfType<MagJoinGameResponse>().Any(reply => reply.Result == 0)))
            {
                await registry.TickAsync().WaitAsync(deadline.Token);
                await Task.Delay(10, deadline.Token);
            }
            var expectedWorld = registry.Worlds.Single(world => world.WorkerId == first.Id);
            var gameReplies = output.Select(queue => queue.OfType<MagJoinGameResponse>().Single(reply => reply.Result == 0)).ToArray();
            Check(expectedWorld.ReservedPlayers == 6 && gameReplies.All(reply => reply.WorldId == expectedWorld.WorldId) &&
                gameReplies.Select(reply => Convert.ToHexString(reply.AccessKey)).Distinct().Count() == 6,
                "Actual worker group descriptors did not name six unique owners in one world");
            var partyAudits = first.ReadJournal().Where(row => row["op"]?.GetValue<string>() == "party_reserved").ToArray();
            Check(partyAudits.Length == 3 && partyAudits.Select(row => row["party_reservation"]["squad"].GetValue<int>()).Distinct().Count() == 3 &&
                partyAudits.Select(row => row["party_reservation"]["team"].GetValue<int>()).Distinct().Count() == 2 &&
                partyAudits.All(row => row["party_reservation"]["reservation_ids"].AsArray().Count == 2) &&
                !second.ReadJournal().Any(row => row["op"]?.GetValue<string>() == "party_reserved"),
                "Actual worker did not reserve exactly three distinct authored squads");
        }
    });
}
