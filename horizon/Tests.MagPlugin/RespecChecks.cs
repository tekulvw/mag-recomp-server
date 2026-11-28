using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// Skill respecs and the operator respec point floor (MAG_RESPEC_POINTS_FLOOR). Every account,
// catalog, cost and fee here is invented; the fees are deliberately not the retail ones.
internal static partial class LeadershipProgressionChecks
{
    // Invented fees: the first respec costs 1200, every later one 2600.
    static readonly uint[] RespecFees = { 1200, 2600 };
    const uint RespecFloor = 3000;

    /// <summary>A retail Armory whose faction-0 kit 0 is a starter rifle (wire 1025) and kits 1 to 4 are empty, and a
    /// skill table with a free starter skill 0, a bought skill 3 (cost 2: rifle 1026 and sight 1028) and skill 5 (cost 1).</summary>
    static (MagLoadoutCatalog Armory, MagSkillCatalog Skills) RespecCatalogs(string directory, bool withFees = true)
    {
        object Item(int bit, string name, int? ammo) => new {
            wire_id = 1024 + bit, name, authored_ref = "synthetic:" + bit, available_bit = bit, factions = new[] { 0, 1, 2 }, primary_ammo = ammo };
        var starter = Enumerable.Repeat<short?>(null, 22).ToArray();
        starter[0] = 1025;
        var empty = Enumerable.Repeat<short?>(null, 22).ToArray();
        string armory = Path.Combine(directory, "respec-armory.json");
        File.WriteAllText(armory, JsonSerializer.Serialize(new {
            schema_version = 1, application_id = 20231,
            source_sha256 = new { synthetic = new string('c', 64) },
            items = new[] { Item(1, "Example starter rifle", 7), Item(2, "Example bought rifle", 9), Item(4, "Example bought sight", null), Item(7, "Example scope", null) },
            factions = Enumerable.Range(0, 3).Select(faction => new {
                faction, available_item_bits = faction == 0 ? new[] { 1 } : Array.Empty<int>(), skill_ids = new[] { 0 },
                loadouts = Enumerable.Range(0, 5).Select(kit => new { update_id = 1, items = kit == 0 ? starter : empty })
            })
        }));
        object Faction(bool available, params string[] tokens) => new {
            Available = available,
            Equipment = tokens.Select((token, index) => (token, index)).ToDictionary(pair => "Reward" + pair.index, pair => new { element = pair.token })
        };
        object Skill(int cost, int tier, params string[] tokens) => new { MagCharacterSkill = new {
            Cost = cost, Tier = tier, Category = "kSkillCategoryExample", Green = Faction(true, tokens), Black = Faction(false), Red = Faction(false) } };
        object Thresholds() => new { Tier2_PrereqPoints = 0, Tier3_PrereqPoints = 0, Tier4_PrereqPoints = 0, Tier5_PrereqPoints = 0 };
        var table = new JsonObject
        {
            ["Tiers"] = JsonSerializer.SerializeToNode(new { ExampleTier = new { MagCharacterSkillTier = new {
                Category = "kSkillCategoryExample", Green = Thresholds(), Black = Thresholds(), Red = Thresholds() } } }),
            ["Skills"] = JsonSerializer.SerializeToNode(new Dictionary<string, object> {
                ["MagCharacterSkill0"] = Skill(0, 0, "starter_rifle"),
                ["MagCharacterSkill3"] = Skill(2, 1, "bought_rifle", "bought_sight"),
                ["MagCharacterSkill5"] = Skill(1, 1, "scope")
            })
        };
        if (withFees)
            table["ReSpec"] = JsonSerializer.SerializeToNode(RespecFees.Select((fee, index) => (fee, index))
                .ToDictionary(row => "MagCharacterSkillReSpec" + row.index, row => (object)new { MagCharacterSkillReSpec = new { Points = row.fee } }));
        string skills = Path.Combine(directory, withFees ? "respec-skills.json" : "respec-skills-no-fees.json");
        File.WriteAllText(skills, new JsonObject {
            ["schema_version"] = 2, ["application_id"] = 20231,
            ["equipment_token_item_ids"] = JsonSerializer.SerializeToNode(new Dictionary<string, int[]> {
                ["starter_rifle"] = new[] { 1 }, ["bought_rifle"] = new[] { 2 }, ["bought_sight"] = new[] { 4 }, ["scope"] = new[] { 7 } }),
            ["table"] = table
        }.ToJsonString());
        return (MagLoadoutCatalog.LoadRetail(armory), MagSkillCatalog.LoadForApplication(skills, 20231));
    }

