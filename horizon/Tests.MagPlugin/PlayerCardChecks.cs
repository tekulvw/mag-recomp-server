using System.Text;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Medius;

// Invented accounts, names and NP identities built in memory; no client data or real stores.
internal static partial class LeadershipProgressionChecks
{
    private static byte[] SyntheticNpId(string onlineId)
    {
        var value = new byte[36];
        Encoding.ASCII.GetBytes(onlineId).CopyTo(value, 0);
        return value;
    }

    private static void CreateLinked(LocalCharacterStore store, string account, string name, byte[] npId)
    {
        var request = new MagCreateCharacterRequest { AccountName = name, Faction = 1, UniverseId = 26, NpId = npId };
        Check(store.CreateForAccount(request, account).Result == MagNetResult.Success, "Synthetic linked character creation failed");
    }

    private static RT_MSG_CLIENT_APP_TO_PLUGIN CardEnvelope(MagMessage message, int applicationId)
    {
        byte[] payload = MagMessageCodec.SerializeForApplication(message, applicationId);
        using var buffer = new MemoryStream();
        using (var writer = new BinaryWriter(buffer, Encoding.UTF8, leaveOpen: true))
        { writer.Write((ushort)payload.Length); writer.Write((byte)0); writer.Write(payload); }
        buffer.Position = 0;
        using var reader = new MessageReader(buffer);
        var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN(); envelope.Deserialize(reader);
        Check(envelope.IsValid, "Synthetic character-data envelope rejected");
        return envelope;
    }

    private static MagCharacterDataResponse AskCard(MagMlsSession session, MagCharacterDataRequest request)
    {
        var replies = session.HandleMany(CardEnvelope(request, session.ApplicationId), DateTime.UtcNow);
        Check(replies.Count == 1, "A ready MLS session must answer every character-data request exactly once");
        Check(MagMessageCodec.TryParseForApplication(replies[0].Payload, session.ApplicationId, out var parsed) &&
            parsed is MagCharacterDataResponse, "Character-data reply did not parse as a 0239 response");
        var response = (MagCharacterDataResponse)parsed;
        Check(response.TransactionId == request.TransactionId && response.RequestId == request.RequestId,
            "Character-data reply lost the request's transaction or request id");
        return response;
    }

    private static void WithApplication(int applicationId, Action test)
    {
        var settings = Server.Medius.Program.Settings;
        int previous = settings.MagApplicationId;
        settings.MagApplicationId = applicationId;
        try { test(); }
        finally { settings.MagApplicationId = previous; }
    }

