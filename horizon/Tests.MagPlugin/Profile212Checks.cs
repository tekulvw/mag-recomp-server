using RT.Models.MAG;
using RT.Models;
using Server.Medius;
using System.Text;
using Server.Common.Stream;

internal static class Profile212Checks
{
    private static void Check(bool condition, string message)
    { if (!condition) throw new Exception(message); }
    private static void Throws(Action action)
    { try { action(); } catch (ArgumentException) { return; } throw new Exception("Expected profile rejection"); }
    private static byte[] Header(ushort type, string body = "") =>
        new byte[] { (byte)(type >> 8), (byte)type }.Concat(new byte[68]).Concat(Convert.FromHexString(body)).ToArray();

    public static void WireIds()
    {
        // Literal IDs are independently corroborated from exact 2.12 constructor calls.
        var rows = new (MagMessage Message, ushort Id)[] {
            (new MagHello(), 0x0001), (new MagProtocolInfo {ProtocolVersion = 1793}, 0x0002),
            (new MagKeepAlive(), 0x0003), (new MagTrainingCompletedRequest(), 0x002d), (new MagRetailNpAuthenticateRequest(), 0x0036),
            (new MagRetailNpAuthenticateResponse(), 0x0037), (new MagRetailMapsHelloResponse(), 0x0038),
            (new MagRetailUniverseListRequest(), 0x0039), (new MagRetailUniverseListResponse(), 0x003a),
            (new MagRetailAccountLoginRequest(), 0x0030), (new MagRetailAccountLoginResponse(), 0x0031),
            (new MagRetailAccountLogoutRequest(), 0x0032), (new MagRetailAccountLogoutResponse(), 0x0033),
            (new MagNewsEulaRequest(), 0x0252), (new MagNewsEulaResponse(), 0x0253),
            (new MagCharacterListRequest(), 0x024a), (new MagRetailCharacterListResponse(), 0x024b),
            (new MagCharacterDataRequest {QueryKind = 2}, 0x024c), (new MagCharacterDataResponse(), 0x024d),
            (new MagCreateCharacterRequest(), 0x024e), (new MagCreateCharacterResponse(), 0x024f),
            (new MagDeleteCharacterRequest(), 0x0250), (new MagDeleteCharacterResponse(), 0x0251),
            (new MagCharacterRecordNotification(), 0x0267)
        };
        foreach (var row in rows)
        {
            var wire = MagMessageCodec.SerializeForProtocol(row.Message, 20231, 1793);
            Check(wire[0] == row.Id >> 8 && wire[1] == (byte)row.Id, "Wrong 1793 ID for " + row.Message.GetType().Name);
            Check(MagMessageCodec.TryParseForProtocol(wire, 20231, 1793, out var parsed) &&
                parsed.GetType() == row.Message.GetType(), "1793 body did not parse for " + row.Message.GetType().Name);
        }
        Check(!MagMessageCodec.TryParseClientForProtocol(Header(0x0037, "80"), 20231, 1793, out _), "Server NP reply entered client handler");
        Check(!MagMessageCodec.TryParseClientForProtocol(Header(0x0236), 20231, 1793, out _), "Legacy list ID accepted by 1793");
        Check(!MagMessageCodec.TryParseClientForProtocol(Header(0x0012), 20231, 1793, out _), "Unimplemented join used legacy fallback");
        Check(!MagMessageCodec.TryParseClientForProtocol(Header(0x024a), 20230, 1793, out _), "1793 used prototype app");
        Check(!MagMessageCodec.TryParseClientForProtocol(Header(0x024a), 20231, 1794, out _), "Unknown profile guessed");
        Throws(() => MagMessageCodec.SerializeForProtocol(new MagRetailJoinGameRequest(), 20231, 1793));
        Throws(() => MagMessageCodec.SerializeForProtocol(new MagHello(), 20230, 1793));
        Throws(() => MagMessageCodec.SerializeForProtocol(new MagNpAuthenticateRequest(), 20231, 1793));
        Throws(() => MagMessageCodec.SerializeForProtocol(new MagUniverseListRequest(), 20231, 1793));
        Throws(() => MagMessageCodec.SerializeForProtocol(new MagUniverseListResponse(), 20231, 1793));
        Throws(() => MagMessageCodec.SerializeForProtocol(new MagRetailPatchRequest(), 20231, 1793));
        // The overlapping 0030 must mean login under1793 and logout under1740.
        Check(MagMessageCodec.TryParseClientForProtocol(Header(0x0030), 20231, 1740, out var old) && old is MagRetailAccountLogoutRequest,
            "1740 overlap changed");
        Check(!MagMessageCodec.TryParseClientForProtocol(Header(0x0030), 20231, 1793, out _), "1793 misread old logout as complete login");
    }

