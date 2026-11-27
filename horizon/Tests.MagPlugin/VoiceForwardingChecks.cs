using System.Text;
using System.Text.Json.Nodes;
using RT.Common;
using RT.Models;
using Server.Common.Stream;
using Server.Dme;
using Server.Medius;

// Synthetic voice forwarding checks: invented groups, talkers and pattern bytes; no capture or game data.
internal static class VoiceForwardingChecks
{
    static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }

    /// <summary>A group send as a client builds it: the address block (a base group when the group is
    /// not 0), then the type-47 message with its voice header and pattern data.</summary>
    public static byte[] Uplink(int group, int sequence, int dataLength, byte media = 1)
    {
        var bytes = new List<byte>();
        if (group == 0) bytes.AddRange(new byte[] { 0x01, 0x01 });
        else bytes.AddRange(new byte[] { 0x81, (byte)group, (byte)(group >> 8), 0x01 });
        bytes.AddRange(new byte[] { 0x00, 0x2F, (byte)group, media, (byte)dataLength, (byte)(dataLength >> 8),
            (byte)sequence, (byte)(sequence >> 8), 0x00, 0x00 });
        for (int i = 0; i < dataLength; i++) bytes.Add((byte)(i * 7 + group + sequence));
        return bytes.ToArray();
    }

    /// <summary>The body of a message as the DME writes it to a listener.</summary>
    public static byte[] Body(BaseScertMessage message)
    {
        using var stream = new MemoryStream();
        using (var writer = new MessageWriter(stream, Encoding.UTF8, true)) message.Serialize(writer);
        return stream.ToArray();
    }

    /// <summary>What the listener's network library takes from an app-family message: the source
    /// index (two bytes, little-endian) and the application data after it.</summary>
    public static (int source, byte[] app) Listen(BaseScertMessage message)
    {
        var body = Body(message);
        Check(body.Length >= 2, "a listener message without a source index");
        return (body[0] | (body[1] << 8), body[2..]);
    }

    public static void Parsing()
    {
        // The lab's layout for one nonzero group: the 14-byte header is a 4-byte address block and the voice header.
        var squad = Uplink(36, 1, 37);
        Check(squad.Length == 51 && squad[0] == 0x81 && squad[1] == 36 && squad[4] == 0 && squad[5] == 0x2F && squad[6] == 36,
            "synthetic uplink matches the observed layout");
        Check(VoiceGroupSend.TryParse(squad, out var send) && send.Group == 36 && send.AppOffset == 4 && send.MediaType == 1 &&
            send.DataLength == 37 && send.Sequence == 1, $"squad send: {send}");
        Check(VoicePacketHeader.TryParse(squad, out var header) && header.Group == 36 && header.DataLength == 37,
            "the diagnostic header parser agrees on a nonzero group");

        // Group 0 (side 0 team broadcast) has no base group, so its address block is two bytes.
        var team0 = Uplink(0, 9, 37);
        Check(VoiceGroupSend.TryParse(team0, out send) && send.Group == 0 && send.AppOffset == 2 && send.Sequence == 9, $"team 0: {send}");
        Check(VoiceGroupSend.TryParse(Uplink(1, 2, 37), out send) && send.Group == 1 && send.AppOffset == 4, "team 1");
        Check(VoiceGroupSend.TryParse(Uplink(83, 300, 37), out send) && send.Group == 83 && send.Sequence == 300, "proximity group of client 0");
        // A proximity group above 255 travels in the two-byte base; the voice header keeps its low byte.
        Check(VoiceGroupSend.TryParse(Uplink(300, 1, 37), out send) && send.Group == 300, "group above 255");
        // A base and a mask that names a later bit: base 32, second mask byte, bit 3 = group 43.
        var offset = new byte[] { 0x82, 32, 0, 0x00, 0x08 }.Concat(Uplink(43, 5, 37)[4..]).ToArray();
        Check(VoiceGroupSend.TryParse(offset, out send) && send.Group == 43 && send.AppOffset == 5, $"mask bit offset: {send}");

        // Packed packets: six blocks (222 bytes) and the game's 248-byte limit pass; one byte more does not.
        Check(VoiceGroupSend.TryParse(Uplink(20, 3, 222), out send) && send.DataLength == 222, "six blocks");
        Check(VoiceGroupSend.TryParse(Uplink(20, 3, 248), out send) && send.DataLength == 248, "248 data bytes");
        Check(!VoiceGroupSend.TryParse(Uplink(20, 3, 249), out _), "249 data bytes accepted");
        Check(VoiceGroupSend.TryParse(Uplink(20, 3, 0), out send) && send.DataLength == 0, "an empty voice packet");

        var bad = new List<(string, byte[])>
        {
            ("empty", Array.Empty<byte>()),
            ("no mask", new byte[] { 0x80, 20, 0 }),
            ("truncated base", new byte[] { 0x81, 20 }),
            ("truncated mask", new byte[] { 0x82, 20, 0, 0x01 }),
            ("empty mask", new byte[] { 0x81, 20, 0, 0x00 }.Concat(Uplink(20, 1, 37)[4..]).ToArray()),
            ("two groups", new byte[] { 0x81, 20, 0, 0x03 }.Concat(Uplink(20, 1, 37)[4..]).ToArray()),
            ("header names another group", new byte[] { 0x81, 21, 0, 0x01 }.Concat(Uplink(20, 1, 37)[4..]).ToArray()),
            ("short length", Uplink(20, 1, 37)[..^1]),
            ("long length", Uplink(20, 1, 37).Concat(new byte[] { 0 }).ToArray()),
            ("no voice header", Uplink(20, 1, 37)[..12]),
        };
        var otherType = Uplink(20, 1, 37); otherType[5] = 0x30; bad.Add(("another message type", otherType));
        foreach (var (name, body) in bad) Check(!VoiceGroupSend.TryParse(body, out _), "accepted: " + name);
        Check(VoiceGroupSend.IsTransport(53) && VoiceGroupSend.IsTransport(54) && !VoiceGroupSend.IsTransport(3) &&
            !VoiceGroupSend.IsTransport(61), "group send transports");
    }

    public static void ListenerForms()
    {
        // The form switch is gone: a value left set (any of the former lab forms, or garbage) is
        // reported and ignored, never fatal, and never changes the options.
        foreach (var value in new[] { "app", "app53", "from_b2", "whole", "from_b6", "raw", " App" })
        {
            var warnings = new List<string>();
            var parsed = VoiceRelayOptions.FromEnvironment(name =>
                name == VoiceRelayOptions.Variable ? "1" : name == VoiceRelayOptions.RemovedFormVariable ? value : null, warnings.Add);
            Check(parsed.Enabled && parsed.Tier == VoiceRelayTier.All, $"form '{value}' changed the options");
            Check(warnings.Count == 1 && warnings[0].Contains(VoiceRelayOptions.RemovedFormVariable) && warnings[0].Contains("ignored"),
                $"form '{value}' was not reported: [{string.Join("; ", warnings)}]");
        }
        var quiet = new List<string>();
        VoiceRelayOptions.FromEnvironment(name => name == VoiceRelayOptions.RemovedFormVariable ? "" : null, quiet.Add);
        VoiceRelayOptions.FromEnvironment(_ => null, quiet.Add);
        Check(quiet.Count == 0, "an unset or empty form switch was reported");
        Check(VoiceRelayOptions.FromEnvironment(name => name == VoiceRelayOptions.RemovedFormVariable ? "whole" : null) != null,
            "a removed form value without a warning sink");

        foreach (var uplink in new[] { Uplink(36, 7, 37), Uplink(0, 8, 74), Uplink(300, 9, 222) })
        {
            Check(VoiceGroupSend.TryParse(uplink, out var send), "synthetic uplink parses");
            foreach (int talker in new[] { 0, 5, 200, 511 })
            {
                // A client app single from the talker, carrying the library message unchanged.
                var app = VoiceListenerMessage.Build(talker, uplink, send);
                Check(app is RT_MSG_CLIENT_APP_SINGLE single && single.TargetOrSource == talker && app.Id == RT_MSG_TYPE.RT_MSG_CLIENT_APP_SINGLE,
                    "the listener form is a client app single");
                var (source, data) = Listen(app);
                Check(source == talker && data.SequenceEqual(uplink[send.AppOffset..]) && data[0] == 0 && data[1] == 0x2F &&
                    data[2] == (byte)send.Group && data.Length == VoiceGroupSend.VoiceHeaderLength + send.DataLength,
                    $"listener form: source {source}, {data.Length} bytes");
            }
        }
        // The uplink buffer is never shared with or changed by the listener message.
        var original = Uplink(20, 1, 37);
        VoiceGroupSend.TryParse(original, out var once);
        var copy = (byte[])original.Clone();
        var built = (RT_MSG_CLIENT_APP_SINGLE)VoiceListenerMessage.Build(1, original, once);
        built.Payload[0] = 0xFF;
        Check(original.SequenceEqual(copy), "the listener payload aliases the uplink");
    }

    static VoiceGroupsMember Entry(int player, int side, int[] listen, int[] transmit) =>
        new VoiceGroupsMember($"synthetic-voice-reservation-{player:00}", (ushort)player, (byte)side, listen, transmit);

    // Side 0, platoon 0: members 1 and 2 of squad 0 (2 its leader), a member of squad 1, the platoon
    // leader in squad 1 and the OIC in squad 4. Side 1: a member of squad 16 and an entry that names a
    // side-0 squad group.
    static VoiceRelayTable World() => new VoiceRelayTable(new VoiceGroupsFrame(5, new[]
    {
        Entry(1, 0, new[] { 0, 12, 20 }, new[] { 20 }),
        Entry(2, 0, new[] { 0, 4, 12, 20 }, new[] { 4, 20 }),
        Entry(3, 0, new[] { 0, 12, 21 }, new[] { 21 }),
        Entry(4, 0, new[] { 0, 2, 4, 12, 21 }, new[] { 2, 4, 12, 21 }),
        Entry(5, 0, new[] { 0, 2, 13, 24 }, new[] { 0, 2, 5, 24 }),
        Entry(6, 1, new[] { 1, 16, 36 }, new[] { 36 }),
        Entry(7, 1, new[] { 1, 20 }, new[] { 20 }),
    }));

    /// <summary>The relay pipeline without a host: parse, decide, build, and read as each listener would.</summary>
    static Dictionary<int, (int source, byte[] app)> Relay(VoiceRelayTable table, VoiceRelayOptions options, int talker, byte[] uplink,
        VoiceTalkerBudget budget, out VoiceRelayDecision decision, bool muted = false, long now = 0)
    {
        var delivered = new Dictionary<int, (int, byte[])>();
        decision = null;
        if (!VoiceGroupSend.TryParse(uplink, out var send)) return delivered;
        decision = VoiceRelay.Decide(options, table, talker, send.Group, send.DataLength, true, muted, _ => true, budget, now);
        if (!decision.Relayed) return delivered;
        var message = VoiceListenerMessage.Build(talker, uplink, send);
        foreach (int recipient in decision.Recipients) delivered.Add(recipient, Listen(message));
        return delivered;
    }

    public static void Routing()
    {
        var all = new VoiceRelayOptions(true, VoiceRelayTier.All);
        var squadOnly = new VoiceRelayOptions(true, VoiceRelayTier.Squad);
        var table = World();
        void Expect(VoiceRelayOptions options, int talker, int group, int[] listeners, string what, int blocks = 1)
        {
            var uplink = Uplink(group, 11, 37 * blocks);
            var heard = Relay(table, options, talker, uplink, new VoiceTalkerBudget(), out var decision);
            Check(decision != null && decision.Relayed && heard.Keys.OrderBy(k => k).SequenceEqual(listeners),
                $"{what}: {VoiceRelayDecision.Name(decision?.Drop ?? VoiceDropReason.Malformed)} [{string.Join(",", heard.Keys)}]");
            Check(!heard.ContainsKey(talker), $"{what}: the talker heard itself");
            int appOffset = group == 0 ? 2 : 4;
            foreach (var (listener, (source, app)) in heard)
                Check(source == talker && app.SequenceEqual(uplink[appOffset..]), $"{what}: listener {listener} got source {source}");
        }
        void Drop(VoiceRelayOptions options, int talker, int group, VoiceDropReason reason, string what, bool muted = false)
        {
            var heard = Relay(table, options, talker, Uplink(group, 11, 37), new VoiceTalkerBudget(), out var decision, muted);
            Check(decision != null && decision.Drop == reason && heard.Count == 0, $"{what}: {VoiceRelayDecision.Name(decision?.Drop ?? VoiceDropReason.None)}");
        }

        // Squad chat: the squad, never the talker, never the enemy entry that names the same group.
        Expect(squadOnly, 1, 20, new[] { 2 }, "squad chat");
        Expect(squadOnly, 2, 20, new[] { 1 }, "squad chat from the squad leader");
        Expect(squadOnly, 1, 20, new[] { 2 }, "a packed six-block packet", blocks: 6);
        Expect(squadOnly, 7, 20, Array.Empty<int>(), "the other side's entry reaches nobody on side 0");
        // Platoon chat: squad leaders and the platoon leader. Company chat: platoon leaders and the OIC.
        Expect(all, 2, 4, new[] { 4 }, "platoon chat from a squad leader");
        Expect(all, 4, 4, new[] { 2 }, "platoon chat from the platoon leader");
        Expect(all, 4, 2, new[] { 5 }, "company chat from the platoon leader");
        Expect(all, 5, 2, new[] { 4 }, "company chat from the OIC");
        Expect(all, 4, 12, new[] { 1, 2, 3 }, "platoon broadcast");
        Expect(all, 5, 0, new[] { 1, 2, 3, 4 }, "team broadcast on group 0, side 0 only");
        // The squad tier keeps platoon and company chat off; members cannot reach leader channels.
        Drop(squadOnly, 2, 4, VoiceDropReason.NotAllowedGroup, "platoon chat in the squad tier");
        Drop(all, 1, 4, VoiceDropReason.NotAllowedGroup, "a member's platoon chat");
        Drop(all, 1, 21, VoiceDropReason.NotAllowedGroup, "another squad");
        // Proximity copies are dropped while gameplay lists none; another client's group is refused.
        Drop(all, 1, 84, VoiceDropReason.ProximityOff, "the talker's own proximity copy");
        Drop(all, 1, 83, VoiceDropReason.NotAllowedGroup, "another client's proximity group");
        // A server-side mute drops both copies of every frame.
        Drop(all, 1, 20, VoiceDropReason.Muted, "a muted talker's channel copy", muted: true);
        Drop(all, 1, 84, VoiceDropReason.Muted, "a muted talker's proximity copy", muted: true);

        // The budget counts data bytes: packed catch-up passes, a flood does not, and drops reach nobody.
        var budget = new VoiceTalkerBudget();
        int passed = 0, dropped = 0;
        for (int i = 0; i < 20; i++)
        {
            var heard = Relay(table, squadOnly, 1, Uplink(20, i, 222), budget, out var decision);
            if (decision.Relayed) { passed++; Check(heard.Count == 1, "a relayed packet reached its squad"); }
            else { dropped++; Check(decision.Drop == VoiceDropReason.OverBudget && heard.Count == 0, "an over-budget packet was delivered"); }
        }
        Check(passed == VoiceRelayOptions.BudgetBytesPerSecond / 222 && dropped == 20 - passed, $"budget: {passed} passed, {dropped} dropped");
    }
}

