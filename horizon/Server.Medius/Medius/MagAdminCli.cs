using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Newtonsoft.Json;

namespace Server.Medius
{
    /// <summary>
    /// Operator CLI for local account/character maintenance, run inside the medius container
    /// (for example <c>kubectl exec … -- mag-admin accounts list</c>). It reuses the server's own
    /// store, lock, catalogs and atomic commit; it never starts listeners or rewrites medius.json.
    /// </summary>
    public static class MagAdminCli
    {
        public const string Usage =
@"usage: mag-admin [--config DIR] <command>

  accounts list                              Every account's character, level, XP and skill points.
  character show <account>                   One character in detail, with the next level threshold.
  character set-level <account> <level>      Raise a character to <level> as earned progression would:
        [--apply]                            XP becomes the level threshold and each gained level's skill
                                             points are added. Without --apply this is a dry run.

  character-store status                     The configured backend (MAG_CHARACTER_STORE) and its schema.
  character-store migrate                    Apply pending PostgreSQL schema migrations.
  character-store import --from <json>       Copy a JSON store into the empty PostgreSQL store, then verify.
  character-store verify --json <json>       Compare a JSON store with PostgreSQL by record and field name.
  character-store export --to <json>         Write PostgreSQL to a JSON store (rollback); --replace keeps
        [--replace]                          a timestamped copy of an existing file and replaces it.
  character-store prune                      Run PostgreSQL reward-ledger retention now.

--config defaults to /config/main (the medius config directory). A write is refused while the
account has a live login (session markers in $MAG_ACTIVE_ACCOUNTS_DIR, default /tmp), and lowering
a level is refused. Applied changes first copy the store to .maintenance-backups/ beside it.";

        /// <summary>
        /// Command line for <c>Server.Medius.dll admin …</c>; loads configuration like the server.
        /// Deliberately not named Main: a synchronous Main would replace Program's async entry point.
        /// </summary>
        public static int RunCommandLine(string[] args)
        {
            string config = "/config/main";
            var rest = new List<string>();
            for (int i = 0; i < args.Length; i++)
            {
                if (args[i] == "--config" && i + 1 < args.Length) { config = args[++i]; continue; }
                if (args[i] == "--help" || args[i] == "-h") { Console.Out.WriteLine(Usage); return 0; }
                rest.Add(args[i]);
            }
            LocalCharacterStore store;
            string storePath;
            bool storeCommand = rest.Count > 0 && rest[0] == "character-store";
            try
            {
                string file = Path.Combine(config, "medius.json");
                if (!File.Exists(file)) { Console.Error.WriteLine($"mag-admin: no medius.json in {config}"); return 2; }
                // Read only: the server's RefreshConfig would also rewrite the file.
                var settings = new Config.ServerSettings();
                JsonConvert.PopulateObject(File.ReadAllText(file), settings,
                    new JsonSerializerSettings { MissingMemberHandling = MissingMemberHandling.Ignore });
                Program.Settings = settings;
                storePath = settings.MagCharacterStorePath;
                if (string.IsNullOrWhiteSpace(storePath)) { Console.Error.WriteLine("mag-admin: MagCharacterStorePath is not configured"); return 2; }
                Program.LoadCharacterCatalog();
                // Maintenance commands manage the schema themselves; everything else needs it current.
                Program.ConfigureCharacterBackend(storeCommand ? Program.CharacterSchemaPolicy.None : Program.CharacterSchemaPolicy.Require);
                store = Program.CreateCharacterStore();
            }
            catch (Exception e) when (e is IOException || e is InvalidDataException || e is JsonException || e is ArgumentException || e is UnauthorizedAccessException)
            {
                Console.Error.WriteLine("mag-admin: configuration failed: " + e.Message);
                return 2;
            }
            if (storeCommand)
                return RunCharacterStore(rest.Skip(1).ToArray(), Program.CharacterDatabase, Program.CreateCharacterStore,
                    Program.CreateJsonCharacterStore, Console.Out, Console.Error);
            string backups = MagAdminOperations.BackupDirectory(storePath);
            string markers = MagAdminOperations.MarkerDirectory();
            return Run(rest.ToArray(), store, backups, markers, DateTime.UtcNow, Console.Out, Console.Error);
        }

        /// <summary>Exit codes: 0 success or dry run, 1 refused or failed, 2 usage.</summary>
        public static int Run(string[] args, LocalCharacterStore store, string backupDirectory, string markerDirectory,
            DateTime now, TextWriter output, TextWriter error)
        {
            bool apply = args.Contains("--apply");
            var words = args.Where(a => a != "--apply").ToArray();
            try
            {
                switch (words)
                {
                    case ["accounts", "list"]:
                        return List(store, markerDirectory, now, output);
                    case ["character", "show", var account]:
                        return Show(store, account, markerDirectory, now, output, error);
                    case ["character", "set-level", var account, var levelText]:
                        if (!byte.TryParse(levelText, out byte level)) { error.WriteLine("mag-admin: level must be a number from 1 to 255"); return 2; }
                        return SetLevel(store, account, level, apply, backupDirectory, markerDirectory, now, output, error);
                    default:
                        error.WriteLine(Usage);
                        return 2;
                }
            }
            catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
            {
                error.WriteLine("mag-admin: " + e.Message);
                return 1;
            }
        }

