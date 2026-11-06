using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using RT.Models.MAG;
using Server.Medius;

// Synthetic progression (invented thresholds) and accounts exercise mag-admin end to end:
// dry run, applied raise with backup, refusals and the live-login session markers.
internal static class AdminCliChecks
{
    static void Check(bool value, string message) { if (!value) throw new Exception(message); }

    static void Catalog(string path) => File.WriteAllText(path, JsonSerializer.Serialize(new {
        schema_version = 1, application_id = 20231,
        table = new {
            ExperienceRequiredScale = 1,
            Levels = new {
                ExperienceLevel1 = new { ExperienceLevel = new { Experience = 0, SkillPointsGained = 1, LoadoutBudget = 0, Rank = 0 } },
                ExperienceLevel2 = new { ExperienceLevel = new { Experience = 100, SkillPointsGained = 1, LoadoutBudget = 0, Rank = 0 } },
                ExperienceLevel3 = new { ExperienceLevel = new { Experience = 250, SkillPointsGained = 2, LoadoutBudget = 0, Rank = 0 } },
                ExperienceLevel4 = new { ExperienceLevel = new { Experience = 400, SkillPointsGained = 1, LoadoutBudget = 0, Rank = 0 } }
            },
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

    static (int code, string output, string error) Admin(LocalCharacterStore store, string backups, string markers, DateTime now, params string[] args)
    {
        var output = new StringWriter(); var error = new StringWriter();
        int code = MagAdminCli.Run(args, store, backups, markers, now, output, error);
        return (code, output.ToString(), error.ToString());
    }

    public static void Run()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-admin-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var now = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
            string catalogPath = Path.Combine(directory, "progression.json"); Catalog(catalogPath);
            var progression = MagProgressionCatalog.Load(catalogPath, 20231);
            string storePath = Path.Combine(directory, "characters.json");
            LocalCharacterStore Store() => new LocalCharacterStore(storePath) { ProgressionCatalog = progression };
            var store = Store();
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Example", Faction = 0, UniverseId = 26, NpId = new byte[36] },
                "synthetic").Result == MagNetResult.Success, "Synthetic character creation");
            string backups = Path.Combine(directory, ".maintenance-backups");
            string markers = Path.Combine(directory, "markers"); Directory.CreateDirectory(markers);
            string marker = Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json");
            MagRecord Current() { var r = Store().ReadAll().Single().ToRecord(); return new MagRecord(r.Level, r.Experience, r.SkillPoints); }

            // Without any marker, session state is unknown: dry runs work, writes are refused.
            var unknown = Admin(store, backups, markers, now, "character", "set-level", "synthetic", "3", "--apply");
            Check(unknown.code == 1 && unknown.error.Contains("no session marker"), "Write without a session marker must be refused");
            MagActiveAccounts.Write(marker, new Dictionary<string, DateTime>(), now);

            var listed = Admin(store, backups, markers, now, "accounts", "list");
            Check(listed.code == 0 && listed.output.Contains("synthetic") && listed.output.Contains("offline"), "List shows the offline account");
            var shown = Admin(store, backups, markers, now, "character", "show", "synthetic");
            Check(shown.code == 0 && shown.output.Contains("next level:   2 at 100 XP"), "Show reports the next threshold");

            byte[] original = File.ReadAllBytes(storePath);
            var dry = Admin(store, backups, markers, now, "character", "set-level", "synthetic", "3");
            Check(dry.code == 0 && dry.output.Contains("dry run") && dry.output.Contains("1 -> 3"), "Dry run reports the plan");
            Check(File.ReadAllBytes(storePath).SequenceEqual(original) && !Directory.Exists(backups), "Dry run must not write or back up");

            // A live lease refuses the write; an expired one does not count.
            MagActiveAccounts.Write(marker, new Dictionary<string, DateTime> { ["synthetic"] = now.AddSeconds(60) }, now);
            var online = Admin(store, backups, markers, now, "character", "set-level", "synthetic", "3", "--apply");
            Check(online.code == 1 && online.error.Contains("live login") && File.ReadAllBytes(storePath).SequenceEqual(original),
                "Logged-in account must be refused without a write");
            Check(Admin(store, backups, markers, now, "character", "set-level", "synthetic", "3").output.Contains("warning"), "Dry run warns while online");
            MagActiveAccounts.Write(marker, new Dictionary<string, DateTime> { ["synthetic"] = now.AddSeconds(-1) }, now);

            var applied = Admin(store, backups, markers, now, "character", "set-level", "synthetic", "3", "--apply");
            Check(applied.code == 0 && applied.output.Contains("applied"), "Applied raise " + applied.error);
            var after = Current();
            Check(after.Level == 3 && after.Experience == 250 && after.SkillPoints == 1 + 1 + 2,
                $"Raise must match earned progression, got {after.Level}/{after.Experience}/{after.SkillPoints}");
            var backup = Directory.GetFiles(backups).Single();
            Check(File.ReadAllBytes(backup).SequenceEqual(original) && Path.GetFileName(backup).StartsWith("characters.before-admin-set-level-synthetic-3-"),
                "Backup must hold the exact previous store");

            Check(Admin(store, backups, markers, now, "character", "set-level", "synthetic", "3", "--apply").output.Contains("unchanged"), "Same level is a no-op");
            var lower = Admin(store, backups, markers, now, "character", "set-level", "synthetic", "2", "--apply");
            Check(lower.code == 1 && lower.error.Contains("lowering-unsupported"), "Lowering must be refused");
            Check(Admin(store, backups, markers, now, "character", "set-level", "synthetic", "5", "--apply").error.Contains("level-out-of-range"), "Beyond the catalog must be refused");
            Check(Admin(store, backups, markers, now, "character", "set-level", "nobody", "2", "--apply").error.Contains("account-not-found"), "Unknown account refused");
            Check(Admin(store, backups, markers, now, "character", "wipe", "synthetic").code == 2, "Unknown command is a usage error");
            var bare = new LocalCharacterStore(storePath);
            Check(bare.RaiseLevelForAdmin("synthetic", 4, false, backups).Reason == "progression-catalog-unavailable", "No catalog, no level edit");

            // The server-side registry publishes account leases for the CLI.
            var registry = new MagLoginRegistry();
            string registryMarker = Path.Combine(markers, MagActiveAccounts.FilePrefix + "registry.json");
            registry.PublishActiveAccountsTo(registryMarker, now);
            Check(MagActiveAccounts.Read(markers, now).Count == 0, "Fresh registry publishes no accounts");
            var ticket = registry.IssueForAccount("synthetic", 1, 1, now);
            Check(MagActiveAccounts.Read(markers, now).ContainsKey("synthetic"), "Issued login ticket is published");
            Check(registry.TryBind(ticket.AccessToken, ticket.SessionKey, ticket.ApplicationId, 1, "synthetic-channel", now, out _), "Synthetic bind");
            Check(MagActiveAccounts.Read(markers, now.AddSeconds(90))["synthetic"] > now.AddSeconds(90), "Bound lease extends past the pending deadline");
            registry.Release("synthetic-channel");
            Check(!MagActiveAccounts.Read(markers, now).ContainsKey("synthetic"), "Released login is unpublished");
        }
        finally { Directory.Delete(directory, true); }
    }

    record MagRecord(byte Level, uint Experience, byte SkillPoints);
}
