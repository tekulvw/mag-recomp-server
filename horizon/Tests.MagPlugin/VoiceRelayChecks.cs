using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Server.Dme;
using Server.Medius;

// Synthetic voice relay checks: invented clients, groups and frames; no capture or game data.
internal static class VoiceRelayChecks
{
    static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }

    const string A = "synthetic-voice-reservation-01";
    const string B = "synthetic-voice-reservation-02";

    static string Member(string id, int player, int side, string listen, string transmit) =>
        $"{{\"reservation_id\":\"{id}\",\"player_id\":{player},\"side\":{side},\"listen\":[{listen}],\"transmit\":[{transmit}]}}";

    static string Frame(string members, ulong revision = 7, bool ok = true) =>
        $"{{\"v\":2,\"request_id\":\"synthetic-world\",\"ok\":{(ok ? "true" : "false")},\"error\":{(ok ? "null" : "\"synthetic\"")}," +
        $"\"messages\":[],\"voice_groups\":{{\"revision\":{revision},\"members\":[{members}]}}}}";

    static bool Parse(string json, out GameplayBridgeResponse reply) =>
        GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(json), "synthetic-world", true, out reply);

    public static void Options()
    {
        var off = VoiceRelayOptions.FromEnvironment(_ => null);
        Check(!off.Enabled && off.Tier == VoiceRelayTier.Squad, "voice relay is off and squad-only by default");
        foreach (var value in new[] { "", "0", "true", " 1", "yes" })
            Check(!VoiceRelayOptions.FromEnvironment(name => name == VoiceRelayOptions.Variable ? value : null).Enabled, $"enabled by '{value}'");
        var on = VoiceRelayOptions.FromEnvironment(name => name == VoiceRelayOptions.Variable ? "1" : null);
        Check(on.Enabled && on.Tier == VoiceRelayTier.Squad, "MAG_VOICE_RELAY=1 relays squad groups first");
        var all = VoiceRelayOptions.FromEnvironment(name =>
            name == VoiceRelayOptions.Variable ? "1" : name == VoiceRelayOptions.TierVariable ? "all" : null);
        Check(all.Enabled && all.Tier == VoiceRelayTier.All, "MAG_VOICE_RELAY_TIER=all");
        foreach (var bad in new[] { "ALL", "platoon", " squad", "proximity" })
        {
            bool threw = false;
            try { VoiceRelayOptions.FromEnvironment(name => name == VoiceRelayOptions.TierVariable ? bad : null); }
            catch (ArgumentException) { threw = true; }
            Check(threw, $"tier '{bad}' accepted");
        }
        for (int group = -1; group <= 340; group++)
        {
            bool squad = group >= 20 && group <= 51, staticTier = group >= 0 && group <= 51;
            Check(on.Serves(group) == squad, $"squad tier serves {group}");
            Check(all.Serves(group) == staticTier, $"all tiers serve {group}");
        }
        Check(VoiceRelayOptions.BudgetBytesPerSecond == 2 * VoiceRelayOptions.NominalBytesPerSecond, "budget is twice nominal");
    }

    public static void Codec()
    {
        var json = Frame(Member(A, 1, 0, "0,12,20", "20") + "," + Member(B, 2, 1, "1,16,36", "36"));
        Check(Parse(json, out var reply) && reply.VoiceGroups != null, "voice_groups frame refused");
        var frame = reply.VoiceGroups;
        Check(frame.Revision == 7 && frame.Members.Count == 2, "revision and members");
        var first = frame.Members[0];
        Check(first.ReservationId == A && first.PlayerId == 1 && first.Side == 0 &&
              first.Listen.SequenceEqual(new[] { 0, 12, 20 }) && first.Transmit.SequenceEqual(new[] { 20 }), "member fields");
        Check(Parse(Frame(""), out reply) && reply.VoiceGroups.Members.Count == 0, "an empty table is a table");
        Check(Parse("{\"v\":2,\"request_id\":\"synthetic-world\",\"ok\":true,\"error\":null,\"messages\":[]}", out reply) &&
              reply.VoiceGroups == null, "an ordinary frame has no table");
        Check(Parse(Frame(Member(A, 1, 0, "0,12,20,83,90,91,92,338", "20,83")), out reply) &&
              reply.VoiceGroups.Members[0].Listen.Count == 8, "eight listen groups, proximity included");

        foreach (var bad in new[]
        {
            Frame(Member(A, 1, 0, "0", "20"), revision: 0),
            Frame(Member(A, 1, 2, "0", "20")),
            Frame(Member(A, 1, 0, "0,52", "20")),
            Frame(Member(A, 1, 0, "0", "82")),
            Frame(Member(A, 1, 0, "0", "339")),
            Frame(Member(A, 1, 0, "-1", "20")),
            Frame(Member(A, 1, 0, "0,0", "20")),
            Frame(Member(A, 1, 0, "0,1,2,3,4,5,6,7,8", "20")),
            Frame(Member(A, 1, 0, "0", "\"20\"")),
            Frame(Member(A, 1, 0, "0", "20") + "," + Member(B, 1, 0, "0", "20")),
            Frame(Member(A, 1, 0, "0", "20") + "," + Member(A, 2, 0, "0", "20")),
            Frame(Member("synthetic-short", 1, 0, "0", "20")),
            Frame(Member(A, 70000, 0, "0", "20")),
            Frame(Member(A, 1, 0, "0", "20").Replace("}", ",\"extra\":1}")),
            Frame(Member(A, 1, 0, "0", "20")).Replace("\"members\"", "\"people\""),
            Frame(Member(A, 1, 0, "0", "20"), ok: false),
        })
            Check(!Parse(bad, out _), "accepted: " + bad);

        Check(Parse("{\"v\":2,\"request_id\":\"synthetic-world\",\"ok\":true,\"error\":null,\"messages\":[],\"capabilities\":[\"voice_groups_v1\"]}", out reply) &&
              reply.SupportsVoiceGroups, "capability");
        Check(Parse("{\"v\":2,\"request_id\":\"synthetic-world\",\"ok\":true,\"error\":null,\"messages\":[],\"capabilities\":[\"event_replay_v1\"]}", out reply) &&
              !reply.SupportsVoiceGroups, "no capability");

        var table = new VoiceRelayTable(frame);
        Check(table.Revision == 7 && table.Find(2)?.ReservationId == B && table.Find(3) == null, "table lookup");
        bool duplicate = false;
        try { new VoiceRelayTable(new VoiceGroupsFrame(1, new[] { first, first })); }
        catch (ArgumentException) { duplicate = true; }
        Check(duplicate, "a table refuses a duplicate client");
    }

    static VoiceGroupsMember Entry(int player, int side, int[] listen, int[] transmit) =>
        new VoiceGroupsMember($"synthetic-voice-reservation-{player:00}", (ushort)player, (byte)side, listen, transmit);

    // Side 0: two members of squad 0 (group 20), a squad leader of squad 1, a platoon leader of
    // squad 2 in a world with several platoons, an OIC in squad 4 (platoon 1), and an unplaced
    // player. Side 1: a member of squad 16, and an enemy entry that names a side-0 group.
    static VoiceRelayTable World() => new VoiceRelayTable(new VoiceGroupsFrame(3, new[]
    {
        Entry(1, 0, new[] { 0, 12, 20 }, new[] { 20 }),
        Entry(2, 0, new[] { 0, 12, 20 }, new[] { 20 }),
        Entry(3, 0, new[] { 0, 4, 12, 21 }, new[] { 4, 21 }),
        Entry(4, 0, new[] { 0, 2, 4, 12, 22 }, new[] { 2, 4, 12, 22 }),
        Entry(5, 0, new[] { 0, 2, 13, 24 }, new[] { 0, 2, 5, 24 }),
        Entry(6, 1, new[] { 1, 16, 36 }, new[] { 36 }),
        Entry(7, 1, new[] { 1, 20 }, new[] { 20, 52 }),
        Entry(8, 0, new[] { 0 }, Array.Empty<int>()),
    }));

    static readonly VoiceRelayOptions Squad = new VoiceRelayOptions(true, VoiceRelayTier.Squad);
    static readonly VoiceRelayOptions All = new VoiceRelayOptions(true, VoiceRelayTier.All);

    static VoiceRelayDecision Decide(VoiceRelayOptions options, int talker, int group, VoiceRelayTable table = null,
        bool bound = true, bool muted = false, Func<VoiceGroupsMember, bool> recipients = null, VoiceTalkerBudget budget = null,
        int bytes = 37, long now = 0) =>
        VoiceRelay.Decide(options, table ?? World(), talker, group, bytes, bound, muted,
            recipients != null ? recipients : (Func<VoiceGroupsMember, bool>)(_ => true), budget ?? new VoiceTalkerBudget(), now);

    static void Relays(VoiceRelayDecision decision, int[] expected, string message) =>
        Check(decision.Relayed && decision.Recipients.SequenceEqual(expected),
              $"{message}: {VoiceRelayDecision.Name(decision.Drop)} [{string.Join(",", decision.Recipients)}]");

    static void Drops(VoiceRelayDecision decision, VoiceDropReason reason, string message) =>
        Check(!decision.Relayed && decision.Drop == reason && decision.Recipients.Count == 0,
              $"{message}: {VoiceRelayDecision.Name(decision.Drop)}");

    public static void Decisions()
    {
        // Squad chat reaches the squad: never the talker, never the other side.
        Relays(Decide(Squad, 1, 20), new[] { 2 }, "squad chat");
        Relays(Decide(Squad, 2, 20), new[] { 1 }, "squad chat back");

        // Checks in order.
        Drops(Decide(VoiceRelayOptions.Off, 1, 20), VoiceDropReason.RelayOff, "relay off");
        Drops(Decide(new VoiceRelayOptions(false, VoiceRelayTier.All), 1, 20), VoiceDropReason.RelayOff, "relay off, any tier");
        Drops(Decide(Squad, 1, 20, bound: false), VoiceDropReason.Unbound, "unbound talker");
        Drops(Decide(Squad, 9, 20), VoiceDropReason.Unbound, "talker without a table entry");
        Drops(VoiceRelay.Decide(Squad, null, 1, 20, 37, true, false, _ => true, new VoiceTalkerBudget(), 0), VoiceDropReason.Unbound, "no table yet");
        Drops(Decide(Squad, 1, 20, muted: true), VoiceDropReason.Muted, "muted");
        Drops(Decide(Squad, 1, 21), VoiceDropReason.NotAllowedGroup, "another squad");
        Drops(Decide(All, 1, 12), VoiceDropReason.NotAllowedGroup, "a member cannot broadcast");
        Drops(Decide(All, 7, 52), VoiceDropReason.NotAllowedGroup, "party groups are never served");
        Drops(Decide(All, 8, 0), VoiceDropReason.NotAllowedGroup, "no squad, nothing to transmit");

        // The first rollout relays squad groups only.
        Drops(Decide(Squad, 3, 4), VoiceDropReason.NotAllowedGroup, "platoon chat off in the squad tier");
        Drops(Decide(Squad, 5, 0), VoiceDropReason.NotAllowedGroup, "team broadcast off in the squad tier");
        Relays(Decide(Squad, 3, 21), Array.Empty<int>(), "a squad leader alone in its squad");

        // Every tier.
        Relays(Decide(All, 3, 4), new[] { 4 }, "platoon chat: the platoon leader and squad leaders");
        Relays(Decide(All, 4, 4), new[] { 3 }, "platoon chat from the platoon leader");
        Relays(Decide(All, 4, 2), new[] { 5 }, "company chat: the OIC and platoon leaders");
        Relays(Decide(All, 5, 2), new[] { 4 }, "company chat from the OIC");
        Relays(Decide(All, 4, 12), new[] { 1, 2, 3 }, "platoon broadcast: the whole platoon");
        Relays(Decide(All, 5, 0), new[] { 1, 2, 3, 4, 8 }, "team broadcast: the side, unplaced included");
        Relays(Decide(All, 5, 5), Array.Empty<int>(), "platoon chat with nobody listening");
        Relays(Decide(All, 6, 36), Array.Empty<int>(), "the other side's squad");

        // Only bound recipients whose reservation matches.
        Relays(Decide(Squad, 1, 20, recipients: member => member.PlayerId != 2), Array.Empty<int>(), "unbound recipient");
        Relays(Decide(All, 5, 0, recipients: member => member.PlayerId % 2 == 0), new[] { 2, 4, 8 }, "bound recipients only");

        // Budget: twice the nominal rate, one second of burst; drops never spend it.
        var budget = new VoiceTalkerBudget();
        for (int i = 0; i < 100; i++) Decide(Squad, 1, 21, budget: budget, bytes: 248);
        int allowed = 0;
        for (int i = 0; i < 10; i++) if (Decide(Squad, 1, 20, budget: budget, bytes: 248).Relayed) allowed++;
        Check(allowed == VoiceRelayOptions.BudgetBytesPerSecond / 248, $"burst allowance: {allowed}");
        Drops(Decide(Squad, 1, 20, budget: budget, bytes: 248), VoiceDropReason.OverBudget, "over budget");
        Relays(Decide(Squad, 2, 20, budget: budget, bytes: 248), new[] { 1 }, "each talker has its own budget");
        Relays(Decide(Squad, 1, 20, budget: budget, bytes: 248, now: 1000), new[] { 2 }, "the budget refills");

        // The nominal rate (37 bytes every 40 ms) always passes; three times it does not.
        var steady = new VoiceTalkerBudget();
        for (long t = 0; t < 10_000; t += 40)
            Check(Decide(Squad, 1, 20, budget: steady, bytes: 37, now: t).Relayed, $"nominal rate dropped at {t} ms");
        var flood = new VoiceTalkerBudget();
        int floodDrops = 0;
        for (long t = 0; t < 10_000; t += 40)
            if (!Decide(Squad, 1, 20, budget: flood, bytes: 111, now: t).Relayed) floodDrops++;
        Check(floodDrops > 50, $"three times the nominal rate kept: {floodDrops} drops");

        // Proximity: every frame also goes to the talker's own proximity group, 83 + its DME client index.
        Check(VoiceGroupNumbers.ProximityGroup(0) == 83 && VoiceGroupNumbers.ProximityGroup(255) == 338 &&
              VoiceGroupNumbers.ProximityGroup(256) == -1 && VoiceGroupNumbers.ProximityGroup(-1) == -1, "proximity group numbers");
        Drops(Decide(All, 1, 84), VoiceDropReason.ProximityOff, "proximity off: the talker's own copy");
        Drops(Decide(Squad, 6, 89), VoiceDropReason.ProximityOff, "proximity off in the squad tier too");
        Drops(Decide(All, 1, 85), VoiceDropReason.NotAllowedGroup, "another client's proximity group");
        Drops(Decide(All, 1, 84, muted: true), VoiceDropReason.Muted, "a muted talker's proximity copy");
        Drops(Decide(All, 9, 92), VoiceDropReason.Unbound, "an unknown talker's proximity copy");
        // With a proximity source, listeners near the talker hear its group, enemies included.
        var near = new VoiceRelayTable(new VoiceGroupsFrame(4, new[]
        {
            Entry(1, 0, new[] { 0, 12, 20 }, new[] { 20, 84 }),
            Entry(2, 0, new[] { 0, 12, 20 }, new[] { 20, 85 }),
            Entry(3, 0, new[] { 0, 4, 12, 21, 84 }, new[] { 4, 21, 86 }),
            Entry(6, 1, new[] { 1, 16, 36, 84 }, new[] { 36, 89 }),
        }));
        Relays(Decide(All, 1, 84, table: near), new[] { 3, 6 }, "proximity reaches nearby listeners on both sides");
        Relays(Decide(Squad, 1, 84, table: near), new[] { 3, 6 }, "proximity does not depend on the static tier");
        Relays(Decide(All, 2, 85, table: near), Array.Empty<int>(), "proximity with nobody near");
        Drops(Decide(All, 1, 85, table: near), VoiceDropReason.NotAllowedGroup, "never on another talker's proximity group");
        Relays(Decide(All, 1, 84, table: near, recipients: member => member.PlayerId != 6), new[] { 3 }, "bound proximity listeners only");
        // Each copy has its own budget, so the paired copies of one stream never starve each other.
        var paired = new VoiceTalkerBudget();
        for (long t = 0; t < 10_000; t += 40)
        {
            Check(Decide(All, 1, 20, table: near, budget: paired, bytes: 51, now: t).Relayed, $"channel copy dropped at {t} ms");
            Check(Decide(All, 1, 84, table: near, budget: paired, bytes: 51, now: t).Relayed, $"proximity copy dropped at {t} ms");
        }
        paired.Forget(1);

        // Counters by reason.
        var counters = new VoiceRelayCounters();
        counters.Record(Decide(Squad, 1, 20), 37);
        counters.Record(Decide(All, 5, 0), 37);
        counters.Record(Decide(VoiceRelayOptions.Off, 1, 20), 37);
        counters.Record(Decide(Squad, 1, 21), 37);
        Check(counters.ReceivedPackets == 4 && counters.ReceivedBytes == 148 && counters.RelayedPackets == 2 &&
              counters.RelayedBytes == 37 * 6 && counters.Dropped(VoiceDropReason.RelayOff) == 1 &&
              counters.Dropped(VoiceDropReason.NotAllowedGroup) == 1 && counters.Dropped(VoiceDropReason.OverBudget) == 0,
              "counters: " + counters);
        Check(counters.ToString().Contains("relay_off=1") && counters.ToString().Contains("not_allowed_group=1"), "counter line: " + counters);
        counters.Record(Decide(All, 1, 84), 51);
        Check(counters.Dropped(VoiceDropReason.ProximityOff) == 1 && counters.ToString().Contains("proximity_off=1"), "proximity_off counter: " + counters);
    }

    public static void Subscription() => SubscriptionAsync().GetAwaiter().GetResult();

    static async Task SubscriptionAsync()
    {
        foreach (var ask in new[] { false, true })
        {
            var listener = new TcpListener(IPAddress.Loopback, 0);
            listener.Start();
            JsonNode request = null;
            var worker = Task.Run(async () =>
            {
                using var socket = await listener.AcceptTcpClientAsync();
                using var stream = socket.GetStream();
                using var reader = new StreamReader(stream);
                using var writer = new StreamWriter(stream) { AutoFlush = true };
                request = JsonNode.Parse(await reader.ReadLineAsync());
                await writer.WriteLineAsync("{\"v\":2,\"request_id\":\"synthetic-world\",\"ok\":true,\"error\":null,\"messages\":[]}");
                await writer.WriteLineAsync(Frame(Member(A, 1, 0, "0,12,20", "20")));
            });
            var frames = new List<GameplayBridgeResponse>();
            try
            {
                var bridge = new GameplayBridge("127.0.0.1", ((IPEndPoint)listener.LocalEndpoint).Port);
                await bridge.SubscribeWorldAsync(1, "synthetic-world", false, null, ask,
                    reply => { frames.Add(reply); return Task.CompletedTask; }, CancellationToken.None).WaitAsync(TimeSpan.FromSeconds(10));
            }
            catch (Exception error) when (error is IOException || error is SocketException) { } // The synthetic worker closed the stream.
            finally { listener.Stop(); }
            await worker;
            Check(request?["op"]?.GetValue<string>() == "subscribe_world", "subscription request");
            Check((request["voice_groups"] != null) == ask && (!ask || request["voice_groups"].GetValue<bool>()),
                  $"voice_groups requested={request["voice_groups"]?.ToJsonString() ?? "absent"} ask={ask}");
            Check(frames.Count == 2 && frames[0].VoiceGroups == null && frames[1].VoiceGroups?.Revision == 7, $"frames: {frames.Count}");
        }
    }
}
