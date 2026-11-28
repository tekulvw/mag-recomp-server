using System.Text;
using System.Text.Json;
using Npgsql;
using RT.Models;
using RT.Models.MAG;
using Server.Medius;

// Character slots: up to three characters per account, soft deletion with the name moved aside,
// operator restore and purge, the MAPS list, the MAS delete request and the admin tools. Every
// account, name and reward is invented here.
internal static partial class LeadershipProgressionChecks
{
    private static readonly DateTimeOffset SlotNow = new DateTimeOffset(2026, 11, 27, 12, 0, 0, TimeSpan.Zero);

    /// <summary>Characters as a schema-1 or schema-2 database held them: one per account, no slot.</summary>
    private const string SchemaOneCharacters =
        "INSERT INTO mag.characters (id, account, name, faction, head_id, voice_id, universe_id, np_id, experience, level) VALUES " +
        "(1, 'alpha', 'Alpha', 0, 0, 0, 26, decode(repeat('00', 36), 'hex'), 0, 1), " +
        "(2, 'bravo', 'Bravo', 1, 0, 0, 26, decode(repeat('00', 36), 'hex'), 0, 1); " +
        "SELECT setval('mag.character_ids', 2)";

    private static MagCreateCharacterRequest SlotRequest(string name, byte faction = 0) =>
        new MagCreateCharacterRequest { AccountName = name, Faction = faction, UniverseId = 26, NpId = new byte[36] };

    private static uint IdOf(LocalCharacterStore store, string account, string name) =>
        store.ReadEveryCharacter().Single(c => c.Account == account && c.Name == name).Id;