    public static void CharacterLayouts()
    {
        var slot = new MagRetailCharacterSummary { Faction = 255, SkillSpecialty = 0, Level = 0,
            Loadout = new MagLoadout {UpdateId = 0}, Patch212SlotLocked = true };
        var list = new MagRetailCharacterListResponse {Success = true, CharacterCount = 1, AllowedCharacters = 3, Characters = new() {slot}};
        // Success bit + count byte, then empty summary and the two final slot bits.
        byte[] expected = Header(0x024b, "8080000000007f800000000000000000000000000080");
        var wire = MagMessageCodec.SerializeForProtocol(list, 20231, 1793);
        Check(wire.SequenceEqual(expected), "1793 list has a legacy allowed-count byte or wrong slot bit alignment");
        Check(MagMessageCodec.TryParseForProtocol(wire, 20231, 1793, out var parsed) &&
            parsed is MagRetailCharacterListResponse c && c.CharacterCount == 1 &&
            ((MagRetailCharacterSummary)c.Characters[0]).Patch212SlotLocked, "1793 slot flags lost");
        Check(!MagMessageCodec.TryParseForProtocol(wire[..^1], 20231, 1793, out _), "Truncated summary accepted");
        byte[] bad = (byte[])wire.Clone(); bad[^1] |= 1;
        Check(!MagMessageCodec.TryParseForProtocol(bad, 20231, 1793, out _), "Nonzero slot padding accepted");

        var create = new MagCreateCharacterRequest {Patch212UnknownB0 = true};
        var createWire = MagMessageCodec.SerializeForProtocol(create, 20231, 1793);
        Check(createWire.Length == 115 && createWire[^1] == 0x80, "Create flag is not one final MSB bit");
        Check(MagMessageCodec.TryParseClientForProtocol(createWire, 20231, 1793, out var created) &&
            created is MagCreateCharacterRequest {Patch212UnknownB0: true}, "Unknown create flag not preserved");
        Check(!MagMessageCodec.TryParseClientForProtocol(createWire[..^1], 20231, 1793, out _), "Missing create flag accepted");
        createWire[^1] |= 1;
        Check(!MagMessageCodec.TryParseClientForProtocol(createWire, 20231, 1793, out _), "Create nonzero padding accepted");

        var record = new MagCharacterRecord {Faction = 0, SkillSpecialty = 0, Level = 0, LeadershipLevel = 0, LastZonesMux = 0,
            Loadouts = Enumerable.Range(0, 5).Select(_ => new MagLoadout {UpdateId = 0}).ToArray(),
            Patch212Unknown16 = new ushort[] {0x1234,0,0x5678,0,0,0xabcd},
            Patch212Unknown32 = new uint[] {0x89abcdef,0,1,2,uint.MaxValue}};
        byte[] recordExpected = new byte[] {0x02,0x67}.Concat(new byte[214]).Concat(Convert.FromHexString(
            "24680000acf000000001579b13579bde000000000000000200000005fffffffe")).ToArray();
        var recordWire = MagMessageCodec.SerializeForProtocol(new MagCharacterRecordNotification {Record = record}, 20231, 1793);
        Check(recordWire.SequenceEqual(recordExpected), "Record extension widths or bit alignment changed");
        Check(MagMessageCodec.TryParseForProtocol(recordWire, 20231, 1793, out var notice) &&
            notice is MagCharacterRecordNotification n && n.Record.Patch212Unknown16.SequenceEqual(record.Patch212Unknown16) &&
            n.Record.Patch212Unknown32.SequenceEqual(record.Patch212Unknown32), "Unknown record values lost");
        Check(!MagMessageCodec.TryParseForProtocol(recordWire[..^1], 20231, 1793, out _), "Truncated extension accepted");
        record.Patch212Unknown16 = new ushort[5];
        Throws(() => MagMessageCodec.SerializeForProtocol(new MagCharacterRecordNotification {Record = record}, 20231, 1793));
    }
    private static RT_MSG_CLIENT_APP_TO_PLUGIN RawEnvelope(byte[] payload, byte pluginId = 0)
    {
        using var stream = new MemoryStream();
        using (var writer = new BinaryWriter(stream, Encoding.UTF8, leaveOpen: true))
        { writer.Write((ushort)payload.Length); writer.Write(pluginId); writer.Write(payload); }
        stream.Position = 0;
        using var reader = new MessageReader(stream);
        var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN(); envelope.Deserialize(reader);
        Check(envelope.IsValid, "Synthetic RT envelope rejected");
        return envelope;
    }

