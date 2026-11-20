using System.Text;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// Deliberately synthetic points, character levels and account records.
internal static class LeadershipProgressionChecks
{
    private static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    private static JsonObject RewardReply(string kind, string leadershipField)
    {
        return JsonNode.Parse($$"""
            {"v":2,"request_id":"synthetic","ok":true,"error":null,"messages":[],"rewards":[{
              "reward_id":"test-reward","reservation_id":"synthetic-reservation","match_id":"synthetic-match-123",
              "world_id":7,"player_id":2,"account_id":"alpha","character_id":1,"kind":"{{kind}}","units":1{{leadershipField}}}]}
            """)!.AsObject();
    }

    private static bool Parse(JsonObject reply, out GameplayBridgeResponse parsed) =>
        GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(reply.ToJsonString()), "synthetic", true, out parsed);

    public static void BridgeSchema()
    {
        const string final = "match_minor_defeat";
        Check(Parse(RewardReply(final, ""), out var legacy) && legacy.Rewards.Count == 1, "Legacy final receipt rejected");
        const string baseline = "\"baseline\":{\"leadership_level\":2,\"nominations\":255,\"matches_since_last_command\":17}";
        foreach (byte role in new byte[] { 0, 1, 2, 4 })
        {
            string context = $",\"leadership\":{{\"role\":{role}, {baseline}}}";
            Check(Parse(RewardReply(final, context), out var receipt) && receipt.Rewards.Count == 1, "Typed terminal leadership receipt rejected");
            Check(!Parse(RewardReply("enemy_killed", context), out _), "An action reward accepted final leadership facts");
        }
        foreach (string bad in new[] {
            "null", "{}", $"{{\"role\":3,{baseline}}}", $"{{\"role\":1,{baseline},\"points\":99}}",
            "{\"role\":1,\"baseline\":null}", "{\"role\":1,\"baseline\":{\"leadership_level\":2,\"nominations\":1}}",
            "{\"role\":1,\"baseline\":{\"leadership_level\":2,\"nominations\":256,\"matches_since_last_command\":17}}"
        })
            Check(!Parse(RewardReply(final, ",\"leadership\":" + bad), out _), "Malformed leadership facts accepted");
    }

    private static JsonObject Catalog() => JsonNode.Parse("""
        {"schema_version":1,"application_id":20231,"table":{"ExperienceRequiredScale":1,"Levels":{
          "ExperienceLevel1":{"ExperienceLevel":{"Experience":0,"SkillPointsGained":1,"LoadoutBudget":0,"Rank":0}},
          "ExperienceLevel2":{"ExperienceLevel":{"Experience":7,"SkillPointsGained":2,"LoadoutBudget":0,"Rank":0}},
          "ExperienceLevel3":{"ExperienceLevel":{"Experience":29,"SkillPointsGained":3,"LoadoutBudget":0,"Rank":0}}},
          "LeadershipLevels":{
            "ExperienceLeadershipLevel0":{"ExperienceLeadershipLevel":{}},
            "ExperienceLeadershipLevel1":{"ExperienceLeadershipLevel":{"LeadershipPoints":0,"MinimumLevel":2}},
            "ExperienceLeadershipLevel2":{"ExperienceLeadershipLevel":{"LeadershipPoints":23,"MinimumLevel":2}},
            "ExperienceLeadershipLevel3":{"ExperienceLeadershipLevel":{"LeadershipPoints":61,"MinimumLevel":3}}},
          "Awards":{"EnemyKilled":7,"ExperienceToHealOtherRatio":0,"PrimaryObjectiveActivated":0,"PrimaryObjectiveSecured":0,
            "PrimaryObjective":0,"SecondaryObjective":0,"ChargePlanted":0,"ChargeDefused":0,
            "VictoryModifier":{"MajorVictory":0,"MinorVictory":0,"MajorDefeat":0,"MinorDefeat":0},
            "LeadershipPoints_Squad":{"MajorVictory":11,"MinorVictory":12.75,"MajorDefeat":13,"MinorDefeat":14},
            "LeadershipPoints_Platoon":{"MajorVictory":-21.5,"MinorVictory":22,"MajorDefeat":23,"MinorDefeat":24},
            "LeadershipPoints_OIC":{"MajorVictory":31,"MinorVictory":32,"MajorDefeat":33,"MinorDefeat":34}}},
          "creation_settings":{"root_defaults":{"StartingLevel":1,"AdditionalSkillPoints":0,"AdditionalLeadershipPoints":23}}}
        """)!.AsObject();

    private static void WithDirectory(Action<string> test)
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-leadership-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try { test(directory); }
        finally { Directory.Delete(directory, true); }
    }

    private static MagProgressionCatalog Load(string directory, JsonObject fixture)
    {
        string path = Path.Combine(directory, "progression.json");
        File.WriteAllText(path, fixture.ToJsonString());
        return MagProgressionCatalog.Load(path, 20231);
    }

    private static void Create(LocalCharacterStore store, string account, string name)
    {
        var request = new MagCreateCharacterRequest { AccountName = name, Faction = 0, UniverseId = 26, NpId = new byte[36] };
        Check(store.CreateForAccount(request, account).Result == MagNetResult.Success, "Synthetic character creation failed");
    }

    public static void CreationAndExperience() => WithDirectory(directory =>
    {
        var catalog = Load(directory, Catalog());
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = catalog };
        Create(store, "alpha", "Alpha");
        var starting = store.ReadForAccount("alpha").Single().ToRecord();
        Check(starting.LeadershipPoints == 23 && starting.LeadershipLevel == 0, "Authored creation points or below-minimum leadership level lost");
        var gain = store.ApplyRewardForAccount("alpha", 1, "action", "first-round", "enemy_killed", 1);
        Check(gain.Result == MagNetResult.Success && gain.Level == 2, "Ordinary XP threshold not reached");
        var record = store.ReadForAccount("alpha").Single().ToRecord();
        Check(record.LeadershipPoints == 23 && record.LeadershipLevel == 2, "Both inclusive thresholds should qualify after character promotion");
        var notification = new MagCharacterRecordNotification { Record = record };
        byte[] encoded = MagMessageCodec.SerializeForApplication(notification, 20231);
        Check(MagMessageCodec.TryParseForApplication(encoded, 20231, out var wire) && wire is MagCharacterRecordNotification parsed &&
            parsed.Record.LeadershipPoints == 23 && parsed.Record.LeadershipLevel == 2, "Leadership promotion lost in full character notification");

        var sequential = Catalog();
        sequential["table"]!["LeadershipLevels"]!["ExperienceLeadershipLevel1"]!["ExperienceLeadershipLevel"]!["MinimumLevel"] = 3;
        var gated = new LocalCharacterStore(Path.Combine(directory, "gated.json")) { ProgressionCatalog = Load(directory, sequential), StartingLevel = 2 };
        Create(gated, "alpha", "Alpha");
        Check(gated.ReadForAccount("alpha").Single().ToRecord().LeadershipLevel == 0,
            "Leadership resolution skipped a disqualified preceding rank");
    });

    private static JsonObject LegacyCatalog()
    {
        var legacy = Catalog();
        legacy["table"]!.AsObject().Remove("LeadershipLevels");
        var awards = legacy["table"]!["Awards"]!.AsObject();
        foreach (string key in new[] { "LeadershipPoints_Squad", "LeadershipPoints_Platoon", "LeadershipPoints_OIC" }) awards.Remove(key);
        legacy["creation_settings"]!["root_defaults"]!.AsObject().Remove("AdditionalLeadershipPoints");
        return legacy;
    }

    public static void CatalogValidation() => WithDirectory(directory =>
    {
        Load(directory, LegacyCatalog());
        foreach (Action<JsonObject> damage in new Action<JsonObject>[] {
            doc => doc["table"]!.AsObject().Remove("LeadershipLevels"),
            doc => doc["table"]!["Awards"]!.AsObject().Remove("LeadershipPoints_Platoon"),
            doc => doc["table"]!["Awards"]!["LeadershipPoints_Squad"]!.AsObject().Remove("MinorDefeat"),
            doc => doc["table"]!["Awards"]!["LeadershipPoints_OIC"]!["MajorVictory"] = 2147483648L,
            doc => doc["table"]!["LeadershipLevels"]!.AsObject().Remove("ExperienceLeadershipLevel2"),
            doc => doc["table"]!["LeadershipLevels"]!["ExperienceLeadershipLevel1"]!["ExperienceLeadershipLevel"]!["LeadershipPoints"] = 2147483648L,
            doc => doc["creation_settings"]!["root_defaults"]!.AsObject().Remove("AdditionalLeadershipPoints")
        })
        {
            var invalid = Catalog(); damage(invalid);
            try { Load(directory, invalid); }
            catch (InvalidDataException) { continue; }
            throw new Exception("Partial or invalid authored leadership data was accepted");
        }
    });

    public static void DurableSettlement() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var legacy = new LocalCharacterStore(path) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(legacy, "alpha", "Alpha");
        Create(legacy, "beta", "Beta");
        var fallback = new GameplayLeadershipBaseline(3, 5, 77);
        var squad = new GameplayLeadershipResult(1, fallback);
        var legacyFinal = legacy.ApplyRewardForAccount("alpha", 1, "legacy-final", "legacy-round", "match_minor_victory", 1, squad);
        Check(legacyFinal.Result == MagNetResult.Success && legacyFinal.Leadership == null && !legacyFinal.ShouldNotifyCharacterRecord,
            "A catalog without leadership rows changed legacy final settlement");
        var fixture = JsonNode.Parse(File.ReadAllBytes(path))!;
        fixture["characters"]![1]!["leadership"] = JsonNode.Parse("""{"leadershipLevel":3,"nominations":198,"matchesSinceLastCommand":254}""");
        fixture["characters"]![1]!["unrelatedMetadata"] = "retain";
        File.WriteAllText(path, fixture.ToJsonString());
        var catalog = Load(directory, Catalog());
        LocalCharacterStore Reopen() => new(path) { ProgressionCatalog = catalog };
        var store = Reopen();
        Check(store.ReadForAccount("alpha").Single().Leadership == null, "Legacy missing leadership row was materialized by reading");
        Check(store.ReadForAccount("beta").Single().ToRecord().LeadershipLevel == 3, "Legacy eligibility not preserved");
        Check(store.ApplyRewardForAccount("alpha", 1, "action", "first-round", "enemy_killed", 1).Result == MagNetResult.Success,
            "Synthetic round action did not reach character level 2");
        var first = store.ApplyRewardForAccount("alpha", 1, "first-final", "first-round", "match_minor_victory", 1, squad);
        Check(first.Result == MagNetResult.Success && first.ExperienceAwarded == 0 && first.ShouldNotifyCharacterRecord && first.Leadership is {
            LeadershipPoints: 12, MatchLeadershipPoints: 12, LeadershipLevel: 1, Nominations: 5, MatchesSinceLastCommand: 77, Changed: true },
            "Zero-XP terminal result did not truncate the authored role award or preserve the fallback nominations and recency");
        var record = Reopen().ReadForAccount("alpha").Single().ToRecord();
        Check(record.LeadershipPoints == 12 && record.LeadershipLevel == 1 && record.LeadershipNominations == 5 && record.MatchesSinceLastCommand == 77,
            "Persisted leadership fields did not survive restart");
        Check(JsonNode.DeepEquals(fixture["characters"]![1], JsonNode.Parse(File.ReadAllBytes(path))!["characters"]![1]),
            "A leadership settlement changed the unrelated account");

        var platoon = new GameplayLeadershipResult(2, new GameplayLeadershipBaseline(0, 0, 0));
        Check(Reopen().ApplyRewardForAccount("alpha", 1, "other-final", "first-round", "match_minor_victory", 1, squad).Result != MagNetResult.Success,
            "Distinct terminal receipt replay for the same round was accepted");
        Check(Reopen().ApplyRewardForAccount("alpha", 1, "first-final", "first-round", "match_minor_victory", 1, platoon).Result != MagNetResult.Success,
            "Changing the receipt role/baseline on replay was accepted");
        Check(Reopen().ApplyRewardForAccount("alpha", 2, "other-account", "first-round", "match_minor_victory", 1, squad).Result != MagNetResult.Success,
            "Unowned character accepted a leadership settlement");
        Check(Reopen().ApplyRewardForAccount("alpha", 1, "too-early", "round-b", "enemy_killed", 1, platoon).Result != MagNetResult.Success,
            "A direct store action accepted terminal leadership context");
        var second = Reopen().ApplyRewardForAccount("alpha", 1, "second-final", "round-b", "match_major_defeat", 1, platoon);
        Check(second.Leadership is { LeadershipPoints: 35, MatchLeadershipPoints: 23, LeadershipLevel: 2, Nominations: 5 },
            "Subsequent settlement used the wrong role/outcome or overwrote a saved baseline");
        var replay = Reopen().ApplyRewardForAccount("alpha", 1, "first-final", "first-round", "match_minor_victory", 1, squad);
        Check(replay.Result == MagNetResult.Success && replay.Duplicate && replay.Leadership == first.Leadership &&
            Reopen().ReadForAccount("alpha").Single().ToRecord().LeadershipPoints == 35, "Historical receipt replay changed or paid again");

        var beta = Reopen().ApplyRewardForAccount("beta", 2, "beta-final", "round-b", "match_major_victory", 1, platoon);
        Check(beta.Leadership is { LeadershipPoints: 0, MatchLeadershipPoints: 0, LeadershipLevel: 3, Nominations: 198, MatchesSinceLastCommand: 254 } &&
            Reopen().ReadForAccount("beta").Single().Leadership.LeadershipLevelFloor == 3, "Legacy rank floor or native nonpositive award behavior lost");
        var oic = new GameplayLeadershipResult(4, fallback);
        foreach (string round in new[] { "round-c", "round-d" })
            Check(Reopen().ApplyRewardForAccount("beta", 2, round + "-final", round, "match_minor_defeat", 1, oic).Leadership?.MatchLeadershipPoints == 34,
                "OIC reward used the wrong outcome column");
        for (int n = 0; n < 5; n++)
            Check(Reopen().ApplyRewardForAccount("beta", 2, "beta-action-" + n, "round-e", "enemy_killed", 1).Result == MagNetResult.Success,
                "Synthetic activity XP rejected");
        var betaSaved = Reopen().ReadForAccount("beta").Single().Leadership;
        Check(betaSaved.LeadershipPoints == 68 && betaSaved.LeadershipLevel == 3 && betaSaved.LeadershipLevelFloor == null,
            "Compatibility floor did not clear when both authored thresholds passed");
    });

    private sealed class FailedCommitter(bool visible) : ICharacterStoreCommitter
    {
        public void WriteAtomically(string path, byte[] contents)
        {
            if (visible) File.WriteAllBytes(path, contents);
            throw new IOException("synthetic commit failure");
        }
    }

    public static void FailedCommit() => WithDirectory(directory =>
    {
        var catalog = Load(directory, Catalog());
        string path = Path.Combine(directory, "characters.json");
        var plain = new LocalCharacterStore(path) { ProgressionCatalog = catalog };
        Create(plain, "alpha", "Alpha");
        var context = new GameplayLeadershipResult(4, new GameplayLeadershipBaseline(1, 7, 255));
        ProgressionRewardOutcome Settle(LocalCharacterStore store, string id, string round) =>
            store.ApplyRewardForAccount("alpha", 1, id, round, "match_minor_defeat", 1, context);
        byte[] before = File.ReadAllBytes(path);
        foreach (bool visible in new[] { false, true })
        {
            var refusingCommit = new LocalCharacterStore(path, new FailedCommitter(visible)) { ProgressionCatalog = catalog };
            try { Settle(refusingCommit, "first-final", "round-a"); throw new Exception("Commit failure returned success"); }
            catch (IOException) { }
            if (!visible) Check(before.SequenceEqual(File.ReadAllBytes(path)), "Commit failure mutated store or receipt");
        }
        var replay = Settle(plain, "first-final", "round-a");
        Check(replay.Duplicate && replay.Leadership is { LeadershipPoints: 57, MatchLeadershipPoints: 34 } &&
            plain.ReadForAccount("alpha").Single().ToRecord().LeadershipPoints == 57, "Visible uncertain settlement was awarded twice");
        var highBit = JsonNode.Parse(File.ReadAllBytes(path))!;
        highBit["characters"]![0]!["leadership"]!["leadershipPoints"] = 2147483647u;
        highBit["characters"]![0]!["leadership"]!["leadershipLevel"] = 3;
        highBit["characters"]![0]!["experience"] = 29;
        highBit["characters"]![0]!["level"] = 3;
        File.WriteAllText(path, highBit.ToJsonString());
        var crossed = Settle(plain, "high-bit-final", "round-b");
        Check(crossed.Leadership is { LeadershipPoints: 2147483681u, MatchLeadershipPoints: 34, LeadershipLevel: 0, Changed: true },
            "Crossing the high bit must retain the unsigned balance and resolve rank with native signed comparison");
        var nearLimit = JsonNode.Parse(File.ReadAllBytes(path))!;
        nearLimit["characters"]![0]!["leadership"]!["leadershipPoints"] = uint.MaxValue;
        File.WriteAllText(path, nearLimit.ToJsonString());
        before = File.ReadAllBytes(path);
        Check(Settle(plain, "overflow-final", "round-c").Result != MagNetResult.Success && before.SequenceEqual(File.ReadAllBytes(path)),
            "Leadership overflow committed or partially changed the store");
    });
}