    static LocalCharacterStore RespecStore(string path, string directory, uint? floor, bool withFees = true)
    {
        var (armory, skills) = RespecCatalogs(directory, withFees);
        return new LocalCharacterStore(path) { LoadoutCatalog = armory, SkillCatalog = skills, RespecPointsFloor = floor };
    }

    /// <summary>Account alpha with characters in slots 0 to 2 (alpha!0..alpha!2), each with five skill points.</summary>
    static void CreateThreeSlots(LocalCharacterStore store, string path)
    {
        foreach (var name in new[] { "alpha!0", "alpha!1", "alpha!2" })
            Check(store.CreateForAccount(SlotRequest(name), "alpha").Result == MagNetResult.Success, "Synthetic creation " + name);
        EditCharacters(path, c => c["skillPoints"] = 5);
    }

    static void EditCharacters(string path, Action<JsonObject> edit, string name = null)
    {
        var document = JsonNode.Parse(File.ReadAllBytes(path))!.AsObject();
        foreach (var node in document["characters"]!.AsArray())
            if (name == null || node!["name"]!.GetValue<string>() == name) edit(node!.AsObject());
        File.WriteAllText(path, document.ToJsonString());
    }

    static StoredCharacter Slot(LocalCharacterStore store, byte slot) => store.ReadForAccount("alpha").Single(c => c.Slot == slot);

    static MagLoadout RespecKit(params (int Slot, short Item)[] items)
    {
        var kit = new MagLoadout { UpdateId = 2 };
        foreach (var (slot, item) in items) kit.Items[slot] = item;
        return kit;
    }

    static MagRetailSkillRespecResponse Respec(MagMlsSession session)
    {
        var replies = session.HandleMany(CardEnvelope(new MagRetailSkillRespecRequest(), 20231), DateTime.UtcNow);
        Check(replies.Count == 1, "A ready MLS session must answer every respec request exactly once");
        Check(MagMessageCodec.TryParseForApplication(replies[0].Payload, 20231, out var parsed) && parsed is MagRetailSkillRespecResponse,
            "Respec reply did not parse as a 0255 response");
        return (MagRetailSkillRespecResponse)parsed;
    }

    public static void RespecWire()
    {
        // 0254 is the bare type: no MAS header, no character, fee or transaction.
        byte[] request = MagMessageCodec.SerializeForApplication(new MagRetailSkillRespecRequest(), 20231);
        Check(request.SequenceEqual(new byte[] { 0x02, 0x54 }), "Respec request wire changed");
        Check(MagMessageCodec.TryParseClientForApplication(request, 20231, out var parsed) && parsed is MagRetailSkillRespecRequest,
            "The retail lobby did not accept a respec request");
        Check(!MagMessageCodec.TryParseClientForApplication(request, 20230, out _), "The prototype must not claim retail 0254");
        Check(!MagMessageCodec.TryParseClientForApplication(new byte[] { 0x02, 0x54, 0x01 }, 20231, out _), "Respec request with a payload parsed");
        Check(!MagMessageCodec.TryParseClientForApplication(MagMessageCodec.SerializeForApplication(new MagRetailSkillRespecResponse(), 20231), 20231, out _),
            "A respec response is not a client request");

        // A refusal is the type and one clear bit; success adds the full retail record.
        byte[] refused = MagMessageCodec.SerializeForApplication(new MagRetailSkillRespecResponse { Success = false }, 20231);
        Check(refused.SequenceEqual(new byte[] { 0x02, 0x55, 0x00 }), "A refused respec must carry no record");
        var record = new MagCharacterRecord { Name = "Example", AccountId = 9, Faction = 0, Level = 4, SkillPoints = 7, RespecCount = 2, RespecPoints = 3456, VeteranLevel = 1 };
        byte[] success = MagMessageCodec.SerializeForApplication(new MagRetailSkillRespecResponse { Success = true, Record = record }, 20231);
        byte[] purchase = MagMessageCodec.SerializeForApplication(new MagPurchaseSkillResponse { Success = true, Record = record }, 20231);
        Check(success.Skip(2).SequenceEqual(purchase.Skip(2)), "A successful respec must carry the same bit and record as a skill purchase");
        Check(MagMessageCodec.TryParseForApplication(success, 20231, out var back) &&
            back is MagRetailSkillRespecResponse { Success: true } round && round.Record.RespecPoints == 3456 && round.Record.RespecCount == 2 &&
            round.Record.SkillPoints == 7 && round.Record.Name == "Example", "Respec response round trip");
        bool retailOnly = false;
        try { MagMessageCodec.SerializeForApplication(new MagRetailSkillRespecRequest(), 20230); } catch (ArgumentException) { retailOnly = true; }
        Check(retailOnly, "Respec messages are retail-only");
    }