    public static void PlayerCardLookup() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, Catalog()) };
        Create(store, "alpha", "Alpha");
        CreateLinked(store, "examplebeta", "ExampleBeta!0", SyntheticNpId("ExampleBeta"));
        var beta = store.ReadForAccount("examplebeta").Single();
        Check(beta.Id == 2, "Synthetic second character id");
        foreach (int applicationId in new[] { 20230, 20231 })
            WithApplication(applicationId, () =>
            {
                var session = MakeSession(store, applicationId, "alpha", 1);

                var byId = new MagCharacterDataRequest { TransactionId = 41, QueryKind = 2, AccountId = beta.Id, RequestId = 7 };
                // The retail by-id request is 76 bytes: header, kind, 32-bit id and request id.
                if (applicationId == 20231)
                    Check(MagMessageCodec.SerializeForApplication(byId, applicationId).Length == 76, "Synthetic by-id request width changed");
                var card = AskCard(session, byId);
                Check(card.Success && card.Record.AccountId == beta.Id && card.Record.Name == "ExampleBeta!0" && card.Record.Faction == 1 &&
                    card.Record.NpId.SequenceEqual(beta.NpId), "By-id lookup must return another account's full character record");
                Check(session.StorageStatus == "character-data-served", "By-id lookup status");

                var own = AskCard(session, new MagCharacterDataRequest { TransactionId = 42, QueryKind = 2, AccountId = 1, RequestId = 8 });
                Check(own.Success && own.Record.Name == "Alpha", "By-id lookup must also answer for the session's own character");

                var byName = AskCard(session, new MagCharacterDataRequest { TransactionId = 43, QueryKind = 1, AccountName = "ExampleBeta!0", RequestId = 9 });
                Check(byName.Success && byName.Record.AccountId == beta.Id, "By-name lookup must match another account's stored name");
                var byShown = AskCard(session, new MagCharacterDataRequest { TransactionId = 44, QueryKind = 1, AccountName = "ExampleBeta", RequestId = 10 });
                Check(byShown.Success && byShown.Record.AccountId == beta.Id, "By-name lookup must match the name shown in a match");

                foreach (var unknown in new[] {
                    new MagCharacterDataRequest { TransactionId = 45, QueryKind = 2, AccountId = 99, RequestId = 11 },
                    new MagCharacterDataRequest { TransactionId = 46, QueryKind = 2, AccountId = 0, RequestId = 12 },
                    new MagCharacterDataRequest { TransactionId = 47, QueryKind = 1, AccountName = "Nobody", RequestId = 13 },
                    new MagCharacterDataRequest { TransactionId = 48, QueryKind = 1, AccountName = "examplebeta!0", RequestId = 14 } })
                {
                    var missing = AskCard(session, unknown);
                    var blank = new MagCharacterRecord();
                    Check(!missing.Success && missing.Record.AccountId == 0 && missing.Record.Name == string.Empty &&
                        missing.Record.Level == blank.Level && missing.Record.NpId.All(b => b == 0),
                        "Unknown character must get a failure carrying only the cleared record");
                    Check(session.StorageStatus == "character-data-not-found", "Unknown character status");
                }
            });

        // A session that has not finished the protocol handshake stays silent.
        WithApplication(20231, () =>
        {
            var now = DateTime.UtcNow;
            var registry = new MagLoginRegistry();
            var ticket = registry.IssueForAccount("alpha", 1, 1, now);
            var pending = new MagMlsSession { CharacterStore = store };
            Check(pending.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20231, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey },
                registry, "synthetic-channel", now) && pending.Accept(now) && pending.CompleteTransport(now), "Synthetic pending MLS session setup failed");
            var request = new MagCharacterDataRequest { TransactionId = 49, QueryKind = 2, AccountId = beta.Id, RequestId = 15 };
            Check(pending.HandleMany(CardEnvelope(request, 20231), now).Count == 0, "A non-ready MLS session answered a character-data request");
            Check(new MagMlsSession { CharacterStore = store }.HandleMany(CardEnvelope(request, 20231), now).Count == 0,
                "An unbound MLS session answered a character-data request");
        });

        // An unreadable store is reported as a failure instead of leaving the card waiting.
        File.WriteAllText(Path.Combine(directory, "characters.json"), "{ not json");
        WithApplication(20231, () =>
        {
            var session = MakeSession(store, 20231, "alpha", 1);
            var broken = AskCard(session, new MagCharacterDataRequest { TransactionId = 50, QueryKind = 2, AccountId = beta.Id, RequestId = 16 });
            Check(!broken.Success && session.StorageStatus == "character-data-storage-error", "Unreadable store must answer with a failure");
        });
    });

    public static void PlayerCardDisplayName() => WithDirectory(directory =>
    {
        Check(StoredCharacter.DisplayNameFor("ExampleLinux!0", SyntheticNpId("ExampleLinux")) == "ExampleLinux", "Online ID must replace the stored login key");
        Check(StoredCharacter.DisplayNameFor("Example!1", SyntheticNpId("ExampleSixteen16")) == "ExampleSixteen16", "A full 16-byte online ID must be used whole");
        Check(StoredCharacter.DisplayNameFor("Alpha", new byte[36]) == "Alpha", "A zero NP identity keeps the stored name");
        Check(StoredCharacter.DisplayNameFor("Alpha", null) == "Alpha", "A missing NP identity keeps the stored name");
        Check(StoredCharacter.DisplayNameFor("Alpha", new byte[35]) == "Alpha", "A short NP identity keeps the stored name");
        var spaced = SyntheticNpId("Example Space");
        Check(StoredCharacter.DisplayNameFor("Alpha", spaced) == "Alpha", "A non-printable online ID keeps the stored name");
        var high = SyntheticNpId("ExampleHigh"); high[0] = 0xC5;
        Check(StoredCharacter.DisplayNameFor("Alpha", high) == "Alpha", "A non-ASCII online ID keeps the stored name");

        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
        CreateLinked(store, "examplelinux", "ExampleLinux!0", SyntheticNpId("ExampleLinux"));
        var stored = store.ReadForAccount("examplelinux").Single();
        Check(stored.Name == "ExampleLinux!0" && stored.ToRecord().Name == "ExampleLinux!0", "The stored login key must stay the character's name");
        Check(stored.DisplayName() == "ExampleLinux", "A stored linked character must display its online ID");
    });

    public static void PlayerCardAdmissionName() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, Catalog()) };
        CreateLinked(store, "alpha", "Alpha!0", SyntheticNpId("Alpha"));
        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Linked-character synthetic admission failed");
            var character = bridge.Latest("reserve")["character"]!.AsObject();
            Check(character["name"]!.GetValue<string>() == "Alpha", "Gameplay admission must carry the display name, not the stored login key");
        }).GetAwaiter().GetResult();

        var plain = new LocalCharacterStore(Path.Combine(directory, "plain.json")) { ProgressionCatalog = Load(directory, Catalog()) };
        Create(plain, "alpha", "Alpha");
        WithAdmission(plain, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Plain synthetic admission failed");
            Check(bridge.Latest("reserve")["character"]!["name"]!.GetValue<string>() == "Alpha", "A zero NP identity must admit the stored name");
        }).GetAwaiter().GetResult();
    });
}