        /// <summary>
        /// <c>character-store</c> maintenance (docs/admin-cli.md). Prints counts, digests and record or field
        /// names only, never player data or credentials. Exit codes as for <see cref="Run"/>.
        /// </summary>
        public static int RunCharacterStore(string[] args, PostgresCharacterDatabase database, Func<LocalCharacterStore> configured,
            Func<string, LocalCharacterStore> json, TextWriter output, TextWriter error)
        {
            string Option(string name)
            {
                int index = Array.IndexOf(args, name);
                return index >= 0 && index + 1 < args.Length ? args[index + 1] : null;
            }
            string command = args.Length > 0 ? args[0] : null;
            if (command != "status" && database == null)
            {
                error.WriteLine("mag-admin: character-store commands other than status need MAG_CHARACTER_STORE=postgres");
                return 2;
            }
            try
            {
                switch (command)
                {
                    case "status":
                        if (database == null) { output.WriteLine("backend: json"); return 0; }
                        output.WriteLine($"backend: postgres {database.Target}{(database.ReadOnly ? " (read-only)" : "")}");
                        output.WriteLine($"schema:  {database.SchemaVersion()} of {database.LatestSchemaVersion}");
                        return 0;
                    case "migrate":
                        int applied = database.Migrate();
                        output.WriteLine($"migrated: {applied} applied, schema {database.SchemaVersion()} of {database.LatestSchemaVersion}");
                        return 0;
                    case "import" when Option("--from") is string from:
                    {
                        database.RequireCurrentSchema();
                        var target = configured(); var source = json(from);
                        output.WriteLine($"target: {database.Target}");
                        output.WriteLine("imported: " + target.ImportFrom(source).Describe());
                        return Report(target.CompareWith(source), output, error);
                    }
                    case "verify" when Option("--json") is string path:
                        database.RequireCurrentSchema();
                        return Report(configured().CompareWith(json(path)), output, error);
                    case "export" when Option("--to") is string to:
                        database.RequireCurrentSchema();
                        output.WriteLine("exported: " + configured().ExportTo(json(to), args.Contains("--replace")).Describe());
                        return 0;
                    case "prune":
                        database.RequireCurrentSchema();
                        output.WriteLine(database.PruneLedger(Program.LedgerRetention).Describe());
                        return 0;
                    default:
                        error.WriteLine(Usage);
                        return 2;
                }
            }
            catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException || e is InvalidOperationException)
            {
                error.WriteLine("mag-admin: " + e.Message);
                return 1;
            }
        }

        private static int Report(IReadOnlyList<string> differences, TextWriter output, TextWriter error)
        {
            if (differences.Count == 0) { output.WriteLine("verified: the JSON and PostgreSQL stores agree"); return 0; }
            foreach (var difference in differences) error.WriteLine("difference: " + difference);
            error.WriteLine($"mag-admin: {differences.Count} difference(s) between the stores");
            return 1;
        }

        private static int List(LocalCharacterStore store, string markerDirectory, DateTime now, TextWriter output)
        {
            output.WriteLine($"{"ACCOUNT",-20} {"ID",4} {"NAME",-20} {"FACTION",7} {"LEVEL",5} {"XP",7} {"SKILLPTS",8}  SESSION");
            foreach (var c in MagAdminOperations.ListAccounts(store, markerDirectory, now))
                output.WriteLine($"{c.Account,-20} {c.CharacterId,4} {c.Name,-20} {c.Faction,7} {c.Level,5} {c.Experience,7} {c.SkillPoints,8}  {c.Session}");
            return 0;
        }

        private static int Show(LocalCharacterStore store, string account, string markerDirectory, DateTime now, TextWriter output, TextWriter error)
        {
            var c = MagAdminOperations.Show(store, account, markerDirectory, now);
            if (c == null) { error.WriteLine($"mag-admin: account {account} has no character"); return 1; }
            output.WriteLine($"account:      {c.Account}");
            output.WriteLine($"character:    {c.CharacterId} {c.Name} (faction {c.Faction})");
            output.WriteLine($"level:        {c.Level}");
            output.WriteLine($"experience:   {c.Experience}");
            output.WriteLine($"skill points: {c.SkillPoints}");
            if (c.HasProgressionCatalog)
                output.WriteLine(c.NextLevel != null
                    ? $"next level:   {c.NextLevel} at {c.NextLevelExperience} XP"
                    : "next level:   maximum level reached");
            output.WriteLine($"session:      {c.Session}");
            return 0;
        }

        private static int SetLevel(LocalCharacterStore store, string account, byte level, bool apply, string backupDirectory,
            string markerDirectory, DateTime now, TextWriter output, TextWriter error)
        {
            var result = MagAdminOperations.SetLevel(store, account, level, apply, backupDirectory, markerDirectory, now);
            // The CLI keeps reporting an unchanged level as a successful no-op.
            if (result.Refusal != MagAdminRefusal.None && result.Refusal != MagAdminRefusal.SameLevel)
            {
                error.WriteLine($"mag-admin: refused: {result.Message}");
                return 1;
            }
            if (result.OnlineWarning) output.WriteLine($"warning: {account} is logged in; --apply will be refused until it logs out");
            var change = result.Change;
            string verb = change.Applied ? "applied" : result.Refusal == MagAdminRefusal.SameLevel ? "unchanged" : "dry run (add --apply to write)";
            output.WriteLine($"{verb}: {change.Account} ({change.CharacterId} {change.Name})");
            output.WriteLine($"  level        {change.PreviousLevel} -> {change.Level}");
            output.WriteLine($"  experience   {change.PreviousExperience} -> {change.Experience}");
            output.WriteLine($"  skill points {change.PreviousSkillPoints} -> {change.SkillPoints}");
            if (change.BackupPath != null) output.WriteLine($"  backup       {change.BackupPath}");
            return 0;
        }
    }
}
