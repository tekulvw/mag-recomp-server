using System.Text;
using System.Security.Cryptography;
using RT.Models;
using RT.Models.MAG;
using Server.Medius;

internal static partial class DualAuthenticationChecks
{
    static CharacterCreationOutcome ConsoleCreate(LocalCharacterStore store, MagCreateCharacterRequest request, string owner) =>
        store.CreateForAccount(request, owner, "ConsoleUser", true);
    static MagCreateCharacterRequest ConflictRequest(string name = "ConsoleUser!0")
    {
        byte[] np = new byte[36]; Encoding.ASCII.GetBytes("ConsoleUser").CopyTo(np, 0);
        return new MagCreateCharacterRequest { AccountName = name, Faction = 1, UniverseId = 26, NpId = np };
    }
    internal static void ConflictDeletion(LocalCharacterStore store)
    {
        var old = store.ReadForAccount("portalowner").Single();
        Check(store.DeleteCharacterForAccount("portalowner", old.Id, old.Name).Result == MagNetResult.Success, "Delete assigned character");
        Check(ConsoleCreate(store, ConflictRequest(), "portalowner").Result == MagNetResult.Success, "Deleted alias releases slot and alias");
        var next = store.ReadForAccount("portalowner").Single();
        Check(next.Id != old.Id && next.Slot == 0 && next.ConsoleRequestedName == "ConsoleUser!0", "Recreation uses new live identity");
        string backups = Path.Combine(Path.GetTempPath(), "mag-console-restore-" + Guid.NewGuid().ToString("N"));
        try
        {
            string deletedName = store.ReadCharacter("portalowner", old.Id).Name;
            Check(store.RestoreCharacterForAdmin("portalowner", deletedName, true, backups).Applied, "Restore historical character after alias reuse");
            foreach (int app in new[] { 20230, 20231 })
            {
                var restored = store.ReadCharacter("portalowner", old.Id);
                var payload = MagMessageCodec.SerializeForApplication(new MagCharacterDataResponse { Success = true, Record = restored.ToRecord() }, app);
                Check(MagMessageCodec.TryParseForApplication(payload, app, out var decoded) && decoded is MagCharacterDataResponse { Success: true } response &&
                    response.Record.AccountId == restored.Id && response.Record.Name == restored.Name, "Successfully restored record serializes in both profiles");
            }
            Check(store.ReadCharacter("portalowner", old.Id).ConsoleRequestedName == null &&
                store.ReadCharacter("portalowner", next.Id).ConsoleRequestedName == "ConsoleUser!0", "Restored history cannot steal the new live alias");
        }
        finally { if (Directory.Exists(backups)) Directory.Delete(backups, true); }
    }
    internal static void RestoreChosenNameChecks(LocalCharacterStore store)
    {
        CharacterCreationOutcome Create(string name, string owner, bool sony = false) =>
            store.CreateForAccount(ConflictRequest(name), owner, "ConsoleUser", sony);
        void WireRecord(StoredCharacter character)
        {
            foreach (int app in new[] { 20230, 20231 })
            {
                var payload = MagMessageCodec.SerializeForApplication(new MagCharacterDataResponse { Success = true, Record = character.ToRecord() }, app);
                Check(MagMessageCodec.TryParseForApplication(payload, app, out var decoded) && decoded is MagCharacterDataResponse { Success: true } data &&
                    data.Record.Name == character.Name && data.Record.AccountId == character.Id, "Restoration leaves usable full records in both profiles");
            }
        }
        void RefusedWithoutMutation(string owner, StoredCharacter deleted, string reason)
        {
            string before = System.Text.Json.JsonSerializer.Serialize(store.ReadAll());
            string backups = Path.Combine(Path.GetTempPath(), "mag-restore-refusal-" + Guid.NewGuid().ToString("N"));
            try
            {
                foreach (bool apply in new[] { false, true })
                {
                    var outcome = store.RestoreCharacterForAdmin(owner, deleted.Name, apply, backups);
                    Check(outcome.Refusal && !outcome.Applied && outcome.Reason == reason,
                        "Restore must refuse unsafe final chosen name in dry run and apply: " + outcome.Reason);
                    Check(System.Text.Json.JsonSerializer.Serialize(store.ReadAll()) == before && !Directory.Exists(backups),
                        "Refused restoration must not mutate rows or create backups");
                }
            }
            finally { if (Directory.Exists(backups)) Directory.Delete(backups, true); }
        }
        Check(Create("Hero", "restorealpha").Result == MagNetResult.Success, "Hero restore fixture");
        var hero = store.ReadForAccount("restorealpha").Single();
        string aside = "Hero~" + hero.Id;
        Check(Create(aside, "restorebravo").Result == MagNetResult.Success && Create(aside, "restorealpha", true).Result == MagNetResult.Success, "Owned alias for eventual moved-aside name");
        var alias = store.ReadForAccount("restorealpha").Single(c => c.Id != hero.Id);
        var holder = store.ReadForAccount("restorebravo").Single();
        Check(store.DeleteCharacterForAccount("restorebravo", holder.Id, holder.Name).Result == MagNetResult.Success &&
            store.DeleteCharacterForAccount("restorealpha", hero.Id, hero.Name).Result == MagNetResult.Success &&
            Create("Hero", "restorebravo").Result == MagNetResult.Success, "Original name is held; moved-aside name is now an owned alias");
        RefusedWithoutMutation("restorealpha", store.ReadCharacter("restorealpha", hero.Id), "character-name-alias-taken");
        Check(store.ReadForAccount("restorealpha").Count(c => c.Name == aside || c.ConsoleRequestedName == aside) == 1, "Final chosen name cannot make selection ambiguous");
        WireRecord(store.ReadCharacter("restorealpha", alias.Id));

        string longRequest = new string('L', 29) + "!1";
        Check(Create(longRequest, "longrestorehold").Result == MagNetResult.Success && Create(longRequest, "longrestore", true).Result == MagNetResult.Success, "Long console restore fixture");
        var assigned = store.ReadForAccount("longrestore").Single();
        Check(assigned.Name.Length == 31, "Assigned name uses full record capacity");
        WireRecord(assigned);
        Check(store.DeleteCharacterForAccount("longrestore", assigned.Id, assigned.Name).Result == MagNetResult.Success &&
            Create(assigned.Name, "longrestoreclaim").Result == MagNetResult.Success, "Assigned name claimed after deletion");
        var deleted = store.ReadCharacter("longrestore", assigned.Id);
        Check(deleted.Name.Length == 32, "Deletion moved-aside fixture exceeds full record capacity");
        RefusedWithoutMutation("longrestore", deleted, "character-name-unserializable");
        Check(store.ReadForAccount("longrestore").Count == 0 && store.ReadCharacter("longrestore", assigned.Id).Deleted, "Unserializable identity stays deleted");
        WireRecord(store.ReadForAccount("longrestoreclaim").Single());
    }
    internal static void ConflictBounds(LocalCharacterStore store)
    {
        string requested = new string('X', 30) + "!2";
        Check(store.CreateForAccount(ConflictRequest(requested), "blocker", "ConsoleUser").Result == MagNetResult.Success, "Long name fixture");
        Check(ConsoleCreate(store, ConflictRequest(requested), "longowner").Result == MagNetResult.Success, "Maximum length conflict");
        var longName = store.ReadForAccount("longowner").Single();
        Check(longName.Name.Length == 31 && longName.Name.All(c => c >= 33 && c <= 126) && longName.Name.EndsWith("!2") && longName.Slot == 2, "Bounded ASCII assignment preserves slot");
        Check(store.CreateForAccount(ConflictRequest("NoSuffixName"), "noholder", "ConsoleUser").Result == MagNetResult.Success &&
            ConsoleCreate(store, ConflictRequest("NoSuffixName"), "nosuffix").Result == MagNetResult.Success &&
            store.ReadForAccount("nosuffix").Single().Name.Length <= 32, "Names without slot suffix also assign safely");
        string legacyLong = new string('K', 30) + "!1";
        Check(store.CreateForAccount(ConflictRequest(legacyLong), "legacylong", "ConsoleUser").Result == MagNetResult.Success &&
            store.ReadForAccount("legacylong").Single().Name == legacyLong && store.ReadForAccount("legacylong").Single().ConsoleRequestedName == null,
            "Non-Sony store API retains its existing 32-byte name behavior");
        Check(ConsoleCreate(store, ConflictRequest(new string('X', 33)), "invalidname").Result == MagNetResult.NameInvalid, "Name bound retained");
        var bad = ConflictRequest(); bad.NpId[0] = (byte)'X';
        Check(ConsoleCreate(store, bad, "badidentity").Reason == "np-id-account-mismatch", "Fallback retains verified NP validation");
    }
    internal static void ConflictWire(LocalCharacterStore store, bool fresh = false, string requestedName = "ConsoleUser!0")
    {
        using var portal = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var sony = PsnTicketChecks.Ticket(Now, 42, "ConsoleUser", Service, out var keys);
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        try
        {
            foreach (int app in new[] { 20230, 20231 })
            {
                Server.Medius.Program.Settings.MagApplicationId = app;
                if (fresh)
                {
                    foreach (var prior in store.ReadForAccount("portalowner"))
                        Check(store.DeleteCharacterForAccount("portalowner", prior.Id, prior.Name).Result == MagNetResult.Success, "Fresh wire fixture releases owned slot");
                }
                MagNpAuthenticateRequest Auth(byte[] ticket)
                {
                    MagNpAuthenticateRequest request = app == 20231 ? new MagRetailNpAuthenticateRequest() : new MagNpAuthenticateRequest();
                    request.FullTicketSize = (ushort)ticket.Length; request.TicketChunkSize = (ushort)ticket.Length;
                    request.TicketChunk = ticket.Concat(new byte[900 - ticket.Length]).ToArray(); request.LastMessage = true;
                    return request;
                }
                var registry = new MagLoginRegistry();
                MagMasSession Mas(byte[] ticket)
                {
                    var mas = new MagMasSession { PortalTickets = new MagPortalTicketPolicy(Settings(portal, keys), () => Clock, identities: new Identities()),
                        CharacterStore = store, CharacterStorageEnabled = true, AccountLoginEnabled = true, CharacterDataEnabled = true, LoginRegistry = registry };
                    Wire(e => mas.HandleMany(true, app, e, Clock), new MagHello(), app);
                    Wire(e => mas.HandleMany(true, app, e, Clock), new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, app);
                    Wire(e => mas.HandleMany(true, app, e, Clock), Auth(ticket), app);
                    return mas;
                }
                MagMessage Reply(MagMasSession mas, MagMessage request)
                {
                    var replies = Wire(e => mas.HandleMany(true, app, e, Clock), request, app);
                    Check(replies.Count == 1, "Wire reply absent: " + mas.StorageStatus);
                    Check(MagMessageCodec.TryParseForApplication(replies[0].Payload, app, out var response), "Wire reply malformed");
                    return response;
                }
                MagAccountLoginRequest Login(string name)
                {
                    MagAccountLoginRequest request = app == 20231 ? new MagRetailAccountLoginRequest() : new MagAccountLoginRequest();
                    request.TransactionId = 2; request.VersionMajor = 3; request.VersionMinor = 3; request.VersionBuild = 1; request.AccountName = name;
                    return request;
                }
                var mas = Mas(sony);
                Check(mas.AuthenticatedProvider == "psn", "Only verified Sony provider activates fallback");
                if (requestedName.Length == 32)
                {
                    Check(!store.ReadAll().Any(c => c.Name == requestedName), "32-byte Sony request has no foreign name collision");
                    bool refusedEncoding = false;
                    try { MagMessageCodec.SerializeForApplication(ConflictRequest(requestedName), app); }
                    catch (ArgumentException) { refusedEncoding = true; }
                    Check(refusedEncoding, "Create wire CString remains limited to 31 bytes");
                    Check(ConsoleCreate(store, ConflictRequest(requestedName), "portalowner").Result == MagNetResult.Success,
                        "Store API accepts and assigns a safe name for a 32-byte Sony request");
                }
                else Check(Reply(mas, ConflictRequest(requestedName)) is MagCreateCharacterResponse { Result: MagNetResult.Success }, "Wire create retry using original name");
                var own = store.ReadForAccount("portalowner").Single();
                if (requestedName.Length == 32)
                {
                    Check(own.Name.Length <= 31 && own.Name.EndsWith("!0") && own.ConsoleRequestedName == requestedName,
                        "Noncolliding 32-byte Sony request receives a wire-safe assigned name and durable alias");
                    Check(ConsoleCreate(store, ConflictRequest(requestedName), "portalowner").Result == MagNetResult.Success &&
                        store.ReadForAccount("portalowner").Single().Id == own.Id, "Long-name exact wire creation retry stays stable");
                }
                var foreign = store.ReadForAccount("otherowner").Single();
                Check(Reply(mas, Login(foreign.Name)) is MagAccountLoginResponse { Result: 0 }, "Foreign assigned name must not select a foreign character");
                var login = Reply(mas, Login(requestedName)) as MagAccountLoginResponse;
                Check(login is { Result: 1 } && login.AccountId == own.Id, "Original alias login selects owner");
                Check(Wire(e => mas.HandleMany(true, app, e, Clock), new MagCharacterDataRequest { QueryKind = 1, AccountName = foreign.Name, RequestId = 3 }, app).Count == 0 &&
                    Wire(e => mas.HandleMany(true, app, e, Clock), new MagCharacterDataRequest { QueryKind = 2, AccountId = foreign.Id, RequestId = 3 }, app).Count == 0,
                    "MAS character data must retain selected owner identity");
                Check(Reply(mas, new MagCharacterDataRequest { QueryKind = 1, AccountName = requestedName, RequestId = 3 }) is MagCharacterDataResponse { Success: true } data &&
                    data.Record.AccountId == own.Id && data.Record.Name == own.Name, "Original alias data returns assigned owner record");
                var mls = new MagMlsSession { CharacterStore = store };
                string Key(byte[] bytes) => Encoding.ASCII.GetString(bytes).TrimEnd('\0');
                Check(mls.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = app, TargetWorldId = 1, AccessToken = Key(login.AccessKey), SessionKey = Key(login.SessionKey) },
                    registry, "console-alias-handoff-" + app, Clock) && mls.AuthenticatedAccount == "portalowner" && mls.CharacterId == own.Id && mls.Accept(Clock) && mls.CompleteTransport(Clock), "Registry/MLS canonical owner handoff");
                mas.EndPortalSession();
                var portalMas = Mas(Portal(portal));
                Check(portalMas.AuthenticatedProvider == "portal", "Portal provider retained");
                var portalCreation = ConflictRequest(requestedName); portalCreation.NpId = new byte[36];
                if (requestedName.Length == 32)
                    Check(store.CreateForAccount(portalCreation, "portalowner", portalMas.AuthenticatedOnlineId).Result == MagNetResult.NameTaken, "Portal API creation does not opt in to aliases");
                else Check(Reply(portalMas, portalCreation) is MagCreateCharacterResponse { Result: MagNetResult.NameTaken }, "Portal creation does not opt in to aliases");
                Check(Reply(portalMas, Login(requestedName)) is MagAccountLoginResponse { Result: 0 }, "Portal original alias cannot bypass collision handling");
                portalMas.EndPortalSession();
                var maps = new MagMapsSession { PortalTickets = new MagPortalTicketPolicy(Settings(portal, keys), () => Clock, identities: new Identities()), CharacterStore = store, CharacterStorageEnabled = true };
                Wire(e => maps.HandleMany(true, app, e, Clock), new MagHello(), app);
                if (app == 20230) Wire(e => maps.HandleMany(true, app, e, Clock), new MagUniverseListRequest(), app);
                Wire(e => maps.HandleMany(true, app, e, Clock), Auth(sony), app);
                if (app == 20231) Wire(e => maps.HandleMany(true, app, e, Clock), new MagRetailUniverseListRequest(), app);
                Wire(e => maps.HandleMany(true, app, e, Clock), new MagNewsEulaRequest { Language = "english" }, app);
                var listed = Wire(e => maps.HandleMany(true, app, e, Clock), new MagCharacterListRequest(), app);
                Check(listed.Count == 1 && MagMessageCodec.TryParseForApplication(listed[0].Payload, app, out var list) && list is MagCharacterListResponse { Success: true } chars &&
                    chars.Characters.Single(c => c.AccountId != 0).Name == own.Name, "Character list exposes assigned name");
                maps.EndPortalSession();
            }
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    }
    public static void CharacterConflicts()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-console-conflict-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new Committer());
            Check(store.CreateForAccount(ConflictRequest(), "foreignowner", "ConsoleUser").Result == MagNetResult.Success, "Foreign fixture");
            Check(store.CreateForAccount(ConflictRequest(), "portalowner", "ConsoleUser").Result == MagNetResult.NameTaken, "Online ID alone must not opt in");
            Check(ConsoleCreate(store, ConflictRequest(), "portalowner").Result == MagNetResult.Success, "Sony collision fallback");
            var assigned = store.ReadForAccount("portalowner").Single();
            var independent = new LocalCharacterStore(Path.Combine(directory, "independent.json"), new Committer());
            Check(independent.CreateForAccount(ConflictRequest(), "foreignowner", "ConsoleUser").Result == MagNetResult.Success &&
                ConsoleCreate(independent, ConflictRequest(), "portalowner").Result == MagNetResult.Success &&
                independent.ReadForAccount("portalowner").Single().Name != assigned.Name, "Fallback names cannot be predicted from public owner/request alone");
            Check(assigned.Name != "ConsoleUser!0" && assigned.Name.EndsWith("!0") && assigned.Name.Length <= 32 && assigned.Slot == 0, "Assigned name and slot");
            Check(ConsoleCreate(store, ConflictRequest(), "portalowner").Result == MagNetResult.Success && store.ReadForAccount("portalowner").Single().Id == assigned.Id, "Exact creation retry");
            var development = ConflictRequest(); development.NpId = new byte[36];
            Check(store.CreateForAccount(development, "devowner").Result == MagNetResult.NameTaken, "Development collisions retain NameTaken");
            Check(ConsoleCreate(store, ConflictRequest("consoleuser!0"), "portalowner").Result == MagNetResult.NameTaken, "Case-only alias change is not an exact retry");
            var changed = ConflictRequest(); changed.HeadId = 4;
            Check(ConsoleCreate(store, changed, "portalowner").Result == MagNetResult.NameTaken, "Alias must not make changed request idempotent");
            store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new Committer());
            Check(ConsoleCreate(store, ConflictRequest(), "portalowner").Result == MagNetResult.Success && store.ReadForAccount("portalowner").Single().Id == assigned.Id, "Persisted alias restart retry");
            Check(ConsoleCreate(store, ConflictRequest(), "otherowner").Result == MagNetResult.Success, "Aliases scoped by owner");
            Check(store.ReadForAccount("foreignowner").Single().Name == "ConsoleUser!0", "Foreign holder preserved");
            ConflictWire(store, fresh: true);
            store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new Committer());
            ConflictWire(store); // A new store and newly authenticated connections must resolve the durable alias.
            ConflictDeletion(store);
            RestoreChosenNameChecks(new LocalCharacterStore(Path.Combine(directory, "restore-chosen.json"), new Committer()));
            var longPath = Path.Combine(directory, "long-no-collision.json");
            var longStore = new LocalCharacterStore(longPath, new Committer());
            Check(longStore.CreateForAccount(ConflictRequest("Foreign!0"), "otherowner", "ConsoleUser").Result == MagNetResult.Success, "Unrelated foreign selection fixture");
            string longRequested = new string('N', 30) + "!0";
            ConflictWire(longStore, fresh: true, requestedName: longRequested);
            ConflictWire(new LocalCharacterStore(longPath, new Committer()), requestedName: longRequested);
            ConflictBounds(new LocalCharacterStore(Path.Combine(directory, "bounds.json"), new Committer()));
        }
        finally { Directory.Delete(directory, true); }
    }
}
