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
                store = Program.CreateCharacterStore();
            }
            catch (Exception e) when (e is IOException || e is InvalidDataException || e is JsonException || e is ArgumentException || e is UnauthorizedAccessException)
            {
                Console.Error.WriteLine("mag-admin: configuration failed: " + e.Message);
                return 2;
            }
            string backups = Path.Combine(Path.GetDirectoryName(Path.GetFullPath(storePath)), ".maintenance-backups");
            string markers = Environment.GetEnvironmentVariable("MAG_ACTIVE_ACCOUNTS_DIR") ?? "/tmp";
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

        private static string Online(IReadOnlyDictionary<string, DateTime> active, string account) =>
            active == null ? "unknown" : active.ContainsKey(account) ? "online" : "offline";

        private static int List(LocalCharacterStore store, string markerDirectory, DateTime now, TextWriter output)
        {
            var active = MagActiveAccounts.Read(markerDirectory, now);
            output.WriteLine($"{"ACCOUNT",-20} {"ID",4} {"NAME",-20} {"FACTION",7} {"LEVEL",5} {"XP",7} {"SKILLPTS",8}  SESSION");
            foreach (var c in store.ReadAll().OrderBy(c => c.Id))
            {
                var r = c.ToRecord();
                output.WriteLine($"{c.Account,-20} {c.Id,4} {c.Name,-20} {c.Faction,7} {r.Level,5} {r.Experience,7} {r.SkillPoints,8}  {Online(active, c.Account)}");
            }
            return 0;
        }

        private static int Show(LocalCharacterStore store, string account, string markerDirectory, DateTime now, TextWriter output, TextWriter error)
        {
            var c = store.ReadAll().SingleOrDefault(x => x.Account == account);
            if (c == null) { error.WriteLine($"mag-admin: account {account} has no character"); return 1; }
            var r = c.ToRecord();
            var catalog = store.ProgressionCatalog;
            output.WriteLine($"account:      {c.Account}");
            output.WriteLine($"character:    {c.Id} {c.Name} (faction {c.Faction})");
            output.WriteLine($"level:        {r.Level}");
            output.WriteLine($"experience:   {r.Experience}");
            output.WriteLine($"skill points: {r.SkillPoints}");
            if (catalog != null)
                output.WriteLine(r.Level < catalog.MaximumLevel
                    ? $"next level:   {r.Level + 1} at {catalog.ThresholdForLevel((byte)(r.Level + 1))} XP"
                    : "next level:   maximum level reached");
            output.WriteLine($"session:      {Online(MagActiveAccounts.Read(markerDirectory, now), c.Account)}");
            return 0;
        }

        private static int SetLevel(LocalCharacterStore store, string account, byte level, bool apply, string backupDirectory,
            string markerDirectory, DateTime now, TextWriter output, TextWriter error)
        {
            var active = MagActiveAccounts.Read(markerDirectory, now);
            if (apply && active == null)
            {
                error.WriteLine($"mag-admin: refused: no session marker in {markerDirectory}; cannot confirm {account} is logged out");
                return 1;
            }
            if (active != null && active.TryGetValue(account, out var until))
            {
                if (apply)
                {
                    error.WriteLine($"mag-admin: refused: {account} has a live login (lease until {until:O}); log it out and retry");
                    return 1;
                }
                output.WriteLine($"warning: {account} is logged in; --apply will be refused until it logs out");
            }
            var change = store.RaiseLevelForAdmin(account, level, apply, backupDirectory);
            if (change.Refusal) { error.WriteLine($"mag-admin: refused: {change.Reason}"); return 1; }
            string verb = change.Applied ? "applied" : change.Reason == "unchanged" ? "unchanged" : "dry run (add --apply to write)";
            output.WriteLine($"{verb}: {change.Account} ({change.CharacterId} {change.Name})");
            output.WriteLine($"  level        {change.PreviousLevel} -> {change.Level}");
            output.WriteLine($"  experience   {change.PreviousExperience} -> {change.Experience}");
            output.WriteLine($"  skill points {change.PreviousSkillPoints} -> {change.SkillPoints}");
            if (change.BackupPath != null) output.WriteLine($"  backup       {change.BackupPath}");
            return 0;
        }
    }
}