    public static void PortalTickets()
    {
        // Reconstruct the portal's existing public synthetic vector, not a real ticket/key.
        using var vectors = System.Text.Json.JsonDocument.Parse(File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "portal-ticket-vectors.json")));
        var vector = vectors.RootElement.GetProperty("valid")[0];
        var key = vectors.RootElement.GetProperty("key");
        string audience = vector.GetProperty("audience").GetString();
        var settings = MagPortalTicketSettings.Load(name => name switch {
            MagPortalTicketSettings.ModeVariable => "required",
            MagPortalTicketSettings.KeysVariable => key.GetProperty("key_id").GetByte() + ":" + key.GetProperty("spki_base64").GetString(),
            MagPortalTicketSettings.AudienceVariable => audience, _ => null});
        var bytes = new List<byte>(Encoding.ASCII.GetBytes("MAGT")) {1, vector.GetProperty("key_id").GetByte()};
        void Big(ulong value, int width)
        { for (int shift = width * 8 - 8; shift >= 0; shift -= 8) bytes.Add((byte)(value >> shift)); }
        Big(vector.GetProperty("issued_at").GetUInt64(), 8); Big(vector.GetProperty("expires_at").GetUInt64(), 8);
        Big(vector.GetProperty("account_id").GetUInt32(), 4);
        bytes.AddRange(Convert.FromHexString(vector.GetProperty("nonce_hex").GetString()));
        foreach (string value in new[] {audience, vector.GetProperty("player_name").GetString()})
        { bytes.Add((byte)value.Length); bytes.AddRange(Encoding.ASCII.GetBytes(value)); }
        bytes.AddRange(Convert.FromHexString(vector.GetProperty("signature_hex").GetString()));
        var now = DateTimeOffset.FromUnixTimeSeconds(vector.GetProperty("verify_at").GetInt64()).UtcDateTime;
        int oldApp = Server.Medius.Program.Settings.MagApplicationId;
        uint oldProtocol = Server.Medius.Program.Settings.MagProtocolVersion;
        try
        {
            Server.Medius.Program.Settings.MagApplicationId = 20231;
            Server.Medius.Program.Settings.MagProtocolVersion = 1793;
            foreach (bool mapsService in new[] {true, false})
            {
                var policy = new MagPortalTicketPolicy(settings, () => now);
                var maps = new MagMapsSession {PortalTickets = policy};
                var mas = new MagMasSession {PortalTickets = policy};
                IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Send(MagMessage message) {
                    var envelope = RawEnvelope(MagMessageCodec.SerializeForProtocol(message, 20231, 1793));
                    return mapsService ? maps.HandleMany(true, 20231, envelope, now) : mas.HandleMany(true, 20231, envelope, now);
                }
                Send(new MagHello());
                if (!mapsService) Send(new MagProtocolInfo {ProtocolVersion = 1793});
                var plain = new MagRetailNpAuthenticateRequest {FullTicketSize = 12, TicketChunkSize = 12, LastMessage = true};
                Encoding.ASCII.GetBytes("synthetic212").CopyTo(plain.TicketChunk, 0);
                var refused = Send(plain).Single();
                Check(refused.Payload[1] == 0x37 && MagMessageCodec.TryParseForProtocol(refused.Payload, 20231, 1793, out var failure) &&
                    failure is MagRetailNpAuthenticateResponse {Success: false}, "Required1793 mode accepted a plain development name");
                var request = new MagRetailNpAuthenticateRequest {FullTicketSize = (ushort)bytes.Count,
                    TicketChunkSize = (ushort)bytes.Count, LastMessage = true, TransactionId = 23};
                bytes.CopyTo(request.TicketChunk);
                var accepted = Send(request).Single();
                Check(accepted.Payload[1] == 0x37 && MagMessageCodec.TryParseForProtocol(accepted.Payload, 20231, 1793, out var success) &&
                    success is MagRetailNpAuthenticateResponse {Success: true, TransactionId: 23}, "1793 signed ticket failed");
                Check((mapsService ? maps.PortalAccountId : mas.PortalAccountId) == vector.GetProperty("account_id").GetUInt32(),
                    "1793 ticket lost portal account identity");
                if (mapsService) maps.EndPortalSession(); else mas.EndPortalSession();
            }
        }
        finally {Server.Medius.Program.Settings.MagApplicationId = oldApp; Server.Medius.Program.Settings.MagProtocolVersion = oldProtocol;}
    }

    private sealed class SyntheticCommitter : ICharacterStoreCommitter
    { public void WriteAtomically(string path, byte[] contents) => File.WriteAllBytes(path, contents); }

    public static void Sessions()
    {
        var settings = Server.Medius.Program.Settings;
        int oldApp = settings.MagApplicationId; uint oldProtocol = settings.MagProtocolVersion;
        string directory = Path.Combine(Path.GetTempPath(), "mag-profile212-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            settings.MagApplicationId = 20231; settings.MagProtocolVersion = 1793;
            var now = DateTime.UtcNow;
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new SyntheticCommitter());
            var maps = new MagMapsSession(MagServerNews.FromChangelogs(null, null)) {DevelopmentAuthEnabled = true, CharacterStorageEnabled = true, CharacterStore = store};
            RT_MSG_CLIENT_APP_TO_PLUGIN Envelope(MagMessage m) => RawEnvelope(MagMessageCodec.SerializeForProtocol(m, 20231, 1793));
            IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Maps(MagMessage m) => maps.HandleMany(true, 20231, Envelope(m), now);
            MagMessage Read(RT_MSG_SERVER_PLUGIN_TO_APP reply)
            { Check(MagMessageCodec.TryParseForProtocol(reply.Payload, 20231, 1793, out var value), "Malformed 1793 session reply"); return value; }
            var auth = new MagRetailNpAuthenticateRequest {FullTicketSize = 12, TicketChunkSize = 12, LastMessage = true};
            Encoding.ASCII.GetBytes("synthetic212").CopyTo(auth.TicketChunk, 0);
            Check(Maps(new MagHello()).Single().Payload[1] == 0x38, "MAPS sent the old hello ID");
            Check(Read(Maps(auth).Single()) is MagRetailNpAuthenticateResponse {Success: true}, "MAPS auth failed");
            Check(Read(Maps(new MagRetailUniverseListRequest()).Single()) is MagRetailUniverseListResponse {Success: true}, "Universe list failed");

            byte[] Patch(params uint[] levels)
            {
                var bytes = new List<byte> {0x01,0xf5,0,(byte)levels.Length};
                foreach (uint level in levels) bytes.AddRange(new byte[] {(byte)(level >> 24),(byte)(level >> 16),(byte)(level >> 8),(byte)level});
                foreach (string value in new[] {"ABmagprod493603", "english"})
                { bytes.Add(0); bytes.Add((byte)value.Length); bytes.AddRange(Encoding.ASCII.GetBytes(value)); }
                return bytes.ToArray();
            }
            IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> PatchSend(byte[] payload) => maps.HandleMany(true, 20231,
                RawEnvelope(payload), now);
            Check(PatchSend(Patch(445230,465304,467225)).Count == 0, "Incomplete install got a completion reply");
            Check(PatchSend(Patch(445230,465304,467225,489979,489979)).Count == 0, "Duplicate levels accepted");
            var patches = PatchSend(Patch(445230,465304,467225,489979));
            Check(patches.Count == 2 && patches[0].Payload.SequenceEqual(MagPatch212Codec.NoDownloadReply()[0]) &&
                patches[1].Payload.SequenceEqual(Convert.FromHexString("01f70000000000000080")), "Complete installed path lacks split terminal reply");
            Check(Maps(new MagNewsEulaRequest {Language = "english"}).Count == 2, "1793 news/EULA missing");
            MagRetailCharacterListResponse Listed() => (MagRetailCharacterListResponse)Read(Maps(new MagCharacterListRequest()).Single());
            var initial = Listed();
            Check(initial.CharacterCount == 3 && initial.Characters.All(c => c.AccountId == 0 &&
                c is MagRetailCharacterSummary {Patch212SlotLocked: false}), "Empty 1793 account has no three unlocked slots");

            var registry = new MagLoginRegistry();
            var mls = new MagMlsSession {CharacterStore = store};
            var mas = new MagMasSession {DevelopmentAuthEnabled = true, CharacterStorageEnabled = true,
                CharacterDataEnabled = true, AccountLoginEnabled = true, CharacterStore = store, LoginRegistry = registry};
            IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Mas(MagMessage m) => mas.HandleMany(true, 20231, Envelope(m), now);
            Check(Read(Mas(new MagHello()).Single()) is MagProtocolInfo {ProtocolVersion: 1793}, "MAS selected wrong profile");
            Mas(new MagProtocolInfo {ProtocolVersion = 1793});
            Check(Read(Mas(auth).Single()) is MagRetailNpAuthenticateResponse {Success: true}, "MAS auth failed");
            // The already-open sessions retain their selected codec through a config reload.
            settings.MagProtocolVersion = 1740;
            var create = new MagCreateCharacterRequest {AccountName = "synthetic212!1", Faction = 1, UniverseId = 26, Patch212UnknownB0 = false};
            Check(Read(Mas(create).Single()) is MagCreateCharacterResponse {Result: MagNetResult.Success}, "Character create failed: " + mas.StorageStatus);
            var character = store.ReadForAccount("synthetic212").Single();
            var listed = Listed();
            Check(listed.CharacterCount == 3 && listed.Characters[0].AccountId == 0 &&
                listed.Characters[1].AccountId == character.Id && listed.Characters[2].AccountId == 0, "Character slot position moved");
            var login = Read(Mas(new MagRetailAccountLoginRequest {VersionMajor = 3, VersionMinor = 3,
                VersionBuild = 1, AccountName = create.AccountName}).Single()) as MagRetailAccountLoginResponse;
            Check(login is {Result: 1} && login.AccountId == character.Id && mas.State == MagMasSession.SessionState.AccountLoginSent,
                "1793 account login failed: " + mas.StorageStatus);
            Check(Read(Mas(new MagCharacterDataRequest {QueryKind = 2, AccountId = character.Id, RequestId = 1}).Single()) is
                MagCharacterDataResponse {Success: true} card && card.Record.Name == create.AccountName &&
                card.Record.Patch212Unknown16.Length == 6 && card.Record.Patch212Unknown32.Length == 5, "1793 character record unavailable");
            Check(store.ReadForAccount("other-account").Count == 0, "Synthetic session touched another account");

            string Key(byte[] bytes) => Encoding.ASCII.GetString(bytes).TrimEnd('\0');
            Check(mls.TryBind(new RT_MSG_CLIENT_CONNECT_TCP {AppId = 20231, TargetWorldId = 1,
                AccessToken = Key(login.AccessKey), SessionKey = Key(login.SessionKey)}, registry, "synthetic-212-channel", now) &&
                mls.Accept(now) && mls.CompleteTransport(now), "1793 MLS transport bind failed");
            var protocolReply = mls.HandleMany(Envelope(new MagHello()), now).Single();
            Check(protocolReply.PluginId == 1 && protocolReply.Payload.SequenceEqual(Convert.FromHexString("00020000070100000000")),
                "MLS initial protocol reply did not retain1793");
            mls.HandleMany(Envelope(new MagProtocolInfo {ProtocolVersion = 1793}), now);
            Check(mls.State == MagMlsSession.SessionState.Ready, "MLS protocol acknowledgement failed");
            var partyConfig = MagWorldRegistryConfig.Parse("""
                {"format":"mag_world_registry_v1","application_id":20231,"client_address":"192.0.2.212","client_tcp_port":10473,
                 "party_invite_seconds":30,"workers":[{"worker_id":"synthetic212","worker_host":"127.0.0.1","worker_port":19999,
                 "minimum_players":2,"maximum_players":2,"minimum_players_per_team":[1,1],"draining":false}]}
                """);
            var parties = new MagWorldRegistry(partyConfig, store);
            try
            {
                int legacyMessages = 0;
                parties.RegisterPartySession(mls, () => true, _ => legacyMessages++, _ => {});
                parties.RoutePartyMembership(mls, new MagRetailPartyInvite {Inviter = character.Id, Invitee = 999, PartyId = 0});
                Check(legacyMessages == 0, "1793 connection entered the unverified1740 party registry");
            }
            finally {parties.DisposeAsync().AsTask().GetAwaiter().GetResult();}
            var clan = RawEnvelope(Convert.FromHexString("01fc020100000001000000"), 1);
            Check(mls.HandleMany(clan, now).Count == 0 && mls.LastRefusal == "unknown-or-malformed-type",
                "1793 used the unverified legacy clan decoder");
            // A legacy training catalog must not mutate a 1793 character just
            // because the header-only message number happens to be unchanged.
            store.TrainingCompletionCatalog = MagTrainingCompletionCatalog.Load(
                TrainingLevelChecks.Catalog(Path.Combine(directory, "legacy-training.json"), 3));
            byte[] beforeTraining = File.ReadAllBytes(Path.Combine(directory, "characters.json"));
            Check(mls.HandleMany(Envelope(new MagTrainingCompletedRequest()), now).Count == 0 &&
                beforeTraining.SequenceEqual(File.ReadAllBytes(Path.Combine(directory, "characters.json"))),
                "1793 character received an unqualified legacy training reward");
            var logout = mls.HandleMany(Envelope(new MagRetailAccountLogoutRequest()), now).Single();
            Check(logout.PluginId == 1 && Read(logout) is MagRetailAccountLogoutResponse {Success: true}, "1793 MLS logout not acknowledged");
            registry.Release("synthetic-212-channel");

            // A connection's negotiated format remains pinned through later config reloads.
            settings.MagProtocolVersion = 1740;
            Check(Read(Mas(new MagKeepAlive()).Single()) is MagKeepAlive, "Config change switched a live session codec");
        }
        finally
        {
            settings.MagApplicationId = oldApp; settings.MagProtocolVersion = oldProtocol;
            Directory.Delete(directory, true);
        }
    }
}
