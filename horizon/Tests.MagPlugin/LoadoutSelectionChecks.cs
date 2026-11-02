using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using RT.Models.MAG;
using Server.Medius;

internal static class LoadoutSelectionChecks
{
    static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
    static JsonObject Action(ulong revision = 1, byte index = 2, string selection = "selection00000001", string reservation = "reservation00001") => new() {
        ["selection_id"] = selection, ["reservation_id"] = reservation, ["world_id"] = 7, ["player_id"] = 2,
        ["account_id"] = "synthetic", ["character_id"] = 1, ["revision"] = revision, ["inventory_index"] = index
    };
    static JsonObject Reply(JsonArray actions = null) => new() {
        ["v"] = 2, ["request_id"] = "request", ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray(),
        ["capabilities"] = new JsonArray("committed_loadout_selection_v1", "active_loadout_refresh_v1"), ["loadout_selections"] = actions ?? new JsonArray(Action())
    };
    static bool Parse(JsonObject reply, out GameplayBridgeResponse response) => GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(reply.ToJsonString()), "request", true, out response);
    public static void Run()
    {
        Codec();
        Settlement();
        StoreAndReleaseOrdering();
    }
    static GameplayLoadoutSelection Selection(ulong revision = 1, byte index = 2, string id = "selection00000001", string reservation = "reservation00001")
    {
        Check(Parse(Reply(new JsonArray(Action(revision, index, id, reservation))), out var response), "Selection fixture parse");
        return response.LoadoutSelections.Single();
    }
    static void Codec()
    {
        Check(Parse(Reply(), out var parsed) && parsed.LoadoutSelections.Count == 1 && parsed.SupportsCommittedLoadoutSelections,
            "Committed loadout action and capability must parse as typed v2 data");
        var selection = parsed.LoadoutSelections.Single();
        Check(selection.AccountId == "synthetic" && selection.CharacterId == 1 && selection.WorldId == 7 && selection.PlayerId == 2 &&
            selection.Revision == 1 && selection.InventoryIndex == 2, "Typed action fields lost");
        LocalDmeHost.RequireLoadoutSelectionCapability(parsed);
        Check(selection.MatchesOwner("reservation00001", 7, 2, "synthetic", 1), "Exact owner rejected");
        Check(!selection.MatchesOwner("reservation00002", 7, 2, "synthetic", 1) &&
            !selection.MatchesOwner("reservation00001", 8, 2, "synthetic", 1) &&
            !selection.MatchesOwner("reservation00001", 7, 3, "synthetic", 1) &&
            !selection.MatchesOwner("reservation00001", 7, 2, "Synthetic", 1) &&
            !selection.MatchesOwner("reservation00001", 7, 2, "synthetic", 2), "Owner identity mismatch admitted");
        foreach (var field in new[] { "selection_id", "reservation_id", "world_id", "player_id", "account_id", "character_id", "revision", "inventory_index" })
        {
            var bad = Reply(); bad["loadout_selections"][0].AsObject().Remove(field);
            Check(!Parse(bad, out _), "Missing action field admitted: " + field);
        }
        foreach (var pair in new[] { ("inventory_index", "5"), ("revision", "0"), ("revision", "-1"), ("revision", "18446744073709551616"),
            ("character_id", "0"), ("account_id", "\"Synthetic\""), ("player_id", "65536"), ("world_id", "-1") })
        {
            var bad = Reply(); bad["loadout_selections"][0][pair.Item1] = JsonNode.Parse(pair.Item2);
            Check(!Parse(bad, out _), "Invalid action field admitted: " + pair.Item1);
        }
        var extra = Reply(); extra["loadout_selections"][0]["reason"] = "no";
        Check(!Parse(extra, out _), "Unknown action property admitted");
        var duplicate = Reply(new JsonArray(Action(), Action()));
        Check(!Parse(duplicate, out _), "Duplicate action admitted");
        var oversized = Reply(new JsonArray(Enumerable.Range(0, 33).Select(n => (JsonNode)Action((ulong)n + 1, 0, "selection" + n, "reservation000" + n)).ToArray()));
        Check(!Parse(oversized, out _), "Unbounded action batch admitted");
        var denied = Reply(); denied["ok"] = false;
        Check(!Parse(denied, out _), "Uncommitted/error reply admitted action");
        var old = Reply(); old.Remove("capabilities"); old.Remove("loadout_selections");
        Check(Parse(old, out var legacy), "Legacy parse fixture must remain usable");
        try { LocalDmeHost.RequireLoadoutSelectionCapability(legacy); throw new Exception("Missing capability admitted startup"); }
        catch (IOException) { }
        var caps = Reply(); caps["capabilities"] = new JsonArray("committed_loadout_selection_v1", "committed_loadout_selection_v1");
        Check(!Parse(caps, out _), "Duplicate capabilities admitted");
        caps["capabilities"] = new JsonArray(Enumerable.Range(0, 17).Select(n => (JsonNode)JsonValue.Create("cap_" + n)).ToArray());
        Check(!Parse(caps, out _), "Unbounded capabilities admitted");
        caps["capabilities"] = new JsonArray("future_capability");
        Check(Parse(caps, out var future) && future.Capabilities.Single() == "future_capability" && !future.SupportsCommittedLoadoutSelections,
            "Bounded capability retention changed");
    }
    static void Settlement()
    {
        var gate = new object(); var policy = new GameplayLoadoutSelectionSettlement(gate);
        var now = DateTime.UtcNow; int writes = 0; bool current = true;
        CharacterCreationOutcome Save(GameplayLoadoutSelection _) { writes++; return new CharacterCreationOutcome { Result = MagNetResult.Success }; }
        Check(policy.Settle(Selection(), now, _ => current, Save) == "persisted" && writes == 1, "Committed action not saved");
        Check(policy.Settle(Selection(), now, _ => current, Save) == "persisted" && writes == 1, "Uncertain ACK rewrote saved action");
        Check(policy.Settle(Selection(1, 3), now, _ => current, Save) == "rejected" && writes == 1, "Same ID/revision changed payload");
        Check(policy.Settle(Selection(2, 3, "selection00000002"), now, _ => current, Save) == "persisted" && writes == 2, "New revision not saved");
        Check(policy.Settle(Selection(), now, _ => current, Save) == "rejected" && writes == 2, "Old revision overwrote latest");
        current = false;
        Check(policy.Settle(Selection(3, 4, "selection00000003"), now, _ => current, Save) == "rejected" && writes == 2, "Released owner wrote store");
        current = true; int attempts = 0;
        CharacterCreationOutcome Fail(GameplayLoadoutSelection _) { attempts++; throw new IOException("synthetic transient"); }
        var transient = Selection(3, 1, "selection00000003");
        Check(policy.Settle(transient, now, _ => current, Fail) == null && attempts == 1, "First transient not deferred");
        Check(policy.Settle(transient, now.AddMilliseconds(500), _ => current, Fail) == null && attempts == 1, "Backoff not enforced");
        Check(policy.Settle(transient, now.AddSeconds(1), _ => current, Fail) == null && attempts == 2, "Second attempt missing");
        Check(policy.Settle(transient, now.AddSeconds(2), _ => current, Fail) == "rejected" && attempts == 3, "Transient retries were not bounded");
        Check(policy.Settle(transient, now.AddSeconds(3), _ => current, Fail) == "rejected" && attempts == 3, "Rejected receipt retried store");
        Check(policy.Settle(Selection(4, 2, "selection00000004"), now, _ => current, Save) == "persisted" && writes == 3, "New revision did not reset retries");
        Check(policy.Settle(Selection(5, 2, "selection00000005"), now, _ => current,
            _ => new CharacterCreationOutcome { Result = MagNetResult.Error }) == "rejected", "Permanent store error retained pending action");
        Check(policy.Settle(Selection(6, 2, "selection00000006"), now, _ => current,
            _ => throw new InvalidDataException("synthetic malformed store")) == "rejected", "Malformed store must reject permanently");
    }
    static void StoreAndReleaseOrdering()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-loadout-" + Guid.NewGuid()); Directory.CreateDirectory(directory);
        try
        {
            string items = Path.Combine(directory, "items.json"), defaults = Path.Combine(directory, "defaults.json"), path = Path.Combine(directory, "characters.json");
            File.WriteAllText(items, """{"schema_version":1,"application_id":20230,"definitions":{"weapons":{"Example":{"parameters":{"magazine_capacity":1}}}},"items":{"1":{"item_id":1,"name":"Example","item_type":"kWeapon","definition":{"group":"weapons","name":"Example"}}}}""");
            var primary = Enumerable.Repeat((short)-1, 22).ToArray(); primary[0] = 1;
            File.WriteAllText(defaults, JsonSerializer.Serialize(new { application_id = 20230, factions = new[] { new { faction = 0,
                loadouts = Enumerable.Range(0, 5).Select(i => new { update_id = 2, items = i == 4 ? Enumerable.Repeat((short)-1, 22).ToArray() : primary }).ToArray() } } }));
            var store = new LocalCharacterStore(path) { LoadoutCatalog = MagLoadoutCatalog.Load(items, defaults) };
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Example", Faction = 0, UniverseId = 26 }, "synthetic").Result == MagNetResult.Success, "Synthetic store setup");
            var initial = store.ReadForAccount("synthetic").Single();
            Check(initial.Loadouts == null, "Fresh store fixture must retain absent saved inventories");
            var admission = GameplayLoadoutAdmission.Project(initial, initial.ToRecord(), store.LoadoutCatalog);
            Check(admission != null && admission.SelectedIndex == 0 && admission.Inventories.Length == 5 && admission.Inventories[2][0] == 1,
                "Configured default inventories must be admitted even when the saved array is absent");
            var noCatalog = new LocalCharacterStore(path).ReadForAccount("synthetic").Single();
            Check(GameplayLoadoutAdmission.Project(noCatalog, noCatalog.ToRecord(), null) == null,
                "No-catalog absent inventory must retain its prior fallback behavior");
            var gate = new object(); var policy = new GameplayLoadoutSelectionSettlement(gate); bool current = true;
            string currentReservation = "reservation00001";
            bool Owner(GameplayLoadoutSelection a) => current && a.MatchesOwner(currentReservation, 7, 2, "synthetic", 1);
            CharacterCreationOutcome Save(GameplayLoadoutSelection a) => store.SaveLastUsedLoadoutForAccount(a.AccountId, a.CharacterId, a.InventoryIndex);
            Check(policy.Settle(Selection(), DateTime.UtcNow, Owner, Save) == "persisted" && store.ReadForAccount("synthetic").Single().LastUsedLoadout == 2, "Saved choice did not survive store reload");
            var rejoined = store.ReadForAccount("synthetic").Single();
            Check(rejoined.Loadouts == null, "Saving a default-kit preference must not materialize invented inventory rows");
            var rejoinAdmission = GameplayLoadoutAdmission.Project(rejoined, rejoined.ToRecord(), store.LoadoutCatalog);
            Check(rejoinAdmission != null && rejoinAdmission.SelectedIndex == 2 && rejoinAdmission.Inventories[2][0] == 1 &&
                rejoinAdmission.UpdateIds.SequenceEqual(admission.UpdateIds), "Default-kit preference was lost on rejoin projection");
            using (var json = JsonDocument.Parse(JsonSerializer.Serialize(rejoinAdmission)))
                Check(json.RootElement.GetProperty("selected_index").GetByte() == 2 && json.RootElement.GetProperty("inventories")[2][0].GetUInt16() == 1,
                    "Default-kit wire projection lost saved selection or actual primary");
            Check(policy.Settle(Selection(2, 4, "selection00000002"), DateTime.UtcNow, Owner, Save) == "rejected", "No-primary selected kit accepted");
            Check(store.SaveLastUsedLoadoutForAccount("other", 1, 0).Result != MagNetResult.Success &&
                store.SaveLastUsedLoadoutForAccount("synthetic", 1, 5).Result != MagNetResult.Success, "Invalid slot or unowned character accepted");
            // The same production gate covers check+IO and release/requeue. A release
            // cannot complete while an older owned store write is in flight.
            using var entered = new ManualResetEventSlim(); using var finish = new ManualResetEventSlim(); using var released = new ManualResetEventSlim();
            var write = Task.Run(() => policy.Settle(Selection(3, 3, "selection00000003"), DateTime.UtcNow, Owner, a => {
                entered.Set(); Check(finish.Wait(5000), "Synthetic save deadline"); return Save(a);
            }));
            Check(entered.Wait(5000), "Synthetic save did not enter");
            var release = Task.Run(() => { lock (gate) { current = false; policy.Forget("reservation00001"); released.Set(); } });
            Check(!released.Wait(100), "Release crossed in-flight owner write"); finish.Set();
            Check(write.Wait(5000) && release.Wait(5000), "Synthetic release tasks exceeded bounds");
            Check(policy.Settle(Selection(4, 0, "selection00000004"), DateTime.UtcNow, Owner, Save) == "rejected" &&
                store.ReadForAccount("synthetic").Single().LastUsedLoadout == 3, "Released/requeued old owner overwrote saved choice");
            lock (gate) { currentReservation = "reservation00002"; current = true; }
            Check(policy.Settle(Selection(1, 1, "selection00000009", currentReservation), DateTime.UtcNow, Owner, Save) == "persisted",
                "New reservation must have its own revision sequence");
            Check(policy.Settle(Selection(99, 0, "selection00000099"), DateTime.UtcNow, Owner, Save) == "rejected" &&
                store.ReadForAccount("synthetic").Single().LastUsedLoadout == 1, "Old receipt overwrote fresh requeue choice");
        }
        finally { Directory.Delete(directory, true); }
    }

}
