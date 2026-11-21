using System.Collections;
using System.Text.Json.Nodes;
using DotNetty.Transport.Channels.Embedded;
using RT.Common;
using RT.Models;
using RT.Models.MAG;
using Server.Dme.Models;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    private static void RouteFixture(LocalDmeHost host, JsonObject response)
    {
        Check(Parse(response, out var reply), "Synthetic cohort frame rejected by bridge codec");
        var gate = typeof(LocalDmeHost).GetField("_gate", HostInternals)!.GetValue(host)!;
        lock (gate)
            Check((bool)typeof(LocalDmeHost).GetMethod("Route", HostInternals)!.Invoke(host, new[] { reply })!,
                "Synthetic cohort frame was not routed");
    }

    private static JsonObject CohortReply() => new()
    {
        ["v"] = 2, ["request_id"] = "synthetic", ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray()
    };

    private static JsonObject TerminalReply(string match, params JsonObject[] members)
    {
        var response = CohortReply();
        response["match_terminal"] = new JsonObject
        {
            ["world_id"] = members[0]["world_id"]!.DeepClone(), ["match_id"] = match, ["reason"] = "attacker_victory",
            ["reservation_ids"] = new JsonArray(members.Select(member => member["reservation_id"]!.DeepClone()).ToArray())
        };
        return response;
    }

    private static ClientObject ConnectFixture(LocalDmeHost host, JsonObject reservation)
    {
        var token = reservation["access_key"]!.GetValue<string>();
        var client = host.World.Manager.GetClientByAccessToken(token);
        Check(client != null, "Accepted cohort admission did not register its access token");
        Check(host.TryBind(client, new RT_MSG_CLIENT_CONNECT_TCP_AUX_UDP
        {
            AppId = 20231, ARG1 = reservation["world_id"]!.GetValue<uint>(), SessionKey = client.SessionKey, AccessToken = token
        }), "Synthetic cohort transport could not bind");
        client.OnTcpConnected(new EmbeddedChannel());
        client.OnConnectionCompleted();
        client.RecvFlag = RT_RECV_FLAG.RECV_NOTIFICATION;
        host.Connected(client);
        return client;
    }

    public static void IndependentMatchCohortAdmission() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        Create(store, "bravo", "Bravo");
        Create(store, "charlie", "Charlie");
        Create(store, "delta", "Delta");
        Create(store, "echo", "Echo");
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var settings = Server.Medius.Program.Settings;
            int previousApp = settings.MagApplicationId;
            settings.MagApplicationId = 20231;
            var previousPlugins = Server.Dme.Program.Plugins;
            Server.Dme.Program.Plugins = new Server.Plugins.PluginsManager(directory);
            var enteredAck = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var respondAck = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var enteredOldEvent = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var respondOldEvent = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var enteredLiveEvent = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            string retiredEventOwner = null;
            int refuseNextReserve = 0;
            try
            {
                await using var bridge = new FixtureBridge(new[] { "independent_match_cohorts_v1" })
                {
                    CustomizeReply = (q, r) =>
                    {
                        var operation = q["op"]!.GetValue<string>();
                        if (operation == "release") { r["ok"] = false; r["error"] = "synthetic_release_unacknowledged"; }
                        if (operation == "reserve")
                        {
                            switch (Interlocked.Exchange(ref refuseNextReserve, 0))
                            {
                                case 1: r["ok"] = false; r["error"] = "results_cohort_limit"; break;
                                case 2: r["request_id"] = "synthetic-uncertain-reserve"; break;
                            }
                        }
                        return r;
                    },
                    BeforeReply = async q =>
                    {
                        switch (q["op"]!.GetValue<string>())
                        {
                            case "progression_committed":
                                enteredAck.TrySetResult();
                                await respondAck.Task;
                                break;
                            case "event" when q["reservation_id"]!.GetValue<string>() == retiredEventOwner:
                                enteredOldEvent.TrySetResult();
                                await respondOldEvent.Task;
                                break;
                            case "event":
                                enteredLiveEvent.TrySetResult();
                                break;
                        }
                    }
                };
                await using var host = new LocalDmeHost(bridge.Client, store);
                await MakeReady(host, bridge, 20231);
                MagMlsSession Session(string account) => MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id);
                async Task<JsonObject> Admit(MagMlsSession session)
                {
                    Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Expected a fresh cohort admission despite outstanding terminal ownership");
                    return bridge.Latest("reserve");
                }

                var alpha = Session("alpha");
                var oldAlpha = await Admit(alpha);
                var oldBravo = await Admit(Session("bravo"));
                var oldCharlie = await Admit(Session("charlie"));
                var alphaClient = ConnectFixture(host, oldAlpha);
                var bravoClient = ConnectFixture(host, oldBravo);
                var firstTerminal = TerminalReply("synthetic-first-round", oldAlpha, oldBravo, oldCharlie);
                RouteFixture(host, firstTerminal);
                await Release(host, oldCharlie).WaitAsync(AdmissionTimeout);

                var oldReward = RewardReply("enemy_killed", "");
                var reward = oldReward["rewards"]![0]!;
                foreach (string key in new[] { "reservation_id", "world_id", "player_id" }) reward[key] = oldAlpha[key]!.DeepClone();
                Check(Parse(oldReward, out var oldReceipt), "Synthetic old cohort reward rejected by bridge codec");
                Task settling = InvokeHost(host, "SettleRewardsAsync", oldReceipt);
                JsonObject newAlpha;
                try
                {
                    await enteredAck.Task.WaitAsync(AdmissionTimeout);
                    newAlpha = await Admit(alpha);
                    Check(!settling.IsCompleted && newAlpha["character"]!["progression"]!["experience"]!.GetValue<int>() == 7,
                        "Fresh admission waited on the old receipt ACK or missed its durable account XP");
                }
                finally { respondAck.TrySetResult(); }
                await settling.WaitAsync(AdmissionTimeout);
                var delta = Session("delta");
                string ReservationId(JsonObject reserve) => reserve["reservation_id"]!.GetValue<string>();
                var owners = (IDictionary)typeof(LocalDmeHost).GetField("_rewardOwners", HostInternals)!.GetValue(host)!;
                int originalOwnerCount = owners.Count;
                Interlocked.Exchange(ref refuseNextReserve, 1);
                Check((await BeginJoin(host, delta).WaitAsync(AdmissionTimeout)).Result == 6, "Worker results-cohort limit did not give a retry response");
                Check(owners.Count == originalOwnerCount && !owners.Contains(ReservationId(bridge.Latest("reserve"))),
                    "Definitively uncreated reservation consumed durable reward-owner capacity");
                var newDelta = await Admit(delta);
                Interlocked.Exchange(ref refuseNextReserve, 2);
                Check((await BeginJoin(host, Session("echo")).WaitAsync(AdmissionTimeout)).Result == 2, "Uncertain reservation was accepted");
                Check(owners.Contains(ReservationId(bridge.Latest("reserve"))) && owners.Contains(ReservationId(oldAlpha)) &&
                    owners.Contains(ReservationId(newDelta)), "Cleanup discarded uncertain or previously admitted reward ownership");
                var newAlphaClient = ConnectFixture(host, newAlpha);
                var newDeltaClient = ConnectFixture(host, newDelta);
                Check(newAlpha["reservation_id"]!.GetValue<string>() != oldAlpha["reservation_id"]!.GetValue<string>() &&
                    newAlpha["access_key"]!.GetValue<string>() != oldAlpha["access_key"]!.GetValue<string>(), "Requeue reused its original transport identity");
                Check(newDeltaClient.DmeId == bravoClient.DmeId && newDeltaClient.DmeWorld.WorldId == bravoClient.DmeWorld.WorldId,
                    "A retired match held a native slot or changed the subscribed world ID");
                Check(host.CanUse(bravoClient) && host.CanUse(newDeltaClient), "New admission disconnected another account's results transport");

                // A delayed transport close must compare the object in both registries, even for the same MLS session.
                host.World.Manager.RemoveClient(alphaClient);
                Check(ReferenceEquals(host.World.Manager.GetClientBySessionKey(alpha.SessionKey), newAlphaClient), "Old close removed the new session index");
                newDeltaClient.HasJoined = true;
                await bravoClient.DmeWorld.OnPlayerJoined(bravoClient);
                Check(newDeltaClient.TcpSendMessageQueue.IsEmpty, "Old DME join notification crossed cohorts");

                RouteFixture(host, firstTerminal);
                var packets = CohortReply();
                packets["messages"] = new JsonArray(new JsonObject {
                    ["reservation_id"] = oldBravo["reservation_id"]!.DeepClone(), ["world_id"] = oldBravo["world_id"]!.DeepClone(),
                    ["player_id"] = oldBravo["player_id"]!.DeepClone(), ["plugin_id"] = 1, ["payload_hex"] = "aabb"
                });
                RouteFixture(host, packets);
                Check(bravoClient.TcpSendMessageQueue.TryDequeue(out var oldPacket) && oldPacket is RT_MSG_SERVER_PLUGIN_TO_APP &&
                    newDeltaClient.TcpSendMessageQueue.IsEmpty, "Old result publication reached the replacement player index");
                Check(host.Status(1).PlayersInGame.Sum(value => (int)value) == 2, "A late terminal frame changed active cohort occupancy");

                retiredEventOwner = oldBravo["reservation_id"]!.GetValue<string>();
                host.Event(bravoClient, "tcp", 0x3d, 1, new byte[] { 0x11, 0x00 });
                try
                {
                    await enteredOldEvent.Task.WaitAsync(AdmissionTimeout);
                    host.Event(newDeltaClient, "tcp", 0x3d, 1, new byte[] { 0x11, 0x00 });
                    var first = await Task.WhenAny(enteredLiveEvent.Task, Task.Delay(TimeSpan.FromSeconds(1)));
                    Check(first == enteredLiveEvent.Task, "An archived event reply blocked the live event mailbox");
                }
                finally { respondOldEvent.TrySetResult(); }

                await Release(host, oldBravo).WaitAsync(AdmissionTimeout);
                Check(host.CanUse(newDeltaClient) && ReferenceEquals(newDeltaClient.DmeWorld.Clients[newDeltaClient.DmeId], newDeltaClient),
                    "Old results teardown removed the next cohort's same-numbered player");
            }
            finally { Server.Dme.Program.Plugins = previousPlugins; settings.MagApplicationId = previousApp; }
        }
    });

    public static void IndependentFactionCohortFormation() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
        foreach (var (account, name, faction) in new[] { ("alpha", "Alpha", (byte)0), ("bravo", "Bravo", (byte)1), ("charlie", "Charlie", (byte)2) })
        {
            var request = new MagCreateCharacterRequest { AccountName = name, Faction = faction, UniverseId = 26, NpId = new byte[36] };
            Check(store.CreateForAccount(request, account).Result == MagNetResult.Success, "Synthetic faction character creation failed");
        }
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var settings = Server.Medius.Program.Settings;
            int previousApp = settings.MagApplicationId;
            settings.MagApplicationId = 20231;
            try
            {
                var profiles = (from byte attacker in new byte[] { 0, 1, 2 }
                                from byte defender in new byte[] { 0, 1, 2 }
                                where attacker != defender
                                select new JsonObject { ["profile_id"] = $"synthetic-{attacker}-{defender}", ["game_type"] = 2,
                                    ["attacking_faction"] = attacker, ["defending_faction"] = defender, ["maximum_players"] = 8, ["team_capacity"] = 4 }).ToArray();
                await using var bridge = new FixtureBridge(new[] { "match_profiles_v1", "independent_match_cohorts_v1" })
                {
                    CustomizeReply = (request, reply) =>
                    {
                        switch (request["op"]!.GetValue<string>())
                        {
                            case "hello":
                                reply["match_profiles"] = new JsonArray(profiles.Select(profile => profile.DeepClone()).ToArray());
                                break;
                            case "select_match_profile":
                                var profile = profiles.Single(row => row["profile_id"]!.GetValue<string>() == request["profile_id"]!.GetValue<string>());
                                var selected = new JsonObject { ["formation_id"] = request["formation_id"]!.DeepClone() };
                                foreach (string key in new[] { "profile_id", "game_type", "attacking_faction", "defending_faction" })
                                    selected[key] = profile[key]!.DeepClone();
                                reply["selected_match"] = selected;
                                break;
                            case "release":
                                reply["ok"] = false;
                                reply["error"] = "synthetic_release_unacknowledged";
                                break;
                        }
                        return reply;
                    }
                };
                await using var host = new LocalDmeHost(bridge.Client, store);
                await MakeReady(host, bridge, 20231);
                MagMlsSession Session(string account) => MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id);
                var alpha = Session("alpha");
                int transaction = 1;
                Task<MagJoinGameResponse> QueueJoin(MagMlsSession session)
                {
                    var response = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                    host.QueueJoin(session, new MagRetailJoinGameRequest { TransactionId = transaction++, Preference = 1, Value = 2 },
                        message => { if (message is MagJoinGameResponse joined) response.TrySetResult(joined); }, () => true);
                    return response.Task;
                }

                async Task Form(MagMlsSession first, MagMlsSession second)
                {
                    var firstJoin = QueueJoin(first);
                    var secondJoin = QueueJoin(second);
                    var deadline = DateTime.UtcNow + AdmissionTimeout;
                    while (DateTime.UtcNow < deadline && (!firstJoin.IsCompleted || !secondJoin.IsCompleted))
                    {
                        await Task.Delay(10);
                        // The first ticket's background pump may observe the queue
                        // before its partner is added. Reproduce the next lobby tick.
                        typeof(LocalDmeHost).GetMethod("StartMatchmakingPump", HostInternals)!.Invoke(host, null);
                    }
                    Check(firstJoin.IsCompletedSuccessfully && secondJoin.IsCompletedSuccessfully &&
                        (await firstJoin).Result == 0 && (await secondJoin).Result == 0, "Faction cohort did not admit both eligible humans");
                }

                await Form(alpha, Session("bravo"));
                var initial = bridge.Requests.Where(request => request["op"]!.GetValue<string>() == "reserve").ToArray();
                var original = bridge.Latest("select_match_profile");
                var oldBravo = initial.Single(reservation => reservation["account_id"]!.GetValue<uint>() == store.ReadForAccount("bravo").Single().Id);
                var bravo = ConnectFixture(host, oldBravo);
                RouteFixture(host, TerminalReply("synthetic-faction-round-1", initial));
                await Form(alpha, Session("charlie"));
                var next = bridge.Latest("select_match_profile");
                Check(next["formation_id"]!.GetValue<string>() != original["formation_id"]!.GetValue<string>() &&
                    new[] { "synthetic-0-2", "synthetic-2-0" }.Contains(next["profile_id"]!.GetValue<string>()),
                    "Fresh opposing factions reused the old formation or profile");
                Check(host.CanUse(bravo), "Faction formation closed an unrelated result client");
            }
            finally { settings.MagApplicationId = previousApp; }
        }
    });
}