    public static void RespecFloorSetting()
    {
        Check(Server.Medius.Program.ParseRespecPointsFloor(null) == null && Server.Medius.Program.ParseRespecPointsFloor(" ") == null, "Unset floor is null");
        Check(Server.Medius.Program.ParseRespecPointsFloor("3000") == 3000 && Server.Medius.Program.ParseRespecPointsFloor(" 0 ") == 0 &&
            Server.Medius.Program.ParseRespecPointsFloor("4294967295") == uint.MaxValue, "Valid floors parse");
        foreach (var bad in new[] { "-1", "3000.5", "1e3", "x", "4294967296", "+5" })
        {
            bool threw = false;
            try { Server.Medius.Program.ParseRespecPointsFloor(bad); } catch (InvalidDataException) { threw = true; }
            Check(threw, "Invalid MAG_RESPEC_POINTS_FLOOR must fail startup: " + bad);
        }
    }

    /// <summary>The floor applies on load to every slot, never rewrites the store, keeps a higher balance, and is inert when unset.</summary>
    public static void RespecFloorOnLoad() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var store = RespecStore(path, directory, RespecFloor);
        CreateThreeSlots(store, path);
        EditCharacters(path, c => c["respecPoints"] = 5000, "alpha!1");
        byte[] before = File.ReadAllBytes(path);

        for (byte slot = 0; slot < 3; slot++)
        {
            var record = Slot(store, slot).ToRecord();
            Check(record.RespecPoints == (slot == 1 ? 5000u : RespecFloor) && record.RespecCount == 0,
                $"Slot {slot}: floor or higher balance, got {record.RespecPoints}");
        }
        Check(Slot(store, 0).RespecPoints == null && Slot(store, 1).RespecPoints == 5000, "The stored balances are as written");
        Check(before.SequenceEqual(File.ReadAllBytes(path)), "Applying the floor on load rewrote the store");

        // The character list, the player card and the lobby login all read through the same record.
        WithApplication(20231, () =>
        {
            var session = MakeSession(store, 20231, "alpha", Slot(store, 2).Id);
            var card = AskCard(session, new MagCharacterDataRequest { TransactionId = 4, QueryKind = 2, AccountId = Slot(store, 2).Id, RequestId = 1 });
            Check(card.Success && card.Record.RespecPoints == RespecFloor, "Character data must carry the floored balance");
        });

