using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using RT.Models.MAG;
using Server.Medius;

// Synthetic four-level catalog (invented thresholds): an operator starting level makes new
// characters start exactly as if they had earned that level, and bad values fail fast.
internal static class StartingLevelChecks
{
    static void Check(bool value, string message) { if (!value) throw new Exception(message); }

    static MagProgressionCatalog Catalog(string path)
    {
        object Level(int xp, int points) => new { ExperienceLevel = new { Experience = xp, SkillPointsGained = points, LoadoutBudget = 0, Rank = 0 } };
        File.WriteAllText(path, JsonSerializer.Serialize(new {
            schema_version = 1, application_id = 20231,
            table = new {
                ExperienceRequiredScale = 1,
                Levels = new { ExperienceLevel1 = Level(0, 1), ExperienceLevel2 = Level(100, 1), ExperienceLevel3 = Level(250, 2), ExperienceLevel4 = Level(400, 1) },
                Awards = new Dictionary<string, object> {
                    { "EnemyKilled", 3 }, { "ExperienceToHealOtherRatio", 0.25f },
                    { "PrimaryObjectiveActivated", 1 }, { "PrimaryObjectiveSecured", 1 },
                    { "PrimaryObjective", 1 }, { "SecondaryObjective", 1 },
                    { "ChargePlanted", 1 }, { "ChargeDefused", 1 },
                    { "VictoryModifier", new { MajorVictory = 1, MinorVictory = 1, MajorDefeat = 0, MinorDefeat = 0 } }
                }
            },
            creation_settings = new { root_defaults = new { StartingLevel = 1, AdditionalSkillPoints = 0 } }
        }));
        return MagProgressionCatalog.Load(path, 20231);
    }

    static MagCharacterRecord Create(string directory, MagProgressionCatalog catalog, byte? startingLevel, string account)
    {
        string storePath = Path.Combine(directory, account + ".json");
        var store = new LocalCharacterStore(storePath) { ProgressionCatalog = catalog, StartingLevel = startingLevel };
        Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Example", Faction = 0, UniverseId = 26, NpId = new byte[36] },
            account).Result == MagNetResult.Success, "Synthetic character creation");
        // A fresh store must load it: the persisted level matches the catalog for its XP.
        return new LocalCharacterStore(storePath) { ProgressionCatalog = catalog }.ReadAll().Single().ToRecord();
    }

    public static void Run()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-starting-level-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var catalog = Catalog(Path.Combine(directory, "progression.json"));
            var standard = Create(directory, catalog, null, "standard");
            Check(standard.Level == 1 && standard.Experience == 0 && standard.SkillPoints == 1, "Unset keeps the catalog start");
            var raised = Create(directory, catalog, 3, "raised");
            Check(raised.Level == 3 && raised.Experience == 250 && raised.SkillPoints == 1 + 1 + 2,
                $"Starting level 3 must match earned progression, got {raised.Level}/{raised.Experience}/{raised.SkillPoints}");
            var same = Create(directory, catalog, 1, "same");
            Check(same.Level == 1 && same.Experience == 0 && same.SkillPoints == 1, "Starting level equal to the catalog start is a no-op");

            Check(Server.Medius.Program.ParseStartingLevel(null, catalog) == null && Server.Medius.Program.ParseStartingLevel(" ", catalog) == null, "Unset parses to null");
            Check(Server.Medius.Program.ParseStartingLevel("3", catalog) == 3, "Valid level parses");
            foreach (var bad in new[] { "0", "5", "x", "-1", "300" })
            {
                bool threw = false; try { Server.Medius.Program.ParseStartingLevel(bad, catalog); } catch (InvalidDataException) { threw = true; }
                Check(threw, "Invalid MAG_STARTING_LEVEL must fail startup: " + bad);
            }
            bool needsCatalog = false; try { Server.Medius.Program.ParseStartingLevel("3", null); } catch (InvalidDataException) { needsCatalog = true; }
            Check(needsCatalog, "A starting level without a progression catalog must fail startup");
        }
        finally { Directory.Delete(directory, true); }
    }
}
