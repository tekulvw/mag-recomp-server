using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using RT.Models.MAG;
using Server.Medius;

// Synthetic four-level catalog with an invented Training bonus: an operator Training level raises
// a character completing Training to at least that level as if it had earned it, never lowers a
// character already past it, awards once, and bad values fail fast.
internal static class TrainingLevelChecks
{
    const byte CompletedFlag = 0x04;
    const int Bonus = 120;

    static void Check(bool value, string message) { if (!value) throw new Exception(message); }

    internal static string Catalog(string path, int levels)
    {
        object Level(int xp, int points) => new { ExperienceLevel = new { Experience = xp, SkillPointsGained = points, LoadoutBudget = 0, Rank = 0 } };
        var rows = new Dictionary<string, object> { { "ExperienceLevel1", Level(0, 1) }, { "ExperienceLevel2", Level(100, 1) }, { "ExperienceLevel3", Level(250, 2) } };
        if (levels > 3) rows.Add("ExperienceLevel4", Level(400, 1));
        File.WriteAllText(path, JsonSerializer.Serialize(new {
            schema_version = 1, application_id = 20231,
            training_completion = new { completed_flag_mask = CompletedFlag },
            table = new {
                ExperienceRequiredScale = 1, Training_ExperienceBonus = Bonus,
                Levels = rows,
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
        return path;
    }

    static LocalCharacterStore Store(string path, MagTrainingCompletionCatalog training, byte? startingLevel, byte? trainingLevel) =>
        new LocalCharacterStore(path) { ProgressionCatalog = training.Progression, TrainingCompletionCatalog = training,
            StartingLevel = startingLevel, TrainingLevel = trainingLevel };

    // Creates one character (optionally at a starting level), completes Training twice and returns
    // the record a fresh store reads back after the first completion.
    static MagCharacterRecord Complete(string directory, MagTrainingCompletionCatalog training, byte? startingLevel, byte? trainingLevel, string account)
    {
        string path = Path.Combine(directory, account + ".json");
        var store = Store(path, training, startingLevel, trainingLevel);
        Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Example", Faction = 0, UniverseId = 26, NpId = new byte[36] },
            account).Result == MagNetResult.Success, "Synthetic character creation");
        var first = store.CompleteTrainingForAccount(account, 1);
        Check(first.Result == MagNetResult.Success && first.NewlyCompleted && first.Reason == "training-completed", "First Training completion: " + first.Reason);
        var second = store.CompleteTrainingForAccount(account, 1);
        Check(second.Result == MagNetResult.Success && !second.NewlyCompleted && second.Reason == "training-already-completed",
            "Repeated Training completion must acknowledge without a new award: " + second.Reason);
        Check(second.Record.Level == first.Record.Level && second.Record.Experience == first.Record.Experience &&
            second.Record.SkillPoints == first.Record.SkillPoints, "Repeated Training completion must award nothing");
        // A fresh store must load it: the persisted level matches the catalog for its XP.
        var stored = Store(path, training, startingLevel, trainingLevel).ReadAll().Single().ToRecord();
        Check((stored.Flags & CompletedFlag) != 0, "Training completion flag must persist");
        Check(stored.Level == first.Record.Level && stored.Experience == first.Record.Experience && stored.SkillPoints == first.Record.SkillPoints,
            "The returned record must be the persisted one");
        return stored;
    }

    public static void Run()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-training-level-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var training = MagTrainingCompletionCatalog.Load(Catalog(Path.Combine(directory, "training.json"), 4));
            var catalog = training.Progression;

            var standard = Complete(directory, training, null, null, "standard");
            Check(standard.Level == 2 && standard.Experience == Bonus && standard.SkillPoints == 1 + 1,
                $"Unset keeps the bonus alone, got {standard.Level}/{standard.Experience}/{standard.SkillPoints}");
            var raised = Complete(directory, training, null, 3, "raised");
            Check(raised.Level == 3 && raised.Experience == 250 && raised.SkillPoints == 1 + 1 + 2,
                $"Training level 3 must match earned progression, got {raised.Level}/{raised.Experience}/{raised.SkillPoints}");
            var reached = Complete(directory, training, null, 2, "reached");
            Check(reached.Level == 2 && reached.Experience == Bonus && reached.SkillPoints == 1 + 1,
                $"A bonus that already reaches the Training level keeps its XP, got {reached.Level}/{reached.Experience}/{reached.SkillPoints}");
            var above = Complete(directory, training, 4, 3, "above");
            Check(above.Level == 4 && above.Experience == 400 + Bonus && above.SkillPoints == 1 + 1 + 2 + 1,
                $"A character past the Training level keeps only the bonus, got {above.Level}/{above.Experience}/{above.SkillPoints}");

            Check(Server.Medius.Program.ParseTrainingLevel(null, catalog, training) == null &&
                Server.Medius.Program.ParseTrainingLevel(" ", catalog, null) == null, "Unset parses to null");
            Check(Server.Medius.Program.ParseTrainingLevel("3", catalog, training) == 3, "Valid level parses");
            Check(Server.Medius.Program.ParseTrainingLevel("1", catalog, training) == 1, "Level one is a valid no-op");
            var shorter = MagTrainingCompletionCatalog.Load(Catalog(Path.Combine(directory, "short.json"), 3));
            var refusals = new (string Value, MagProgressionCatalog Catalog, MagTrainingCompletionCatalog Training, string Case)[] {
                ("0", catalog, training, "zero"), ("5", catalog, training, "above maximum"), ("x", catalog, training, "non-numeric"),
                ("-1", catalog, training, "negative"), ("300", catalog, training, "beyond a byte"),
                ("3", null, training, "missing progression catalog"), ("3", catalog, null, "missing Training catalog"),
                ("4", catalog, shorter, "above the Training catalog's maximum") };
            foreach (var bad in refusals)
            {
                string message = null;
                try { Server.Medius.Program.ParseTrainingLevel(bad.Value, bad.Catalog, bad.Training); }
                catch (InvalidDataException e) { message = e.Message; }
                Check(message != null && message.Contains("MAG_TRAINING_LEVEL"), "Invalid MAG_TRAINING_LEVEL must fail startup naming it: " + bad.Case);
            }
        }
        finally { Directory.Delete(directory, true); }
    }
}
