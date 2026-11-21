using System.Text;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    private const string FragoCapability = "frago_kill_bonus_v1";

    public static void FragoBridgeSchema()
    {
        const string qualified = ",\"frago\":true";
        Check(Parse(RewardReply("enemy_killed", ""), out _), "Legacy kill receipt rejected");
        Check(Parse(RewardReply("enemy_killed", qualified), out _), "Qualified kill receipt rejected");
        foreach (string invalid in new[] { "false", "null", "1", "\"true\"", "{}" })
            Check(!Parse(RewardReply("enemy_killed", ",\"frago\":" + invalid), out _), "Malformed FRAGO fact accepted");
        foreach (string kind in new[] { "enemy_finished", "teammate_healed", "match_minor_victory" })
            Check(!Parse(RewardReply(kind, qualified), out _), "FRAGO fact accepted on another reward kind");
        var bulk = RewardReply("enemy_killed", qualified);
        bulk["rewards"]![0]!["units"] = 2;
        Check(!Parse(bulk, out _), "FRAGO fact accepted on a bulk reward");
        string duplicate = RewardReply("enemy_killed", qualified).ToJsonString().Replace("\"frago\":true", "\"frago\":true,\"frago\":true");
        Check(!GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(duplicate), "synthetic", true, out _), "Duplicate FRAGO fact accepted");
    }

    private static JsonObject FragoCatalog(string coefficient)
    {
        var catalog = Catalog();
        var table = catalog["table"]!;
        table["Levels"]!["ExperienceLevel2"]!["ExperienceLevel"]!["Experience"] = 23;
        table["Levels"]!["ExperienceLevel3"]!["ExperienceLevel"]!["Experience"] = 59;
        var awards = table["Awards"]!;
        awards["EnemyKilled"] = 11;
        awards["ExperienceToHealOtherRatio"] = 0.625;
        awards["VictoryModifier"]!["MinorVictory"] = 0.125;
        if (coefficient != null) awards["FRAGOBonusPointModifer"] = JsonNode.Parse(coefficient);
        return catalog;
    }

    public static void FragoCatalogValidation() => WithDirectory(directory =>
    {
        foreach (string valid in new[] { null, "0", "0.25" }) Load(directory, FragoCatalog(valid));
        foreach (string invalid in new[] { "-0.25", "-1e-100", "1e100", "null", "true", "\"0.25\"", "{\"value\":0.25}" })
        {
            try { Load(directory, FragoCatalog(invalid)); }
            catch (InvalidDataException) { continue; }
            throw new Exception("Invalid FRAGO coefficient accepted: " + invalid);
        }
    });

    private static Task SettleFragoReply(LocalDmeHost host, JsonObject admission, string id, bool qualifying = true)
    {
        var response = RewardReply("enemy_killed", qualifying ? ",\"frago\":true" : "");
        var reward = response["rewards"]![0]!;
        reward["reward_id"] = id;
        foreach (string key in new[] { "reservation_id", "world_id", "player_id" }) reward[key] = admission[key]!.DeepClone();
        Check(Parse(response, out var parsed), "Synthetic kill reward rejected by bridge codec");
        return InvokeHost(host, "SettleRewardsAsync", parsed).WaitAsync(AdmissionTimeout);
    }

    public static void FragoAdmissionCapability() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, FragoCatalog(null)) };
        Create(store, "alpha", "Alpha");
        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Legacy synthetic admission failed");
            var legacy = bridge.Latest("reserve");
            Check(legacy["character"]!["frago_bonus_enabled"]?.GetValue<bool>() == false, "Legacy catalog advertised FRAGO bonus capability");
            await Release(host, legacy).WaitAsync(AdmissionTimeout);
            store.ProgressionCatalog = Load(directory, FragoCatalog("0"));
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Zero-coefficient synthetic admission failed");
            var admitted = bridge.Latest("reserve");
            Check(admitted["character"]!["frago_bonus_enabled"]?.GetValue<bool>() == true, "An authored zero must support qualified receipts");
            await SettleFragoReply(host, admitted, "ordinary-kill", qualifying: false);
            Check(!bridge.Latest("progression_committed").ContainsKey("match_frago_points"), "Ordinary kill ACK carried FRAGO points");
            await SettleFragoReply(host, admitted, "zero-bonus");
            var ack = bridge.Latest("progression_committed");
            Check(ack["match_frago_points"]!.GetValue<int>() == 0 && ack["match_experience"]!.GetValue<int>() == 22,
                "An authored zero changed ordinary XP or omitted the qualified receipt snapshot");
        }, FragoCapability).GetAwaiter().GetResult();
    });

    public static void FragoOlderWorkerAdmission() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, FragoCatalog("0.25")) };
        Create(store, "alpha", "Alpha");
        foreach (string capability in new[] { null, "FRAGO_KILL_BONUS_V1" })
        {
            WithAdmission(store, async (host, bridge, session) =>
            {
                Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Older-worker synthetic admission failed");
                var character = bridge.Latest("reserve")["character"]!.AsObject();
                Check(!character.ContainsKey("frago_bonus_enabled"), "New Horizon sent an unknown FRAGO admission field to an older worker");
            }, capability).GetAwaiter().GetResult();
        }
    });

    public static void FragoDurableSettlement() => WithDirectory(directory =>
    {
        // The supplied decimal rounds to 2 in f32. Using f64 or multiplying prior carry also changes the award.
        string path = Path.Combine(directory, "characters.json");
        var legacy = Load(directory, FragoCatalog(null));
        var authored = Load(directory, FragoCatalog("1.99999999"));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = legacy };
        Create(store, "alpha", "Alpha");
        Create(store, "beta", "Beta");
        var other = JsonNode.Parse(File.ReadAllBytes(path))!["characters"]![1]!.DeepClone();
        WithAdmission(store, async (host, bridge, session) =>
        {
            string match = $"world-{host.World.WorldId}-synthetic-match-123";
            var pending = store.ApplyRewardForAccount("alpha", 1, "fraction", match, "teammate_healed", 1);
            Check(pending.ExperienceAwarded == 0 && pending.MatchFraction == 0.625f, "Legacy fractional input setup failed");
            store.ProgressionCatalog = authored;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Synthetic admission failed");
            var admitted = bridge.Latest("reserve");
            store.ProgressionCatalog = legacy;
            byte[] before = File.ReadAllBytes(path);
            await SettleFragoReply(host, admitted, "first-kill");
            Check(before.SequenceEqual(File.ReadAllBytes(path)) && !bridge.Requests.Any(request => request["op"]!.GetValue<string>() == "progression_committed"),
                "Missing FRAGO coefficient acknowledged or mutated a qualified receipt");
            store.ProgressionCatalog = authored;
            await SettleFragoReply(host, admitted, "first-kill");
            var first = bridge.Latest("progression_committed");
            Check(first["match_frago_points"]!.GetValue<int>() == 22 && first["match_experience"]!.GetValue<int>() == 33 &&
                first["progression"]!["experience"]!.GetValue<int>() == 33 && first["progression"]!["level"]!.GetValue<int>() == 2 &&
                first["progression"]!["skill_points"]!.GetValue<int>() == 3 &&
                first["leadership"]!["leadership_level"]!.GetValue<int>() == 2 && first["leadership"]!["leadership_points"]!.GetValue<int>() == 23 &&
                !first.ContainsKey("match_leadership_points"), "Combined XP and tally failed to preserve f32 truncation or ordinary leadership promotion");
            await SettleFragoReply(host, admitted, "second-kill");
            var second = bridge.Latest("progression_committed");
            Check(second["match_frago_points"]!.GetValue<int>() == 44 && second["match_experience"]!.GetValue<int>() == 66, "FRAGO tally did not accumulate");
            var reopened = new LocalCharacterStore(path) { ProgressionCatalog = authored };
            var heal = reopened.ApplyRewardForAccount("alpha", 1, "second-fraction", match, "teammate_healed", 1);
            Check(heal.MatchFraction == 0.25f && heal.Experience == 67, "Whole-point FRAGO bonuses changed the ordinary carry");
            var final = reopened.ApplyRewardForAccount("alpha", 1, "round-result", match, "match_minor_victory", 1);
            Check(final.Experience == 75, "Final result did not include the credited FRAGO XP in its existing match basis");

            int acknowledgements = bridge.Requests.Count;
            before = File.ReadAllBytes(path);
            await SettleFragoReply(host, admitted, "first-kill", qualifying: false);
            Check(bridge.Requests.Count == acknowledgements && before.SequenceEqual(File.ReadAllBytes(path)), "Changed FRAGO context replay was accepted");
            store.ProgressionCatalog = legacy;
            await SettleFragoReply(host, admitted, "first-kill");
            var replay = bridge.Latest("progression_committed").DeepClone().AsObject();
            replay["request_id"] = first["request_id"]!.DeepClone();
            Check(JsonNode.DeepEquals(replay, first) && before.SequenceEqual(File.ReadAllBytes(path)), "Late replay did not return the original complete ACK");
            Check(JsonNode.DeepEquals(other, JsonNode.Parse(before)!["characters"]![1]), "Qualified reward changed another account's row");
        }, FragoCapability).GetAwaiter().GetResult();
    });

    public static void FragoFailedCommitAndOverflow() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var catalog = Load(directory, FragoCatalog("0.1875"));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = catalog };
        Create(store, "alpha", "Alpha");
        ProgressionRewardOutcome Settle(LocalCharacterStore target, string id, string kind = "enemy_killed", int units = 1, bool? frago = true) =>
            target.ApplyRewardForAccount("alpha", 1, id, "round", kind, units, frago: frago);

        byte[] before = File.ReadAllBytes(path);
        foreach (bool visible in new[] { false, true })
        {
            var refusingCommit = new LocalCharacterStore(path, new FailedCommitter(visible)) { ProgressionCatalog = catalog };
            try { Settle(refusingCommit, "qualifying-kill"); throw new Exception("A failed FRAGO commit returned success"); }
            catch (IOException) { }
            if (!visible) Check(before.SequenceEqual(File.ReadAllBytes(path)), "FRAGO write failure partially credited XP or tally");
        }
        var retry = Settle(store, "qualifying-kill");
        Check(retry.Duplicate && retry.Experience == 13 && retry.MatchExperience == 13 && retry.MatchFragoPoints == 2,
            "Visible uncertain FRAGO settlement did not replay its original XP and tally");
        before = File.ReadAllBytes(path);
        Check(Settle(store, "explicit-false", frago: false).Result != MagNetResult.Success &&
            Settle(store, "wrong-kind", "teammate_healed").Result != MagNetResult.Success &&
            Settle(store, "bulk", units: 2).Result != MagNetResult.Success && before.SequenceEqual(File.ReadAllBytes(path)),
            "Direct settlement accepted a malformed FRAGO fact");

        var overflowing = FragoCatalog("268435456");
        overflowing["table"]!["Awards"]!["EnemyKilled"] = 8;
        store.ProgressionCatalog = Load(directory, overflowing);
        Check(Settle(store, "overflow-product").Result != MagNetResult.Success && before.SequenceEqual(File.ReadAllBytes(path)),
            "Out-of-range single-precision bonus was converted or partially committed");
        overflowing["table"]!["Awards"]!["EnemyKilled"] = 1073741824;
        overflowing["table"]!["Awards"]!["FRAGOBonusPointModifer"] = 1;
        store.ProgressionCatalog = Load(directory, overflowing);
        Check(Settle(store, "overflow-sum").Result != MagNetResult.Success && before.SequenceEqual(File.ReadAllBytes(path)),
            "Out-of-range combined whole bonus and base were partially committed");

        store.ProgressionCatalog = catalog;
        var high = JsonNode.Parse(before)!;
        high["characters"]![0]!["experience"] = int.MaxValue - 12;
        high["characters"]![0]!["level"] = 3;
        File.WriteAllText(path, high.ToJsonString());
        before = File.ReadAllBytes(path);
        Check(Settle(store, "overflow-account").Result != MagNetResult.Success && before.SequenceEqual(File.ReadAllBytes(path)),
            "FRAGO award partially committed when the lifetime XP total would overflow");

        high["rewardReceipts"]!["qualifying-kill"]!["snapshot"]!.AsObject().Remove("matchFragoPoints");
        File.WriteAllText(path, high.ToJsonString());
        try { store.ReadAll(); }
        catch (InvalidDataException) { return; }
        throw new Exception("FRAGO receipt was accepted without its immutable tally snapshot");
    });
}