        var unset = RespecStore(path, directory, null);
        Check(Slot(unset, 0).ToRecord().RespecPoints == 0 && Slot(unset, 1).ToRecord().RespecPoints == 5000 && Slot(unset, 2).ToRecord().RespecPoints == 0,
            "Without the setting balances are exactly as stored");
        var zero = RespecStore(path, directory, 0);
        Check(Slot(zero, 0).ToRecord().RespecPoints == 0, "A floor of 0 changes nothing");
        Check(before.SequenceEqual(File.ReadAllBytes(path)), "Reading without a floor rewrote the store");
    });

    /// <summary>A respec refunds skills, clears their equipment, rebuilds the kits and spends the fee; the floor then tops the balance up.</summary>
    public static void RespecSpendAndTopUp() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var store = RespecStore(path, directory, RespecFloor);
        CreateThreeSlots(store, path);
        uint id = Slot(store, 0).Id;
        Check(store.PurchaseSkillForAccount("alpha", id, 3).Reason == "skill-purchase-committed" &&
            store.PurchaseSkillForAccount("alpha", id, 5).Reason == "skill-purchase-committed", "Synthetic purchases");
        Check(store.SaveLoadoutForAccount("alpha", id, 2, RespecKit((0, 1026), (14, 1028))).Result == MagNetResult.Success &&
            store.SaveLastUsedLoadoutForAccount("alpha", id, 2).Result == MagNetResult.Success, "Bought kit saved and selected");
        var bought = Slot(store, 0).ToRecord();
        Check(bought.SkillPoints == 2 && (bought.Skills[0] & (1u << 3 | 1u << 5)) != 0 && bought.LastUsedLoadout == 2, "Purchases before the respec");
        string others = JsonSerializer.Serialize(store.ReadForAccount("alpha").Where(c => c.Slot != 0).ToArray());

        var refreshes = new List<(string, uint)>();
        WithApplication(20231, () =>
        {
            var session = MakeSession(store, 20231, "alpha", id);
            session.LoadoutCommitted = (account, character) => refreshes.Add((account, character));
            var reply = Respec(session);
            Check(reply.Success && session.StorageStatus == "skill-respec-committed/fee=1200/refunded=3", "First respec: " + session.StorageStatus);
            var r = reply.Record;
            Check(r.AccountId == id && r.SkillPoints == 5 && r.RespecCount == 1 && r.RespecPoints == RespecFloor,
                $"Reply record after the respec: points {r.SkillPoints}, count {r.RespecCount}, balance {r.RespecPoints}");
            Check(r.Skills[0] == 1u && r.Skills.Skip(1).All(word => word == 0), "Only the free starter skill stays owned");
            Check((r.AvailableItems[0] & (1u << 2 | 1u << 4 | 1u << 7)) == 0 && (r.AvailableItems[0] & (1u << 1)) != 0,
                "Refunded equipment is gone and starter equipment stays");
            Check(r.LastUsedLoadout == 0 && r.Loadouts[0].Items[0] == 1025 && r.Loadouts.Skip(1).All(kit => kit.Items.All(item => item == -1)),
                "Kits are the faction defaults and the emptied selection falls back to kit 0");
            Check(refreshes.SequenceEqual(new[] { ("alpha", id) }) && session.LoadoutRefreshStatus == "notified", "A live match is told the kits changed");

            var stored = Slot(store, 0);
            Check(stored.RespecCount == 1 && stored.RespecPoints == RespecFloor - 1200, "The spend is stored before the floor: " + stored.RespecPoints);
            Check(stored.ToRecord().RespecPoints == RespecFloor, "The floor tops the balance back up after the spend");
            Check(JsonSerializer.Serialize(store.ReadForAccount("alpha").Where(c => c.Slot != 0).ToArray()) == others, "Other slots were changed");

            // The second respec costs the next fee; the floor covers it, and the table's last fee repeats.
            Check(store.PurchaseSkillForAccount("alpha", id, 3).Reason == "skill-purchase-committed", "Rebuy after the respec");
            var second = Respec(session);
            Check(second.Success && session.StorageStatus == "skill-respec-committed/fee=2600/refunded=2" && second.Record.RespecCount == 2 &&
                second.Record.RespecPoints == RespecFloor && Slot(store, 0).RespecPoints == RespecFloor - 2600, "Second respec: " + session.StorageStatus);
            var third = Respec(session);
            Check(third.Success && session.StorageStatus == "skill-respec-committed/fee=2600/refunded=0" && third.Record.SkillPoints == 5 &&
                third.Record.RespecCount == 3, "A respec with nothing to refund still counts: " + session.StorageStatus);

            // A respec on another slot spends that slot only.
            var slotTwo = MakeSession(store, 20231, "alpha", Slot(store, 2).Id);
            Check(Respec(slotTwo).Success && Slot(store, 2).RespecCount == 1 && Slot(store, 1).RespecCount == null && Slot(store, 0).RespecCount == 3,
                "Respecs are per character");
        });

        // A balance above the floor is spent from and kept above it.
        EditCharacters(path, c => c["respecPoints"] = 9000, "alpha!1");
        Check(store.RespecSkillsForAccount("alpha", Slot(store, 1).Id).Reason == "skill-respec-committed" && Slot(store, 1).RespecPoints == 7800 &&
            Slot(store, 1).ToRecord().RespecPoints == 7800, "A higher balance pays from itself and is not lowered to the floor");
    });

    /// <summary>Without the floor, or with a floor below the fee, a respec is refused, answered with a bare failure and changes nothing.</summary>
    public static void RespecRefusals() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var store = RespecStore(path, directory, null);
        CreateThreeSlots(store, path);
        uint id = Slot(store, 0).Id;
        Check(store.PurchaseSkillForAccount("alpha", id, 3).Reason == "skill-purchase-committed", "Synthetic purchase");
        byte[] before = File.ReadAllBytes(path);
        WithApplication(20231, () =>
        {
            var session = MakeSession(store, 20231, "alpha", id);
            var reply = Respec(session);
            Check(!reply.Success && session.StorageStatus == "insufficient-respec-points", "Unset floor and no balance: " + session.StorageStatus);
            var low = MakeSession(RespecStore(path, directory, 1000), 20231, "alpha", id);
            Check(!Respec(low).Success && low.StorageStatus == "insufficient-respec-points", "A floor below the fee is not enough");
            var exact = RespecStore(path, directory, 1199);
            Check(exact.RespecSkillsForAccount("alpha", id).Reason == "insufficient-respec-points", "One point short is refused");
            var noFees = MakeSession(RespecStore(path, directory, RespecFloor, withFees: false), 20231, "alpha", id);
            Check(!Respec(noFees).Success && noFees.StorageStatus == "respec-fees-unavailable", "A catalog without fees refuses visibly");
            var other = RespecStore(path, directory, RespecFloor);
            Check(other.RespecSkillsForAccount("beta", id).Reason == "character-not-owned", "Another account's character is refused");
        });
        Check(before.SequenceEqual(File.ReadAllBytes(path)), "A refused respec changed the store");

        // An exact balance is enough.
        var enough = RespecStore(path, directory, 1200);
        Check(enough.RespecSkillsForAccount("alpha", id).Reason == "skill-respec-committed" && Slot(enough, 0).RespecPoints == 0 &&
            Slot(enough, 0).ToRecord().RespecPoints == 1200, "An exact balance pays the fee");
    });

    /// <summary>mag-admin and the admin API show each slot's respec points.</summary>
    public static void RespecAdminView() => WithDirectory(directory =>
    {
        var now = SlotNow.UtcDateTime;
        string path = Path.Combine(directory, "characters.json");
        var store = RespecStore(path, directory, RespecFloor);
        CreateThreeSlots(store, path);
        EditCharacters(path, c => c["respecPoints"] = 5000, "alpha!1");
        Check(store.RespecSkillsForAccount("alpha", Slot(store, 0).Id).Reason == "skill-respec-committed", "Synthetic respec");
        string backups = Path.Combine(directory, ".maintenance-backups"), markers = Path.Combine(directory, "markers");
        Directory.CreateDirectory(markers);
        MagActiveAccounts.Write(Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json"), new Dictionary<string, DateTime>(), now);
        var output = new StringWriter(); var error = new StringWriter();
        Check(MagAdminCli.Run(new[] { "character", "show", "alpha", "--slot", "0" }, store, backups, markers, now, output, error) == 0, "Show: " + error);
        string shown = output.ToString();
        Check(shown.Contains("respec:       3000 points (stored 1800, floor applied), 1 taken, next costs 2600"), "Show respec line: " + shown);
        Check(shown.Contains("alpha!0              faction 0  level   1  respec points   3000") &&
            shown.Contains("alpha!1              faction 0  level   1  respec points   5000") &&
            shown.Contains("alpha!2              faction 0  level   1  respec points   3000"), "Show lists each slot's respec points: " + shown);

        var api = new MagAdminApi(new System.Net.IPEndPoint(System.Net.IPAddress.Loopback, 0),
            Encoding.UTF8.GetBytes("synthetic-admin-api-secret-0123456789abcdef"), store, backups, markers, () => now, _ => { });
        var detail = JsonDocument.Parse(api.Handle(SignedSlot("GET", "/admin/v1/characters/alpha?slot=0", now)).Body).RootElement;
        Check(detail.GetProperty("respec_points").GetUInt32() == RespecFloor && detail.GetProperty("stored_respec_points").GetUInt32() == 1800 &&
            detail.GetProperty("respec_count").GetInt32() == 1 && detail.GetProperty("next_respec_fee").GetUInt32() == 2600, "API respec detail");
        var slots = detail.GetProperty("characters").EnumerateArray().Select(c => (c.GetProperty("slot").GetInt32(), c.GetProperty("respec_points").GetUInt32())).ToArray();
        Check(slots.SequenceEqual(new[] { (0, RespecFloor), (1, 5000u), (2, RespecFloor) }), "API respec points per slot");
        var accounts = JsonDocument.Parse(api.Handle(SignedSlot("GET", "/admin/v1/accounts", now)).Body).RootElement.GetProperty("accounts");
        Check(accounts.EnumerateArray().All(a => a.GetProperty("respec_points").GetUInt32() >= RespecFloor), "API account list respec points");
    });

    /// <summary>PostgreSQL stores the count and balance, reads null as 0, and applies the floor on read only.</summary>
    public static void PostgresRespec() => WithDirectory(directory => WithPostgres("postgres skill respec", (database, connection) =>
    {
        var (armory, skills) = RespecCatalogs(directory);
        var pg = LocalCharacterStore.ForPostgres(database);
        pg.LoadoutCatalog = armory; pg.SkillCatalog = skills; pg.RespecPointsFloor = RespecFloor;
        foreach (var name in new[] { "alpha!0", "alpha!1", "alpha!2" })
            Check(pg.CreateForAccount(SlotRequest(name), "alpha").Result == MagNetResult.Success, "Synthetic creation " + name);
        Check(Scalar(connection, "SELECT count(*) FROM mag.characters WHERE respec_count IS NULL AND respec_points IS NULL") == 3,
            "New characters store no respec fields");
        Check(pg.ReadForAccount("alpha").All(c => c.ToRecord().RespecPoints == RespecFloor && c.ToRecord().RespecCount == 0), "Floor on load");
        uint id = pg.ReadForAccount("alpha").Single(c => c.Slot == 0).Id;
        Sql(connection, $"UPDATE mag.characters SET skill_points = 5 WHERE id = {id}");
        Check(pg.PurchaseSkillForAccount("alpha", id, 3).Reason == "skill-purchase-committed", "Synthetic purchase");
        var outcome = pg.RespecSkillsForAccount("alpha", id);
        Check(outcome.Result == MagNetResult.Success && outcome.Record.RespecPoints == RespecFloor && outcome.Record.SkillPoints == 5, "Respec: " + outcome.Reason);
        Check(Scalar(connection, $"SELECT respec_count FROM mag.characters WHERE id = {id}") == 1 &&
            Scalar(connection, $"SELECT respec_points FROM mag.characters WHERE id = {id}") == RespecFloor - 1200, "Stored count and balance");
        Check(Scalar(connection, "SELECT count(*) FROM mag.character_loadouts WHERE character_id = " + id) == 5, "Rebuilt kits stored");
        var unset = LocalCharacterStore.ForPostgres(database);
        unset.LoadoutCatalog = armory; unset.SkillCatalog = skills;
        var reread = unset.ReadForAccount("alpha").Single(c => c.Id == id);
        Check(reread.RespecCount == 1 && reread.RespecPoints == 1800 && reread.ToRecord().RespecPoints == 1800 &&
            unset.ReadForAccount("alpha").Count(c => c.ToRecord().RespecPoints == 0) == 2, "Without the floor, balances as stored");
        bool refused = false;
        try { Sql(connection, $"UPDATE mag.characters SET respec_count = 256 WHERE id = {id}"); } catch (Npgsql.PostgresException) { refused = true; }
        Check(refused, "The respec count is a byte");
    }));
}