    /// <summary>The slot rules, run identically against the JSON store and PostgreSQL; returns a log for comparison.</summary>
    private static List<string> RunSlotScript(LocalCharacterStore store, string backups)
    {
        var log = new List<string>();
        string Outcome(CharacterCreationOutcome o) => $"{o.Result}/{o.Reason}";
        string Slots(string account) => string.Join(",", store.ReadForAccount(account).Select(c => $"{c.Slot}:{c.Name}"));

        // Three slots, any faction in any slot; the fourth create is refused with the client's generic error.
        Check(Outcome(store.CreateForAccount(SlotRequest("alpha!0", 0), "alpha")) == "Success/local-profile-committed", "First slot");
        Check(Outcome(store.CreateForAccount(SlotRequest("alpha!2", 2), "alpha")) == "Success/local-profile-committed", "Named slot");
        Check(Outcome(store.CreateForAccount(SlotRequest("Plain", 0), "alpha")) == "Success/local-profile-committed", "Lowest free slot");
        Check(Slots("alpha") == "0:alpha!0,1:Plain,2:alpha!2", "The !N suffix picks its slot, else the lowest free one: " + Slots("alpha"));
        var full = store.CreateForAccount(SlotRequest("alpha!1", 1), "alpha");
        Check(full.Result == MagNetResult.Error && full.Reason == "character-slots-full", "A fourth character: " + Outcome(full));
        Check(Outcome(store.CreateForAccount(SlotRequest("alpha!2", 2), "alpha")) == "Success/existing-exact-profile", "An exact retry still succeeds");
        Check(store.CreateForAccount(SlotRequest("alpha!2", 1), "alpha").Result == MagNetResult.NameTaken, "A different profile under a taken name");
        Check(store.CreateForAccount(SlotRequest("Alpha!0"), "bravo").Result == MagNetResult.NameTaken, "Names are unique across accounts, any case");
        Check(Outcome(store.CreateForAccount(SlotRequest("bravo!0", 1), "bravo")) == "Success/local-profile-committed", "Another account is unaffected");
        log.Add("created " + Slots("alpha") + " | " + Slots("bravo"));

        // Progression is per character: rewards land on the character that earned them.
        uint zero = IdOf(store, "alpha", "alpha!0"), two = IdOf(store, "alpha", "alpha!2"), plain = IdOf(store, "alpha", "Plain");
        Check(store.ApplyRewardForAccount("alpha", two, "slot-kill-1", "slot-match", "enemy_killed", 1).Result == MagNetResult.Success, "Reward");
        Check(store.ApplyRewardForAccount("alpha", zero, "slot-kill-1", "slot-match", "enemy_killed", 1).Reason == "reward-receipt-mismatch",
            "A reward id belongs to one character");
        Check(store.ApplyRewardForAccount("bravo", two, "slot-kill-2", "slot-match", "enemy_killed", 1).Reason == "character-not-owned",
            "Another account's character takes no reward");
        var levels = store.ReadForAccount("alpha").ToDictionary(c => c.Name, c => c.ToRecord().Level);
        Check(levels["alpha!2"] == 2 && levels["alpha!0"] == 1 && levels["Plain"] == 1, "Only the rewarded character levelled");
        Check(store.SaveNominationsForAccount("alpha", plain, 1).Result == MagNetResult.Success, "Per-character save");
        log.Add("levels " + string.Join(",", store.ReadForAccount("alpha").Select(c => $"{c.Name}={c.ToRecord().Level}")));

        // Delete: the character leaves the list and its slot, keeps its progression, and its name moves aside.
        Check(store.DeleteCharacterForAccount("alpha", two, "alpha!0").Reason == "character-not-owned", "Id and name must agree");
        Check(store.DeleteCharacterForAccount("bravo", two, "alpha!2").Reason == "character-not-owned", "Only the owner deletes");
        Check(Outcome(store.DeleteCharacterForAccount("alpha", two, "alpha!2")) == "Success/character-deleted", "Delete");
        Check(Slots("alpha") == "0:alpha!0,1:Plain", "Deleted character left the list: " + Slots("alpha"));
        var deleted = store.ReadEveryCharacter().Single(c => c.Id == two);
        Check(deleted.Deleted && deleted.Name == "alpha!2~" + two && deleted.OriginalName == "alpha!2" && deleted.ToRecord().Level == 2,
            "Soft delete keeps the row, the progression and the original name");
        Check(store.ReadCharacter("alpha", two)?.Id == two && store.ReadAll().All(c => c.Id != two), "Playing paths still find it; lists do not");
        Check(store.DeleteCharacterForAccount("alpha", two, "alpha!2").Reason == "character-not-owned", "A deleted character is not deleted twice");
        Check(store.SaveNominationsForAccount("alpha", two, 1).Reason == "character-not-owned", "A deleted character takes no saves");
        Check(store.ApplyRewardForAccount("alpha", two, "slot-kill-3", "slot-match", "enemy_killed", 1).Result == MagNetResult.Success,
            "A reward still in flight lands on the deleted character");

        // The slot is reusable at once, under the same client-chosen name.
        Check(Outcome(store.CreateForAccount(SlotRequest("alpha!2", 1), "alpha")) == "Success/local-profile-committed", "Re-create in the freed slot");
        Check(Slots("alpha") == "0:alpha!0,1:Plain,2:alpha!2", "The new character took slot 2: " + Slots("alpha"));
        uint replacement = IdOf(store, "alpha", "alpha!2");
        Check(replacement != two && store.ReadCharacter("alpha", replacement).ToRecord().Level == 1, "A new character starts fresh");

        // Restore needs a free slot; then the original name, if free, else the moved-aside name.
        var noRoom = store.RestoreCharacterForAdmin("alpha", "alpha!2", true, backups);
        Check(noRoom.Refusal && noRoom.Reason == "character-slots-full", "Restore into a full account: " + noRoom.Reason);
        Check(store.RestoreCharacterForAdmin("alpha", "Plain", false, backups).Reason == "character-not-deleted", "Restore of a live character");
        Check(store.RestoreCharacterForAdmin("alpha", "nobody", false, backups).Reason == "account-not-found", "Restore of an unknown name");
        Check(store.DeleteCharacterForAccount("alpha", plain, "Plain").Result == MagNetResult.Success, "Delete slot 1");
        var dry = store.RestoreCharacterForAdmin("alpha", "alpha!2", false, backups);
        Check(!dry.Refusal && !dry.Applied && dry.Slot == 1 && dry.Name == "alpha!2~" + two && dry.CharacterId == two,
            "Dry run: slot 1, keeping the moved-aside name while alpha!2 is taken");
        Check(store.ReadEveryCharacter().Single(c => c.Id == two).Deleted, "A dry run wrote");
        var restored = store.RestoreCharacterForAdmin("alpha", "alpha!2", true, backups);
        Check(restored.Applied && restored.Slot == 1 && restored.Name == "alpha!2~" + two && restored.PreviousName == "alpha!2~" + two,
            "Restore with the original name taken");
        var back = store.ReadForAccount("alpha").Single(c => c.Id == two);
        Check(back.Slot == 1 && !back.Deleted && back.OriginalName == null && back.ToRecord().Level == 2, "Restored whole into slot 1");
        Check(Slots("alpha") == $"0:alpha!0,1:alpha!2~{two},2:alpha!2", "List after the first restore: " + Slots("alpha"));
        // Restoring "Plain" (by its original name) gets the name back, since nobody took it.
        Check(store.DeleteCharacterForAccount("alpha", replacement, "alpha!2").Result == MagNetResult.Success, "Delete the replacement");
        var named = store.RestoreCharacterForAdmin("alpha", "Plain", true, backups);
        Check(named.Applied && named.Name == "Plain" && named.PreviousName == "Plain~" + plain && named.Slot == 2, "Restore with the original name free");
        Check(Slots("alpha") == $"0:alpha!0,1:alpha!2~{two},2:Plain", "List after the second restore: " + Slots("alpha"));
        log.Add("restored " + Slots("alpha"));

        // Purge: only deleted characters, by either name; the row and its ledger go, the sanction stays.
        Check(store.PurgeCharacterForAdmin("alpha", "Plain", true, backups).Reason == "character-not-deleted", "Purge of a live character");
        var planned = store.PurgeCharacterForAdmin("alpha", "alpha!2", false, backups);
        Check(!planned.Refusal && planned.CharacterId == replacement && !planned.Applied, "Purge by the original name");
        Check(store.PurgeCharacterForAdmin("alpha", "alpha!2~" + replacement, true, backups).Applied, "Purge by the moved-aside name");
        Check(store.ReadEveryCharacter().All(c => c.Id != replacement), "Purged for good");
        Check(store.ChangeVoiceSanction("bravo", VoiceSanctionKind.Mute, false, null, "staff", null, true, backups).Applied, "Sanction");
        Check(store.DeleteCharacterForAccount("bravo", IdOf(store, "bravo", "bravo!0"), "bravo!0").Result == MagNetResult.Success, "Delete bravo");
        Check(store.PurgeCharacterForAdmin("bravo", "bravo!0", true, backups).Applied, "Purge bravo's last character");
        Check(store.ReadVoiceSanctions().Single().Account == "bravo", "A sanction outlives the account's characters");
        Check(store.ChangeVoiceSanction("bravo", VoiceSanctionKind.Mute, true, null, "staff", null, true, backups).Applied,
            "A sanction on an account without characters can still be lifted");
        log.Add("purged " + Slots("alpha") + " | " + Slots("bravo") + " | " + store.ReadEveryCharacter().Count);
        return log;
    }

