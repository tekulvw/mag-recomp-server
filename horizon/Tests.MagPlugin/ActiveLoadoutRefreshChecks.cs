using System.Text;
using System.Text.Json;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Medius;

internal static class ActiveLoadoutRefreshChecks
{
    static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
    static GameplayLoadoutRefreshTarget Target(string reservation = "reservation00001") => new(reservation, 7, 2, "synthetic", 1);
    static GameplayAdmissionLoadouts Snapshot(short primary = 1)
    {
        var record = new MagCharacterRecord { LastUsedLoadout = 0 };
        foreach (var row in record.Loadouts) { row.Items[0] = primary; row.UpdateId = 0; }
        return GameplayLoadoutAdmission.Project(new StoredCharacter { Loadouts = record.Loadouts.Select(StoredLoadout.FromLoadout).ToArray() }, record, null);
    }
    public static void Run()
    {
        Capability();
        QueueOrderingAndRetry();
        NativeSaveNotification();
    }
    static void Capability()
    {
        var old = Encoding.UTF8.GetBytes("""{"v":2,"request_id":"hello","ok":true,"error":null,"messages":[],"capabilities":["committed_loadout_selection_v1"]}""");
        Check(GameplayBridgeResponseCodec.TryParse(old, "hello", true, out var reply), "Legacy typed Hello parse");
        bool refused = false;
        try { LocalDmeHost.RequireLoadoutSelectionCapability(reply); }
        catch (IOException) { refused = true; }
        Check(refused, "Gameplay without active_loadout_refresh_v1 must be refused before admission");
        var current = Encoding.UTF8.GetBytes("""{"v":2,"request_id":"hello","ok":true,"error":null,"messages":[],"capabilities":["committed_loadout_selection_v1","active_loadout_refresh_v1"]}""");
        Check(GameplayBridgeResponseCodec.TryParse(current, "hello", true, out reply) && reply.SupportsActiveLoadoutRefresh, "Refresh capability retained");
        LocalDmeHost.RequireLoadoutSelectionCapability(reply);
    }
    static void QueueOrderingAndRetry()
    {
        var gate = new object(); var dropped = new List<string>(); var now = DateTime.UtcNow; bool current = true;
        var target = Target(); var sourceRecord = new MagCharacterRecord { LastUsedLoadout = 0 };
        foreach (var row in sourceRecord.Loadouts) { row.Items[0] = 1; row.UpdateId = 0; }
        var canonical = GameplayLoadoutAdmission.Project(new StoredCharacter { Loadouts = sourceRecord.Loadouts.Select(StoredLoadout.FromLoadout).ToArray() }, sourceRecord, null);
        var queue = new GameplayLoadoutRefreshQueue(gate, (_, _, reason) => dropped.Add(reason));
        bool Current(GameplayLoadoutRefreshTarget owner) => current && owner.MatchesOwner(target.ReservationId, 7, 2, "synthetic", 1);
        Check(queue.Schedule(target, now, Current, _ => canonical), "Canonical refresh not scheduled");
        var first = queue.TakeDue(now, Current, _ => throw new Exception("Snapshot should be frozen"));
        string original = JsonSerializer.Serialize(first);
        sourceRecord.Loadouts[0].Items[0] = 2; sourceRecord.Loadouts[0].UpdateId = 99;
        canonical.UpdateIds[0] = 99; canonical.Inventories[0][0] = 99;
        first.Loadouts.UpdateIds[0] = 99; first.Loadouts.Inventories[0][0] = 99;
        Check(JsonSerializer.Serialize(first) == original, "Caller mutated a same-revision retry payload");
        using (var json = JsonDocument.Parse(original))
        {
            var root = json.RootElement;
            Check(root.EnumerateObject().Select(p => p.Name).Order().SequenceEqual(new[] {
                "v", "op", "request_id", "reservation_id", "world_id", "player_id", "account_id", "character_id", "revision", "loadouts" }.Order()), "Refresh request schema changed");
            Check(root.GetProperty("revision").GetUInt64() == 1 && root.GetProperty("op").GetString() == "refresh_loadouts" &&
                root.GetProperty("loadouts").GetProperty("update_ids")[0].GetByte() == 0, "Refresh revision or native byte-zero update ID changed");
        }
        queue.Complete(first, now, false, true, Current);
        Check(queue.TakeDue(now.AddMilliseconds(500), Current, _ => canonical) == null, "Refresh backoff ignored");
        var retry = queue.TakeDue(now.AddSeconds(1), Current, _ => canonical);
        Check(ReferenceEquals(first, retry) && JsonSerializer.Serialize(retry) == original, "Uncertain reply changed retry revision/snapshot");
        queue.Complete(retry, now.AddSeconds(1), false, true, Current);
        retry = queue.TakeDue(now.AddSeconds(2), Current, _ => canonical);
        queue.Complete(retry, now.AddSeconds(2), false, true, Current);
        Check(!queue.HasDue(now.AddSeconds(9)) && dropped.Contains("refresh_send_exhausted"), "Uncertain RPC retries never exhausted visibly");
        Check(queue.Schedule(target, now.AddSeconds(3), Current, _ => Snapshot(2)), "New save did not reset exhausted refresh");
        var second = queue.TakeDue(now.AddSeconds(3), Current, _ => canonical);
        Check(second.Revision == 2 && second.Loadouts.Inventories[0][0] == 2, "New save did not coalesce canonical latest snapshot");
        Check(queue.Schedule(target, now.AddSeconds(3), Current, _ => Snapshot(1)), "Newer in-flight save not scheduled");
        queue.Complete(second, now.AddSeconds(3), true, false, Current);
        var third = queue.TakeDue(now.AddSeconds(3), Current, _ => canonical);
        Check(third?.Revision == 3, "Old reply cleared newer pending refresh");
        current = false;
        Check(!queue.StillCurrent(third, Current), "Released target could dispatch an old refresh");
        queue.Forget(target.ReservationId); queue.Complete(third, now, true, false, Current);
        target = Target("reservation00002"); current = true;
        Check(queue.Schedule(target, now, Current, _ => Snapshot()), "Requeue refresh rejected");
        var requeue = queue.TakeDue(now, Current, _ => canonical);
        Check(requeue?.Revision == 1, "Fresh reservation inherited old refresh revision");
        queue.Complete(third, now, true, false, Current);
        Check(queue.StillCurrent(requeue, Current), "Old reservation reply cleared new owner refresh");
        queue.Complete(requeue, now, false, false, Current);
        Check(dropped.Contains("refresh_rejected") && !queue.HasDue(now.AddSeconds(10)), "Permanent Rust refusal retried indefinitely");
        int reads = 0;
        GameplayAdmissionLoadouts ReadFailure(GameplayLoadoutRefreshTarget _) { reads++; throw new IOException("synthetic read"); }
        Check(queue.Schedule(target, now, Current, ReadFailure), "Transient read not queued");
        Check(queue.TakeDue(now.AddMilliseconds(100), Current, ReadFailure) == null && reads == 1, "Store read backoff ignored");
        Check(queue.TakeDue(now.AddSeconds(1), Current, ReadFailure) == null && reads == 2, "Second store read missing");
        Check(queue.TakeDue(now.AddSeconds(2), Current, ReadFailure) == null && reads == 3 && dropped.Contains("store_read_exhausted"), "Store reads not bounded/visible");
        Check(!queue.Schedule(target, now, Current, _ => throw new InvalidDataException("synthetic malformed store")) && dropped.Contains("invalid_store_projection"), "Malformed projection retried");
        Check(!queue.Schedule(target, now, Current, _ => throw new InvalidOperationException("synthetic invalid read")) &&
            dropped.Contains("unexpected_projection_InvalidOperationException") && !queue.HasDue(now.AddSeconds(10)), "Unexpected projection error remained silently pending");
        current = false; int unownedReads = 0;
        Check(!queue.Schedule(target, now, Current, _ => { unownedReads++; return canonical; }) && unownedReads == 0, "Non-current owner read a store snapshot");
    }
    sealed class CountingCommitter : ICharacterStoreCommitter
    {
        public int Writes;
        public bool Fail;
        public void WriteAtomically(string path, byte[] contents)
        {
            Writes++;
            if (Fail) throw new IOException("synthetic write failure");
            new AtomicCharacterStoreCommitter().WriteAtomically(path, contents);
        }
    }
    static RT_MSG_CLIENT_APP_TO_PLUGIN Envelope(MagMessage message, int application = 20230)
    {
        var payload = MagMessageCodec.SerializeForApplication(message, application);
        using var buffer = new MemoryStream();
        using (var writer = new BinaryWriter(buffer, Encoding.UTF8, true)) { writer.Write((ushort)payload.Length); writer.Write((byte)0); writer.Write(payload); }
        buffer.Position = 0; using var reader = new MessageReader(buffer);
        var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN(); envelope.Deserialize(reader);
        Check(envelope.IsValid, "Synthetic native envelope rejected"); return envelope;
    }
    static MagMlsSession Session(LocalCharacterStore store, string account, DateTime now)
    {
        var registry = new MagLoginRegistry(); var ticket = registry.IssueForAccount(account, 1, 1, now);
        var session = new MagMlsSession { CharacterStore = store };
        Check(session.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20230, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey }, registry, "synthetic-channel", now), "Synthetic authenticated MLS bind");
        Check(session.Accept(now) && session.CompleteTransport(now), "Synthetic MLS transport");
        session.HandleMany(Envelope(new MagHello()), now);
        session.HandleMany(Envelope(new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }), now);
        Check(session.State == MagMlsSession.SessionState.Ready, "Synthetic MLS protocol handshake"); return session;
    }
    static bool Save(MagMlsSession session, short primary, DateTime now)
    {
        var loadout = new MagLoadout { UpdateId = 0 }; loadout.Items[0] = primary;
        var replies = session.HandleMany(Envelope(new MagLoadoutUpdateRequest { LoadoutIndex = 1, Loadout = loadout }), now);
        Check(replies.Count == 1, "Native save reply count");
        Check(MagMessageCodec.TryParseForApplication(replies[0].Payload, 20230, out var parsed), "Native save reply parse");
        return ((MagLoadoutUpdateResponse)parsed).Success;
    }
    static void NativeSaveNotification()
    {
        var previousApplication = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20230;
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-active-loadouts-" + Guid.NewGuid()); Directory.CreateDirectory(directory);
        try
        {
            var now = DateTime.UtcNow; string path = Path.Combine(directory, "characters.json"), items = Path.Combine(directory, "items.json"), defaults = Path.Combine(directory, "defaults.json");
            File.WriteAllText(items, """{"schema_version":1,"application_id":20230,"definitions":{"weapons":{"One":{"parameters":{"magazine_capacity":1}},"Two":{"parameters":{"magazine_capacity":2}}}},"items":{"1":{"item_id":1,"name":"One","item_type":"kWeapon","definition":{"group":"weapons","name":"One"}},"2":{"item_id":2,"name":"Two","item_type":"kWeapon","definition":{"group":"weapons","name":"Two"}}}}""");
            var row = new MagLoadout(); row.Items[0] = 1;
            File.WriteAllText(defaults, JsonSerializer.Serialize(new { application_id = 20230, factions = new[] { new { faction = 0,
                loadouts = Enumerable.Range(0, 5).Select(_ => new { update_id = 1, items = row.Items }).ToArray() } } }));
            var committer = new CountingCommitter(); var store = new LocalCharacterStore(path, committer) { LoadoutCatalog = MagLoadoutCatalog.Load(items, defaults) };
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Example", Faction = 0, UniverseId = 26 }, "synthetic").Result == MagNetResult.Success, "Synthetic native-save store setup");
            var session = Session(store, "synthetic", now); var target = Target(); var gate = new object(); bool accepted = true; int callbacks = 0;
            var queue = new GameplayLoadoutRefreshQueue(gate);
            bool Current(GameplayLoadoutRefreshTarget owner) => accepted && owner.MatchesOwner(target.ReservationId, 7, 2, "synthetic", 1);
            GameplayAdmissionLoadouts Project(GameplayLoadoutRefreshTarget owner)
            {
                var character = store.ReadForAccount(owner.AccountId).Single(c => c.Id == owner.CharacterId);
                return GameplayLoadoutAdmission.Project(character, character.ToRecord(), store.LoadoutCatalog);
            }
            session.LoadoutCommitted = (account, character) => {
                callbacks++; Check(account == "synthetic" && character == 1, "Callback lost authenticated owner");
                queue.Schedule(target, now, Current, Project);
            };
            committer.Writes = 0;
            Check(Save(session, 2, now) && callbacks == 1 && committer.Writes == 1, "Accepted durable native save did not notify exactly once");
            var refresh = queue.TakeDue(now, Current, Project);
            Check(refresh?.Loadouts.Inventories[1][0] == 2 && refresh.Loadouts.SelectedIndex == 0 && refresh.Loadouts.UpdateIds[1] == 0,
                "Callback did not read canonical stored inventory or changed selected preference");
            Check(!Save(session, 99, now) && callbacks == 1 && session.LoadoutRefreshStatus == "not-scheduled", "Rejected native save notified refresh or retained stale diagnostic status");
            committer.Fail = true;
            Check(!Save(session, 1, now) && callbacks == 1 && session.LoadoutRefreshStatus == "not-scheduled", "Failed durable write notified refresh or retained stale diagnostic status");
            committer.Fail = false;
            var unowned = Session(store, "other", now); unowned.LoadoutCommitted = (_, _) => callbacks++;
            Check(!Save(unowned, 1, now) && callbacks == 1, "Unowned native save notified refresh");
            session.LoadoutCommitted = (_, _) => throw new IOException("synthetic refresh callback failure");
            Check(Save(session, 1, now) && session.StorageStatus == "loadout-committed" && session.LoadoutRefreshStatus == "notification-failed-after-commit",
                "Refresh callback failure undid a durable native success");
            // A save can commit while reserve is in flight, before an active row is
            // accepted (or even registered). Post-admission reconciliation must read it.
            accepted = false; queue.Forget(target.ReservationId);
            var beforeAdmission = Project(target);
            session.LoadoutCommitted = (_, _) => queue.Schedule(target, now, Current, Project);
            Check(Save(session, 2, now) && !queue.HasDue(now), "Unaccepted reservation dispatched a refresh");
            accepted = true;
            Check(queue.Schedule(target, now, Current, Project), "Post-admission canonical reconciliation missing");
            var admitted = queue.TakeDue(now, Current, Project);
            Check(beforeAdmission.Inventories[1][0] == 1 && admitted.Loadouts.Inventories[1][0] == 2,
                "Save in canonical-read/reserve window was lost");
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previousApplication; Directory.Delete(directory, true); }
    }
}