internal static partial class LeadershipProgressionChecks
{
    private sealed class PlainCommitter : ICharacterStoreCommitter
    {
        public void WriteAtomically(string path, byte[] contents) => File.WriteAllBytes(path, contents);
    }

    /// <summary>The host path: bound clients, a voice_groups table and group sends through
    /// LocalDmeHost.Voice, delivered to listeners' queues in the listener form.</summary>
    public static void VoiceRelayHost() => WithDirectory(directory =>
    {
        // Durability is not under test here, so a plain write keeps this check portable.
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new PlainCommitter()) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        foreach (var (account, name) in new[] { ("alpha", "Alpha"), ("bravo", "Bravo"), ("charlie", "Charlie"), ("delta", "Delta") })
            Create(store, account, name);
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var settings = Server.Medius.Program.Settings;
            int previousApp = settings.MagApplicationId;
            settings.MagApplicationId = 20231;
            var previousOptions = VoiceRelayOptions.Current;
            VoiceRelayOptions.Current = new VoiceRelayOptions(true, VoiceRelayTier.All);
            var previousPlugins = Server.Dme.Program.Plugins;
            Server.Dme.Program.Plugins = new Server.Plugins.PluginsManager(directory);
            try
            {
                await using var bridge = new FixtureBridge(new[] { VoiceRelayOptions.Capability });
                await using var host = new LocalDmeHost(bridge.Client, store);
                await MakeReady(host, bridge, 20231);
                async Task<(Server.Dme.Models.ClientObject client, string reservation)> Admit(string account)
                {
                    var session = MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id);
                    Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, $"synthetic voice admission of {account}");
                    var reserve = bridge.Latest("reserve");
                    return (ConnectFixture(host, reserve), reserve["reservation_id"]!.GetValue<string>());
                }
                var alpha = await Admit("alpha");
                var bravo = await Admit("bravo");
                var charlie = await Admit("charlie");
                var delta = await Admit("delta");
                var everyone = new[] { alpha.client, bravo.client, charlie.client, delta.client };

