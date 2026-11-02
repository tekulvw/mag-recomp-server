using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using RT.Models.MAG;
using Server.Medius;

// Synthetic values exercise optional configuration; none are retail XP defaults.
internal static class ReviveRewardChecks
{
    private static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    private static void Catalog(string path, Dictionary<string, object> optional)
    {
        var awards = new Dictionary<string, object> {
            { "EnemyKilled", 3 }, { "ExperienceToHealOtherRatio", 0.25f },
            { "PrimaryObjectiveActivated", 1 }, { "PrimaryObjectiveSecured", 1 },
            { "PrimaryObjective", 1 }, { "SecondaryObjective", 1 },
            { "ChargePlanted", 1 }, { "ChargeDefused", 1 },
            { "VictoryModifier", new { MajorVictory = 1, MinorVictory = 1, MajorDefeat = 0, MinorDefeat = 0 } }
        };
        foreach (var entry in optional) awards.Add(entry.Key, entry.Value);
        File.WriteAllText(path, JsonSerializer.Serialize(new {
            schema_version = 1, application_id = 20231,
            table = new {
                ExperienceRequiredScale = 1,
                Levels = new { ExperienceLevel1 = new { ExperienceLevel = new { Experience = 0, SkillPointsGained = 0, LoadoutBudget = 0, Rank = 0 } } },
                Awards = awards
            },
            creation_settings = new { root_defaults = new { StartingLevel = 1, AdditionalSkillPoints = 0 } }
        }));
    }

    public static void Run()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-revive-rewards-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string path = Path.Combine(directory, "progression.json");
            Catalog(path, new Dictionary<string, object> { { "FinishingMoves", 2.75f }, { "Resuscitations", 7.5f } });
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) {
                ProgressionCatalog = MagProgressionCatalog.Load(path, 20231)
            };
            Check(store.CreateForAccount(new MagCreateCharacterRequest {
                AccountName = "Example", Faction = 0, UniverseId = 26, NpId = new byte[36]
            }, "synthetic").Result == MagNetResult.Success, "Synthetic reward character creation");
            var finished = store.ApplyRewardForAccount("synthetic", 1, "finish-1", "match-1", "enemy_finished", 1);
            Check(finished.Result == MagNetResult.Success && finished.ExperienceAwarded == 2 && finished.MatchFraction == 0.75f,
                "Configured finishing reward or fractional carry was lost");
            var revived = store.ApplyRewardForAccount("synthetic", 1, "revive-1", "match-1", "teammate_resuscitated", 1);
            Check(revived.Result == MagNetResult.Success && revived.ExperienceAwarded == 8 && revived.MatchExperience == 10 && revived.MatchFraction == 0.25f,
                "Configured revive reward or shared carry was lost");
            var repeated = store.ApplyRewardForAccount("synthetic", 1, "revive-1", "match-1", "teammate_resuscitated", 1);
            Check(repeated.Result == MagNetResult.Success && repeated.Duplicate && repeated.MatchExperience == 10,
                "Repeated revive receipt awarded again");
            foreach (string kind in new[] { "enemy_finished", "teammate_resuscitated" })
            {
                Check(store.ApplyRewardForAccount("synthetic", 1, kind + "-bulk", "match-1", kind, 2).Result != MagNetResult.Success,
                    "Event reward accepted multiple units");
            }
            Catalog(path, new Dictionary<string, object>());
            store.ProgressionCatalog = MagProgressionCatalog.Load(path, 20231);
            foreach (string kind in new[] { "enemy_finished", "teammate_resuscitated" })
            {
                var settled = store.ApplyRewardForAccount("synthetic", 1, kind + "-missing", "match-1", kind, 1);
                Check(settled.Result == MagNetResult.Success && settled.Reason == "reward-committed" &&
                    settled.ExperienceAwarded == 0 && settled.Experience == 10 && settled.MatchExperience == 10 && settled.MatchFraction == 0.25f,
                    "Missing optional award must settle without XP credit or disturbing fractional carry");
                var duplicate = store.ApplyRewardForAccount("synthetic", 1, kind + "-missing", "match-1", kind, 1);
                Check(duplicate.Result == MagNetResult.Success && duplicate.Duplicate && duplicate.ExperienceAwarded == 0 &&
                    duplicate.Experience == 10 && duplicate.MatchExperience == 10 && duplicate.MatchFraction == 0.25f,
                    "Missing optional award receipt must remain durably deduplicated without credit");
            }
            Check(store.ApplyRewardForAccount("synthetic", 1, "unknown-action", "match-1", "unknown_action", 1).Result != MagNetResult.Success,
                "Unknown reward kind must remain denied");
            foreach (string key in new[] { "FinishingMoves", "Resuscitations" })
            {
                Catalog(path, new Dictionary<string, object> { { key, -1 } });
                try { MagProgressionCatalog.Load(path, 20231); }
                catch (InvalidDataException) { continue; }
                throw new Exception("Negative optional reward admitted: " + key);
            }
        }
        finally { Directory.Delete(directory, true); }
    }
}