    public static void CharacterSlotStore() => WithDirectory(directory =>
    {
        var catalog = Load(directory, Catalog());
        string path = Path.Combine(directory, "characters.json");
        var store = new LocalCharacterStore(path, new PlainCommitter()) { ProgressionCatalog = catalog, Clock = () => SlotNow };

        // A store with one character per account keeps the shape it always had: no slot field for slot 0.
        Check(store.CreateForAccount(SlotRequest("Single"), "single").Result == MagNetResult.Success, "Synthetic single character");
        string text = File.ReadAllText(path);
        Check(!text.Contains("\"slot\"") && !text.Contains("deletedAt") && !text.Contains("originalName"), "Slot 0 writes no new fields");
        var log = RunSlotScript(store, Path.Combine(directory, "backups"));
        Check(log.Count == 4, "Script log");
        var reread = new LocalCharacterStore(path, new PlainCommitter()) { ProgressionCatalog = catalog };
        Check(reread.ReadEveryCharacter().Count == store.ReadEveryCharacter().Count && reread.ReadForAccount("alpha").Count == 3,
            "A fresh reader accepts the slots, deletions and moved-aside names");
        Check(Directory.GetFiles(Path.Combine(directory, "backups")).Any(f => Path.GetFileName(f).StartsWith("characters.before-admin-character-restore-alpha-")) &&
            Directory.GetFiles(Path.Combine(directory, "backups")).Any(f => Path.GetFileName(f).StartsWith("characters.before-admin-character-purge-alpha-")),
            "JSON restores and purges keep a copy of the store first");
        Check(store.RestoreCharacterForAdmin("alpha", "anything", true, null).Reason != "applied", "Nothing applied without a backup directory");

        // The reader refuses two live characters in one slot and slots past the limit.
        string Doctored(Func<string, string> edit) { var copy = Path.Combine(directory, Guid.NewGuid() + ".json"); File.WriteAllText(copy, edit(File.ReadAllText(path))); return copy; }
        foreach (var bad in new[] {
            Doctored(t => t.Replace("\"name\": \"alpha!0\"", "\"name\": \"alpha!0\",\n      \"slot\": 2")),
            Doctored(t => t.Replace("\"name\": \"alpha!0\"", "\"name\": \"alpha!0\",\n      \"slot\": 3")),
            Doctored(t => t.Replace("\"name\": \"alpha!0\"", "\"name\": \"alpha!0\",\n      \"originalName\": \"x\"")) })
        {
            bool refused = false;
            try { new LocalCharacterStore(bad, new PlainCommitter()) { ProgressionCatalog = catalog }.ReadAll(); }
            catch (InvalidDataException) { refused = true; }
            Check(refused, "The reader accepted an invalid slot layout");
        }

        // Concurrent creations on one account fill distinct slots and stop at three.
        string racePath = Path.Combine(directory, "race.json");
        var race = new LocalCharacterStore(racePath, new PlainCommitter()) { ProgressionCatalog = catalog };
        var outcomes = Enumerable.Range(0, 6).AsParallel().WithDegreeOfParallelism(3)
            .Select(i => race.CreateForAccount(SlotRequest("Racer" + i), "racer")).ToList();
        Check(outcomes.Count(o => o.Result == MagNetResult.Success) == 3 && outcomes.Count(o => o.Reason == "character-slots-full") == 3 &&
            race.ReadForAccount("racer").Select(c => c.Slot).OrderBy(s => s).SequenceEqual(new byte[] { 0, 1, 2 }), "Concurrent creations");
    });

    private static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> MapsSend(MagMapsSession session, MagMessage message, DateTime now) =>
        session.HandleMany(true, 20231, CardEnvelope(message, 20231), now);