                // A frame received before the table relays nothing.
                host.Voice(alpha.client, "udp", 53, VoiceForwardingChecks.Uplink(20, 1, 37));
                Check(host.VoiceCounters.Contains("unbound=1"), "no table yet: " + host.VoiceCounters);

                // Alpha leads squad 0 (with bravo); delta is the platoon leader in squad 1; charlie is on the
                // other side and its entry names alpha's squad group.
                string Member((Server.Dme.Models.ClientObject client, string reservation) who, int side, string listen, string transmit) =>
                    $"{{\"reservation_id\":\"{who.reservation}\",\"player_id\":{who.client.DmeId},\"side\":{side},\"listen\":[{listen}],\"transmit\":[{transmit}]}}";
                var frame = "{\"v\":2,\"request_id\":\"synthetic\",\"ok\":true,\"error\":null,\"messages\":[],\"voice_groups\":{\"revision\":2,\"members\":[" +
                    string.Join(",", Member(alpha, 0, "0,4,12,20", "4,20"), Member(bravo, 0, "0,12,20", "20"),
                        Member(charlie, 1, "1,20", "20"), Member(delta, 0, "0,2,4,12,21", "2,4,12,21")) + "]}}";
                Check(GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(frame), "synthetic", true, out var reply) && reply.VoiceGroups != null,
                    "synthetic voice_groups frame");
                var gate = typeof(LocalDmeHost).GetField("_gate", HostInternals)!.GetValue(host)!;
                lock (gate) typeof(LocalDmeHost).GetMethod("ApplyVoiceGroups", HostInternals)!.Invoke(host, new object[] { reply });
                Check(host.VoiceTable?.Revision == 2, "the host keeps the voice table");

                List<BaseScertMessage> Drain(Server.Dme.Models.ClientObject client)
                {
                    var messages = new List<BaseScertMessage>();
                    while (client.TcpSendMessageQueue.TryDequeue(out var message)) messages.Add(message);
                    return messages;
                }
                foreach (var client in everyone) Drain(client);
                void Hears(Server.Dme.Models.ClientObject talker, byte[] uplink, string what, params Server.Dme.Models.ClientObject[] listeners)
                {
                    host.Voice(talker, "udp", 53, uplink);
                    bool parsed = VoiceGroupSend.TryParse(uplink, out var send);
                    Check(parsed || listeners.Length == 0, $"{what}: synthetic uplink");
                    foreach (var client in everyone)
                    {
                        var got = Drain(client).OfType<RT_MSG_CLIENT_APP_SINGLE>().ToList();
                        bool expected = listeners.Contains(client);
                        Check(got.Count == (expected ? 1 : 0), $"{what}: client {client.DmeId} got {got.Count} (expected {(expected ? 1 : 0)})");
                        if (expected)
                            Check(got[0].TargetOrSource == talker.DmeId && got[0].Payload.SequenceEqual(uplink[send.AppOffset..]),
                                $"{what}: client {client.DmeId} got source {got[0].TargetOrSource}");
                    }
                }

                Hears(alpha.client, VoiceForwardingChecks.Uplink(20, 2, 37), "squad chat reaches the squad, not the talker or the enemy", bravo.client);
                Hears(bravo.client, VoiceForwardingChecks.Uplink(20, 3, 222), "a six-block packet", alpha.client);
                Hears(alpha.client, VoiceForwardingChecks.Uplink(4, 4, 37), "platoon chat reaches the platoon leader", delta.client);
                Hears(delta.client, VoiceForwardingChecks.Uplink(12, 5, 37), "platoon broadcast reaches the platoon", alpha.client, bravo.client);
                Hears(bravo.client, VoiceForwardingChecks.Uplink(4, 6, 37), "a member cannot use platoon chat");
                Hears(alpha.client, VoiceForwardingChecks.Uplink(83 + alpha.client.DmeId, 7, 37), "the proximity copy is dropped");
                Hears(charlie.client, VoiceForwardingChecks.Uplink(20, 8, 37), "the other side cannot reach a side-0 squad");
                Hears(alpha.client, new byte[] { 0x81, 20 }, "a malformed group send");
                string counters = host.VoiceCounters;
                foreach (var expected in new[] { "unbound=1", "not_allowed_group=1", "proximity_off=1", "malformed=1" })
                    Check(counters.Contains(expected), $"counters missing {expected}: {counters}");

                // Account sanctions: the relay reads the cache's snapshot, never the store, so a change
                // written to the store takes effect at the next refresh. A mute or a ban drops both copies of
                // every frame as muted; an expired sanction stops dropping at once, without a refresh.
                var previousCache = VoiceSanctionCache.Current;
                var clock = new DateTimeOffset(2026, 11, 27, 12, 0, 0, TimeSpan.Zero);
                store.Clock = () => clock;
                var cache = new VoiceSanctionCache(store, () => clock);
                VoiceSanctionCache.Current = cache;
                try
                {
                    string backups = Path.Combine(directory, "voice-backups");
                    Check(cache.Refresh() && cache.Count == 0, "an empty sanction snapshot");
                    Check(store.ChangeVoiceSanction("alpha", VoiceSanctionKind.Mute, false, "synthetic", "staff", TimeSpan.FromMinutes(10), true, backups).Applied,
                        "synthetic mute");
                    Hears(alpha.client, VoiceForwardingChecks.Uplink(20, 9, 37), "a mute waits for the next refresh", bravo.client);
                    Check(cache.Refresh() && cache.Count == 1, "the refresh loads the mute");
                    long mutedBefore = Muted();
                    Hears(alpha.client, VoiceForwardingChecks.Uplink(20, 10, 37), "a muted talker's channel copy");
                    Hears(alpha.client, VoiceForwardingChecks.Uplink(83 + alpha.client.DmeId, 11, 37), "a muted talker's proximity copy");
                    Hears(alpha.client, VoiceForwardingChecks.Uplink(4, 12, 37), "a muted talker's platoon chat");
                    Check(Muted() == mutedBefore + 3, "a mute counts every dropped copy as muted: " + host.VoiceCounters);
                    Hears(bravo.client, VoiceForwardingChecks.Uplink(20, 13, 37), "a muted listener still hears", alpha.client);
                    clock = clock.AddMinutes(10);
                    Hears(alpha.client, VoiceForwardingChecks.Uplink(20, 14, 37), "an expired mute lifts without a refresh", bravo.client);
                    Check(store.ChangeVoiceSanction("alpha", VoiceSanctionKind.Ban, false, null, "staff", null, true, backups).Applied, "synthetic ban");
                    cache.Refresh();
                    Hears(alpha.client, VoiceForwardingChecks.Uplink(20, 15, 37), "a banned talker");
                    Check(store.ChangeVoiceSanction("alpha", VoiceSanctionKind.Ban, true, "appeal", "staff", null, true, backups).Applied, "synthetic unban");
                    cache.Refresh();
                    Hears(alpha.client, VoiceForwardingChecks.Uplink(20, 16, 37), "an unbanned talker", bravo.client);
                }
                finally { VoiceSanctionCache.Current = previousCache; }
                long Muted()
                {
                    var match = System.Text.RegularExpressions.Regex.Match(host.VoiceCounters, @"\bmuted=(\d+)");
                    return match.Success ? long.Parse(match.Groups[1].Value) : -1;
                }

                // The talker's budget: a burst of packed packets passes up to one second at twice the nominal rate.
                int heard = 0;
                for (int i = 0; i < 20; i++)
                {
                    host.Voice(delta.client, "udp", 53, VoiceForwardingChecks.Uplink(12, 100 + i, 222));
                    heard += Drain(alpha.client).Count;
                }
                Check(heard >= VoiceRelayOptions.BudgetBytesPerSecond / 222 - 1 && heard < 20 && host.VoiceCounters.Contains("over_budget="),
                    $"budget: {heard} of 20 packed packets relayed; {host.VoiceCounters}");
                Check(!host.VoiceCounters.Contains("over_budget=0"), "no packet went over budget: " + host.VoiceCounters);
                Check(bridge.Requests.All(request => request["op"]!.GetValue<string>() != "event"), "voice reached the gameplay event bridge");
            }
            finally
            {
                VoiceRelayOptions.Current = previousOptions;
                Server.Dme.Program.Plugins = previousPlugins;
                settings.MagApplicationId = previousApp;
            }
        }
    });
}