    public static void CharacterSlotSessions() => WithDirectory(directory => WithApplication(20231, () =>
    {
        var now = SlotNow.UtcDateTime;
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new PlainCommitter())
            { ProgressionCatalog = Load(directory, Catalog()), Clock = () => SlotNow };

        // The delete request and reply as the client codes them: a uint16-length name and a uint32 id; one bit.
        var request = new MagDeleteCharacterRequest { TransactionId = 0x0a0b0c0d, CharacterName = "Ab", CharacterId = 0x01020304 };
        byte[] wire = MagMessageCodec.SerializeForApplication(request, 20231);
        Check(wire.Length == 78 && wire[0] == 0x02 && wire[1] == 0x3c && wire[70] == 0 && wire[71] == 2 && wire[72] == (byte)'A' &&
            wire[74] == 1 && wire[77] == 4, "0x023C is the header, a 16-bit name length, the name and a 32-bit id");
        Check(MagMessageCodec.TryParseClientForApplication(wire, 20231, out var parsedRequest) &&
            parsedRequest is MagDeleteCharacterRequest { CharacterName: "Ab", CharacterId: 0x01020304, TransactionId: 0x0a0b0c0d } &&
            MagMessageCodec.TryParseClientForApplication(wire, 20230, out _), "Both clients' delete requests parse");
        Check(!MagMessageCodec.TryParseClientForApplication(wire[..77], 20231, out _), "A truncated delete request parsed");
        byte[] reply = MagMessageCodec.SerializeForApplication(new MagDeleteCharacterResponse { TransactionId = 5, Success = true }, 20231);
        Check(reply.Length == 71 && reply[0] == 0x02 && reply[1] == 0x3d && reply[70] == 0x80, "0x023D is the header and a success bit");
        Check(!MagMessageCodec.TryParseClientForApplication(reply, 20231, out _), "A server-only delete reply parsed as a client message");

        // The list: three allowed slots, characters at their slot's position, a cleared summary for a hole.
        MagRetailCharacterListResponse List()
        {
            var maps = new MagMapsSession { DevelopmentAuthEnabled = true, CharacterStorageEnabled = true, CharacterStore = store };
            MapsSend(maps, new MagHello(), now);
            var auth = new MagRetailNpAuthenticateRequest { TransactionId = 1, FullTicketSize = 5, TicketChunkSize = 5, LastMessage = true };
            Encoding.ASCII.GetBytes("alpha").CopyTo(auth.TicketChunk, 0);
            MapsSend(maps, auth, now);
            MapsSend(maps, new MagRetailUniverseListRequest(), now);
            MapsSend(maps, new MagNewsEulaRequest { Language = "english" }, now);
            var replies = MapsSend(maps, new MagCharacterListRequest { TransactionId = 9 }, now);
            MagMessage parsed = null;
            Check(replies.Count == 1 && MagMessageCodec.TryParseForApplication(replies[0].Payload, 20231, out parsed) &&
                parsed is MagRetailCharacterListResponse { Success: true, AllowedCharacters: 3, TransactionId: 9 }, "Character list: " + maps.StorageStatus);
            return (MagRetailCharacterListResponse)parsed;
        }
        Check(List().Characters.Count == 0, "A new account lists no characters and three allowed slots");
        foreach (var name in new[] { "alpha!0", "alpha!1", "alpha!2" }) Check(store.CreateForAccount(SlotRequest(name), "alpha").Result == MagNetResult.Success, name);
        Check(List().Characters.Select(c => c.Name).SequenceEqual(new[] { "alpha!0", "alpha!1", "alpha!2" }), "Three characters in slot order");

        // MAS: the delete request is answered once the account is authenticated, and only for its own character.
        MagMasSession Mas()
        {
            var mas = new MagMasSession { DevelopmentAuthEnabled = true, CharacterStorageEnabled = true, CharacterStore = store };
            MasSend(mas, new MagHello(), now);
            MasSend(mas, new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, now);
            var auth = new MagRetailNpAuthenticateRequest { TransactionId = 1, FullTicketSize = 5, TicketChunkSize = 5, LastMessage = true };
            Encoding.ASCII.GetBytes("alpha").CopyTo(auth.TicketChunk, 0);
            Check(MasSend(mas, auth, now).Count == 1 && mas.State == MagMasSession.SessionState.LocalDevelopmentAuthenticated, "MAS sign-in");
            return mas;
        }
        bool Delete(MagMasSession mas, string name, uint id, int transaction)
        {
            var replies = MasSend(mas, new MagDeleteCharacterRequest { TransactionId = transaction, CharacterName = name, CharacterId = id }, now);
            MagMessage answer = null;
            Check(replies.Count == 1 && MagMessageCodec.TryParseForApplication(replies[0].Payload, 20231, out answer) &&
                answer is MagDeleteCharacterResponse response && response.TransactionId == transaction, "One delete reply echoing the transaction");
            return ((MagDeleteCharacterResponse)answer).Success;
        }
        var unauthenticated = new MagMasSession { DevelopmentAuthEnabled = true, CharacterStorageEnabled = true, CharacterStore = store };
        MasSend(unauthenticated, new MagHello(), now);
        Check(MasSend(unauthenticated, new MagDeleteCharacterRequest { CharacterName = "alpha!1", CharacterId = 2 }, now).Count == 0,
            "An unauthenticated MAS session deleted");
        uint middle = IdOf(store, "alpha", "alpha!1");
        var session = Mas();
        Check(!Delete(session, "alpha!0", middle, 20), "A mismatched name and id must not delete");
        Check(Delete(session, "alpha!1", middle, 21) && session.StorageStatus == "character-deleted", "Delete the middle character");
        var holed = List().Characters;
        Check(holed.Count == 3 && holed[1].AccountId == 0 && holed[1].Name == "" && holed[1].Faction == 0xff && holed[2].Name == "alpha!2",
            "A freed middle slot is a cleared summary, so slot 2 keeps its place");
        Check(store.CreateForAccount(SlotRequest("alpha!1"), "alpha").Result == MagNetResult.Success, "Re-create in the freed middle slot");
        Check(List().Characters.Select(c => c.Name).SequenceEqual(new[] { "alpha!0", "alpha!1", "alpha!2" }), "The slot is back in place");
        Check(Delete(Mas(), "alpha!2", IdOf(store, "alpha", "alpha!2"), 22) && List().Characters.Count == 2, "A freed last slot is left out");

        // Logging in binds the character the player picked, by its listed name.
        var registry = new MagLoginRegistry();
        var login = new MagMasSession { DevelopmentAuthEnabled = true, AccountLoginEnabled = true, CharacterStorageEnabled = true,
            CharacterStore = store, LoginRegistry = registry };
        MasSend(login, new MagHello(), now);
        MasSend(login, new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, now);
        var signIn = new MagRetailNpAuthenticateRequest { TransactionId = 1, FullTicketSize = 5, TicketChunkSize = 5, LastMessage = true };
        Encoding.ASCII.GetBytes("alpha").CopyTo(signIn.TicketChunk, 0);
        MasSend(login, signIn, now);
        var granted = MasSend(login, new MagRetailAccountLoginRequest { TransactionId = 2, VersionMajor = 3, VersionMinor = 3, VersionBuild = 1, AccountName = "alpha!1" }, now);
        Check(granted.Count == 1 && MagMessageCodec.TryParseForApplication(granted[0].Payload, 20231, out var loginReply) &&
            loginReply is MagRetailAccountLoginResponse { Result: 1 } ok && ok.AccountId == IdOf(store, "alpha", "alpha!1"),
            "Login binds the picked character's id");

        // Player cards: an online ID that one account's characters share names the one it plays.
        var cards = store.ReadAll();
        var byShown = new MagCharacterDataRequest { QueryKind = 1, AccountName = "alpha!1" };
        Check(MagMlsSession.FindCharacter(cards, byShown, _ => false)?.Name == "alpha!1", "Stored names still match first");
    }));

    /// <summary>
    /// After an in-game delete the client stays on character select with its MAS channel open and sends a
    /// keepalive every 10 s; an unanswered one makes it drop the connection. The session must keep answering
    /// and keep its signed-in phase, so the player can create another character and log in on it.
    /// </summary>
    public static void CharacterDeleteKeepsMasSession() => WithDirectory(directory => WithApplication(20231, () =>
    {
        var now = SlotNow.UtcDateTime;
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new PlainCommitter())
            { ProgressionCatalog = Load(directory, Catalog()), Clock = () => SlotNow };
        foreach (var name in new[] { "alpha!0", "alpha!1" }) Check(store.CreateForAccount(SlotRequest(name), "alpha").Result == MagNetResult.Success, name);
        const int timeout = 30;

        bool KeepAliveAnswered(MagMasSession session, DateTime at)
        {
            var replies = MasSend(session, new MagKeepAlive(), at);
            return replies.Count == 1 && replies[0].PluginId == 0 &&
                MagMessageCodec.TryParseForApplication(replies[0].Payload, 20231, out var echoed) && echoed is MagKeepAlive;
        }
        List<string> Listed()
        {
            var maps = new MagMapsSession { DevelopmentAuthEnabled = true, CharacterStorageEnabled = true, CharacterStore = store };
            MapsSend(maps, new MagHello(), now);
            var auth = new MagRetailNpAuthenticateRequest { TransactionId = 1, FullTicketSize = 5, TicketChunkSize = 5, LastMessage = true };
            Encoding.ASCII.GetBytes("alpha").CopyTo(auth.TicketChunk, 0);
            MapsSend(maps, auth, now);
            MapsSend(maps, new MagRetailUniverseListRequest(), now);
            MapsSend(maps, new MagNewsEulaRequest { Language = "english" }, now);
            var replies = MapsSend(maps, new MagCharacterListRequest { TransactionId = 9 }, now);
            MagMessage parsed = null;
            Check(replies.Count == 1 && MagMessageCodec.TryParseForApplication(replies[0].Payload, 20231, out parsed) &&
                parsed is MagRetailCharacterListResponse { Success: true }, "Character list after the delete: " + maps.StorageStatus);
            return ((MagRetailCharacterListResponse)parsed).Characters.Where(c => c.AccountId != 0).Select(c => c.Name).ToList();
        }

        var registry = new MagLoginRegistry();
        var mas = new MagMasSession { DevelopmentAuthEnabled = true, AccountLoginEnabled = true, CharacterDataEnabled = true,
            CharacterStorageEnabled = true, CharacterStore = store, LoginRegistry = registry };
        Check(MasSend(mas, new MagKeepAlive(), now).Count == 0, "A keepalive before the hello was answered");
        MasSend(mas, new MagHello(), now);
        MasSend(mas, new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, now);
        var signIn = new MagRetailNpAuthenticateRequest { TransactionId = 1, FullTicketSize = 5, TicketChunkSize = 5, LastMessage = true };
        Encoding.ASCII.GetBytes("alpha").CopyTo(signIn.TicketChunk, 0);
        Check(MasSend(mas, signIn, now).Count == 1 && mas.State == MagMasSession.SessionState.LocalDevelopmentAuthenticated, "MAS sign-in");

        // Delete, then the client's keepalives at 10 s intervals: each is echoed and the phase stays signed in.
        uint deletedId = IdOf(store, "alpha", "alpha!1");
        var deleted = MasSend(mas, new MagDeleteCharacterRequest { TransactionId = 21, CharacterName = "alpha!1", CharacterId = deletedId }, now);
        Check(deleted.Count == 1 && MagMessageCodec.TryParseForApplication(deleted[0].Payload, 20231, out var deleteReply) &&
            deleteReply is MagDeleteCharacterResponse { Success: true, TransactionId: 21 }, "Delete: " + mas.StorageStatus);
        for (int seconds = 10; seconds <= 60; seconds += 10)
        {
            var at = now.AddSeconds(seconds);
            Check(KeepAliveAnswered(mas, at), $"The keepalive {seconds} s after a delete went unanswered");
            Check(mas.State == MagMasSession.SessionState.LocalDevelopmentAuthenticated && mas.AuthenticatedAccount == "alpha",
                "A keepalive after a delete changed the MAS session: " + mas.State);
            Check(!mas.IsExpired(at.AddSeconds(timeout - 1), timeout) && mas.IsPortalSessionLive(at.AddSeconds(timeout - 1)),
                "MAS would close a session whose keepalives are answered");
        }
        Check(mas.IsExpired(now.AddSeconds(60 + timeout + 1), timeout), "A silent MAS session never expires");
        var later = now.AddSeconds(60);

        // A fresh list (the client reads it over MAPS) no longer shows the deleted character.
        Check(Listed().SequenceEqual(new[] { "alpha!0" }), "The list after a delete still shows it: " + string.Join(",", Listed()));

        // Re-create in the freed slot and log in on it, all in the same MAS session.
        var created = MasSend(mas, SlotRequest("alpha!1"), later);
        Check(created.Count == 1 && MagMessageCodec.TryParseForApplication(created[0].Payload, 20231, out var createReply) &&
            createReply is MagCreateCharacterResponse { Result: MagNetResult.Success } && mas.State == MagMasSession.SessionState.CharacterCreated,
            "Re-create after a delete in the same MAS session: " + mas.StorageStatus);
        Check(KeepAliveAnswered(mas, later.AddSeconds(5)) && mas.State == MagMasSession.SessionState.CharacterCreated,
            "A keepalive after the re-create went unanswered or changed the phase");
        uint recreatedId = IdOf(store, "alpha", "alpha!1");
        Check(recreatedId != deletedId, "The re-created character reused the deleted one's id");
        var login = MasSend(mas, new MagRetailAccountLoginRequest { TransactionId = 2, VersionMajor = 3, VersionMinor = 3, VersionBuild = 1, AccountName = "alpha!1" }, later);
        Check(login.Count == 1 && MagMessageCodec.TryParseForApplication(login[0].Payload, 20231, out var loginReply) &&
            loginReply is MagRetailAccountLoginResponse { Result: 1 } granted && granted.AccountId == recreatedId &&
            mas.State == MagMasSession.SessionState.AccountLoginSent, "Login on the re-created character: " + mas.StorageStatus);
        var card = MasSend(mas, new MagCharacterDataRequest { TransactionId = 3, QueryKind = 2, AccountId = recreatedId, RequestId = 1 }, later);
        Check(card.Count == 1 && MagMessageCodec.TryParseForApplication(card[0].Payload, 20231, out var data) &&
            data is MagCharacterDataResponse { Success: true } served && served.Record.Name == "alpha!1" &&
            mas.State == MagMasSession.SessionState.CharacterDataSent, "Character data for the re-created character: " + mas.StorageStatus);
        Check(KeepAliveAnswered(mas, later.AddSeconds(10)), "A keepalive after the login went unanswered");
        Check(Listed().SequenceEqual(new[] { "alpha!0", "alpha!1" }), "The list after the re-create: " + string.Join(",", Listed()));
    }));

    static MagAdminApiRequest SignedSlot(string method, string path, DateTime now, string body = null)
    {
        byte[] secret = Encoding.UTF8.GetBytes("synthetic-admin-api-secret-0123456789abcdef");
        byte[] bytes = body == null ? Array.Empty<byte>() : Encoding.UTF8.GetBytes(body);
        string ts = new DateTimeOffset(now).ToUnixTimeSeconds().ToString(), nonce = "synthetic-slot-nonce-" + Guid.NewGuid().ToString("N");
        return new MagAdminApiRequest
        {
            Method = method, PathAndQuery = path, Body = bytes,
            Headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
            {
                [MagAdminApiAuthenticator.TimestampHeader] = ts, [MagAdminApiAuthenticator.NonceHeader] = nonce,
                [MagAdminApiAuthenticator.ActorHeader] = "staff@example.invalid",
                [MagAdminApiAuthenticator.SignatureHeader] = MagAdminApiAuthenticator.Sign(secret, method, path, ts, nonce, bytes),
            }
        };
    }

    public static void CharacterSlotAdmin() => WithDirectory(directory =>
    {
        var now = SlotNow.UtcDateTime;
        string catalogPath = Path.Combine(directory, "admin-progression.json"); AdminCliChecks.Catalog(catalogPath);
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new PlainCommitter())
            { ProgressionCatalog = MagProgressionCatalog.Load(catalogPath, 20231), Clock = () => SlotNow };
        string backups = Path.Combine(directory, ".maintenance-backups"), markers = Path.Combine(directory, "markers");
        Directory.CreateDirectory(markers);
        MagActiveAccounts.Write(Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json"), new Dictionary<string, DateTime>(), now);
        foreach (var name in new[] { "alpha!0", "alpha!1" }) Check(store.CreateForAccount(SlotRequest(name), "alpha").Result == MagNetResult.Success, name);
        Check(store.CreateForAccount(SlotRequest("Solo"), "solo").Result == MagNetResult.Success, "Solo");
        (int Code, string Output, string Error) Admin(params string[] args)
        {
            var output = new StringWriter(); var error = new StringWriter();
            int code = MagAdminCli.Run(args, store, backups, markers, now, output, error);
            return (code, output.ToString(), error.ToString());
        }
        int Level(string name) => store.ReadAll().Single(c => c.Name == name).ToRecord().Level;

        var listed = Admin("accounts", "list");
        Check(listed.Code == 0 && listed.Output.Contains("SLOT") && listed.Output.Split('\n').Count(l => l.StartsWith("alpha ")) == 2, "List: " + listed.Output);
        var shown = Admin("character", "show", "alpha", "--slot", "1");
        Check(shown.Code == 0 && shown.Output.Contains("alpha!1 (faction 0, slot 1)") && shown.Output.Contains("slot 0") && shown.Output.Contains("alpha!0"),
            "Show by slot lists the account's characters: " + shown.Output);
        var ambiguous = Admin("character", "set-level", "alpha", "3", "--apply");
        Check(ambiguous.Code == 1 && ambiguous.Error.Contains("ambiguous-character") && ambiguous.Error.Contains("slot 1 alpha!1"), "Ambiguous: " + ambiguous.Error);
        Check(Admin("character", "set-level", "alpha", "3", "--slot", "1", "--apply").Code == 0 && Level("alpha!1") == 3 && Level("alpha!0") == 1,
            "set-level by slot raises only that character");
        Check(Admin("character", "set-level", "ALPHA!0", "2", "--apply").Code == 0 && Level("alpha!0") == 2, "set-level by character name");
        Check(Admin("character", "set-level", "solo", "2", "--apply").Code == 0 && Level("Solo") == 2, "An account with one character as before");
        Check(Admin("character", "set-level", "alpha", "4", "--slot", "2").Error.Contains("no character in slot 2"), "An empty slot");
        Check(Admin("character", "set-level", "alpha", "4", "--slot", "3").Code == 2 && Admin("accounts", "list", "--slot", "1").Code == 2, "Bad --slot use");

        // A player deletion is shown, restored and purged through the CLI and the API.
        Check(store.DeleteCharacterForAccount("alpha", IdOf(store, "alpha", "alpha!1"), "alpha!1").Result == MagNetResult.Success, "Player deletion");
        Check(Admin("accounts", "list").Output.Contains("deleted 2026-11-27T12:00:00Z"), "List marks the deleted character");
        var dry = Admin("character", "restore", "alpha", "alpha!1");
        Check(dry.Code == 0 && dry.Output.Contains("dry run") && dry.Output.Contains("slot 1") && store.ReadForAccount("alpha").Count == 1, "Restore dry run: " + dry.Output);
        var restore = Admin("character", "restore", "alpha", "alpha!1", "--apply");
        Check(restore.Code == 0 && restore.Output.Contains("applied") && restore.Output.Contains("-> alpha!1") && Level("alpha!1") == 3, "Restore: " + restore.Output);
        Check(Admin("character", "purge", "alpha", "alpha!1", "--apply").Error.Contains("not_deleted"), "Purge refuses a live character");

        var api = new MagAdminApi(new System.Net.IPEndPoint(System.Net.IPAddress.Loopback, 0),
            Encoding.UTF8.GetBytes("synthetic-admin-api-secret-0123456789abcdef"), store, backups, markers, () => now, _ => { });
        JsonElement Json(MagAdminApiResponse r) => JsonDocument.Parse(r.Body).RootElement;
        var accounts = Json(api.Handle(SignedSlot("GET", "/admin/v1/accounts", now))).GetProperty("accounts");
        Check(accounts.GetArrayLength() == 3 && accounts[1].GetProperty("account").GetString() == "alpha" && accounts[1].GetProperty("slot").GetInt32() == 1 &&
            accounts[1].GetProperty("name").GetString() == "alpha!1" && !accounts[1].GetProperty("deleted").GetBoolean() &&
            accounts[1].GetProperty("deleted_at").ValueKind == JsonValueKind.Null && accounts[1].TryGetProperty("level", out _), "API lists every character");
        var detail = Json(api.Handle(SignedSlot("GET", "/admin/v1/characters/alpha?slot=1", now)));
        Check(detail.GetProperty("name").GetString() == "alpha!1" && detail.GetProperty("level").GetInt32() == 3 &&
            detail.GetProperty("characters").GetArrayLength() == 2, "API character by slot");
        Check(Json(api.Handle(SignedSlot("GET", "/admin/v1/characters/alpha", now))).GetProperty("slot").GetInt32() == 0, "Without a slot, the lowest");
        Check(api.Handle(SignedSlot("GET", "/admin/v1/characters/alpha?slot=2", now)).Status == 404, "An empty slot is 404");
        var unnamed = api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/level", now, "{\"level\":4,\"apply\":false}"));
        Check(unnamed.Status == 409 && Json(unnamed).GetProperty("error").GetString() == "ambiguous_character", "API level needs a slot");
        var raised = api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/level", now, "{\"level\":4,\"apply\":true,\"slot\":1}"));
        Check(raised.Status == 200 && Json(raised).GetProperty("slot").GetInt32() == 1 && Level("alpha!1") == 4 && Level("alpha!0") == 2, "API level by slot");
        Check(api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/level", now, "{\"level\":4,\"apply\":true,\"slot\":7}")).Status == 400, "API bad slot");
        Check(api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha!0/level", now, "{\"level\":4,\"apply\":false}")).Status == 404,
            "The API path takes an account, not a character name");
        Check(api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/kill", now, "{\"apply\":false}")).Status == 409, "API kill needs a slot too");
        Check(store.DeleteCharacterForAccount("alpha", IdOf(store, "alpha", "alpha!0"), "alpha!0").Result == MagNetResult.Success, "Second deletion");
        var purgeDry = api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/purge", now, "{\"name\":\"alpha!0\",\"apply\":false}"));
        Check(purgeDry.Status == 200 && !Json(purgeDry).GetProperty("applied").GetBoolean() && store.ReadEveryCharacter().Count == 3, "API purge dry run");
        Check(api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/restore", now, "{\"name\":\"nobody\",\"apply\":true}")).Status == 404, "API restore unknown");
        Check(api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/restore", now, "{\"apply\":true}")).Status == 400, "API restore needs a name");
        var restoredApi = api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/restore", now, "{\"name\":\"alpha!0\",\"apply\":true}"));
        Check(restoredApi.Status == 200 && Json(restoredApi).GetProperty("name").GetString() == "alpha!0" && Json(restoredApi).GetProperty("slot").GetInt32() == 0,
            "API restore");
        Check(store.DeleteCharacterForAccount("alpha", IdOf(store, "alpha", "alpha!0"), "alpha!0").Result == MagNetResult.Success, "Third deletion");
        var purged = api.Handle(SignedSlot("POST", "/admin/v1/characters/alpha/purge", now, "{\"name\":\"alpha!0\",\"apply\":true}"));
        Check(purged.Status == 200 && Json(purged).GetProperty("applied").GetBoolean() && store.ReadEveryCharacter().Count == 2, "API purge");
    });

    /// <summary>The slot rules give the same results on PostgreSQL as on the JSON store, and leave equal stores.</summary>
    public static void PostgresCharacterSlots() => WithDirectory(directory => WithPostgres("postgres character slots match the JSON store", (database, connection) =>
    {
        var catalog = Load(directory, Catalog());
        var json = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new PlainCommitter()) { ProgressionCatalog = catalog, Clock = () => SlotNow };
        var pg = LocalCharacterStore.ForPostgres(database);
        pg.ProgressionCatalog = catalog; pg.Clock = () => SlotNow;
        var expected = RunSlotScript(json, Path.Combine(directory, "json-backups"));
        var actual = RunSlotScript(pg, Path.Combine(directory, "pg-backups"));
        Check(expected.SequenceEqual(actual), "Backends disagree: json " + string.Join(" / ", expected) + " postgres " + string.Join(" / ", actual));
        var differences = pg.CompareWith(json);
        Check(differences.Count == 0 && pg.Summarize() == json.Summarize(), "Stores differ: " + string.Join("; ", differences));
        // The script deletes four characters (alpha!2, Plain, the replacement alpha!2 and bravo!0), restores two and purges two.
        long deletes = Scalar(connection, "SELECT count(*) FROM mag.admin_changes WHERE operation = 'character-delete' AND character_id IS NOT NULL");
        long purges = Scalar(connection, "SELECT count(*) FROM mag.admin_changes WHERE operation = 'character-purge' AND after = '{}'::jsonb");
        long restores = Scalar(connection, "SELECT count(*) FROM mag.admin_changes WHERE operation = 'character-restore'");
        Check(deletes == 4 && purges == 2 && restores == 2,
            $"Deletes, restores and purges are journalled: {deletes} deletes, {restores} restores, {purges} purges");
        Check(Scalar(connection, "SELECT count(*) FROM mag.match_progress p LEFT JOIN mag.characters c ON c.id = p.character_id WHERE c.id IS NULL") == 0,
            "A purge leaves no ledger rows behind");

        // The schema refuses what the store refuses: two live characters in a slot, a fourth slot, an original name on a live row.
        foreach (var sql in new[] {
            "UPDATE mag.characters SET slot = 0 WHERE account = 'alpha' AND deleted_at IS NULL",
            "UPDATE mag.characters SET slot = 3 WHERE account = 'alpha' AND slot = 2",
            "UPDATE mag.characters SET original_name = 'x' WHERE deleted_at IS NULL" })
        {
            bool refused = false;
            try { Sql(connection, sql); } catch (PostgresException) { refused = true; }
            Check(refused, "The schema accepted: " + sql);
        }

        // Export and import carry slots, deletions and moved-aside names.
        string exported = Path.Combine(directory, "exported.json");
        var export = pg.ExportTo(new LocalCharacterStore(exported, new PlainCommitter()) { ProgressionCatalog = catalog }, false);
        Check(export.Comparison.Differences.Count == 0, "Export: " + string.Join("; ", export.Comparison.Differences));
        Sql(connection, "DELETE FROM mag.characters; DELETE FROM mag.voice_sanctions");
        pg.ImportFrom(new LocalCharacterStore(exported, new PlainCommitter()) { ProgressionCatalog = catalog });
        Check(pg.CompareWith(json).Count == 0, "Import round trip");

        // Concurrent creations on one account: three succeed in distinct slots, the rest are refused.
        var outcomes = Enumerable.Range(0, 6).AsParallel().WithDegreeOfParallelism(6)
            .Select(i => pg.CreateForAccount(SlotRequest("Racer" + i), "racer")).ToList();
        Check(outcomes.Count(o => o.Result == MagNetResult.Success) == 3 && outcomes.Count(o => o.Reason == "character-slots-full") == 3 &&
            pg.ReadForAccount("racer").Select(c => c.Slot).OrderBy(s => s).SequenceEqual(new byte[] { 0, 1, 2 }),
            "Concurrent creations: " + string.Join(",", outcomes.Select(o => o.Reason)));
    }));

    /// <summary>A schema-2 database (one character per account, live since Medius 0.14.0) upgrades in place:
    /// every character becomes slot 0 with its progression, kits and sanctions, and gains two more slots.</summary>
    public static void PostgresCharacterSlotUpgrade() => WithDirectory(directory => WithPostgres("postgres character slots upgrade a schema-2 database", (database, connection) =>
    {
        Sql(connection, "DROP SCHEMA mag CASCADE");
        Check(database.Migrate(2) == 2 && database.SchemaVersion() == 2, "Synthetic schema-2 database");
        Sql(connection, SchemaOneCharacters);
        Sql(connection, "UPDATE mag.characters SET experience = 7, level = 2 WHERE account = 'alpha'; " +
            "INSERT INTO mag.character_loadouts SELECT 1, s, 1, array_fill(-1::smallint, ARRAY[22]) FROM generate_series(0, 4) s; " +
            "INSERT INTO mag.match_progress VALUES (1, 'old-match', 7, 0, NULL, true, 'match_major_victory', now(), now()); " +
            "INSERT INTO mag.voice_sanctions VALUES ('bravo', 'mute', NULL, 'staff', now(), NULL)");
        bool refused = false;
        try { Sql(connection, "INSERT INTO mag.characters (id, account, name, faction, head_id, voice_id, universe_id, np_id) " +
            "VALUES (3, 'alpha', 'Second', 0, 0, 0, 26, decode(repeat('00', 36), 'hex'))"); }
        catch (PostgresException) { refused = true; }
        Check(refused, "Schema 2 allows one character per account");

        Check(database.Migrate() == 1 && database.SchemaVersion() == database.LatestSchemaVersion, "Migration 0003 applies");
        var pg = LocalCharacterStore.ForPostgres(database);
        pg.ProgressionCatalog = Load(directory, Catalog()); pg.Clock = () => SlotNow;
        var alpha = pg.ReadForAccount("alpha").Single();
        Check(alpha.Slot == 0 && !alpha.Deleted && alpha.Name == "Alpha" && alpha.ToRecord().Level == 2 && alpha.Loadouts?.Length == 5,
            "The existing character is slot 0 with its progression and kits");
        Check(Scalar(connection, "SELECT count(*) FROM mag.match_progress WHERE character_id = 1") == 1, "Match history kept");
        Check(pg.ReadVoiceSanctions().Single().Account == "bravo", "Sanctions kept");
        Check(pg.CreateForAccount(SlotRequest("alpha!1"), "alpha").Result == MagNetResult.Success &&
            pg.CreateForAccount(SlotRequest("Third"), "alpha").Result == MagNetResult.Success &&
            pg.CreateForAccount(SlotRequest("Fourth"), "alpha").Reason == "character-slots-full", "Two more slots after the upgrade");
        Check(pg.ReadForAccount("alpha").Select(c => c.Slot).SequenceEqual(new byte[] { 0, 1, 2 }) && pg.ReadForAccount("alpha").Last().Id == 4,
            "New characters take the free slots and fresh ids");
        // A purge of an account's last character no longer cascades to its sanction.
        Check(pg.DeleteCharacterForAccount("bravo", 2, "Bravo").Result == MagNetResult.Success &&
            pg.PurgeCharacterForAdmin("bravo", "Bravo", true, null).Applied && pg.ReadVoiceSanctions().Count == 1, "The sanction outlives the purge");
    }));
}
