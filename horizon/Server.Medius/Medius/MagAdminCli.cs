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

  accounts list                              Every character: account, slot, name, level, XP, skill points,
                                             session, and whether the player deleted it.
  character show <character> [--slot N]      One character in detail, with the next level threshold, and
                                             every character of its account.
  character set-level <character> <level>    Raise a character to <level> as earned progression would:
        [--slot N] [--apply]                 XP becomes the level threshold and each gained level's skill
                                             points are added. Without --apply this is a dry run.
  character kill <character> [--slot N]      Kill the character's player in its running match, as if it had
        [--reason TEXT] [--operator NAME]    pressed Kill Self (for a player stuck in a parachute or a seat).
        [--apply]                            Needs the running server's admin API (MAG_ADMIN_API_LISTEN and
                                             MAG_ADMIN_API_SECRET_FILE). Without --apply this is a dry run.
  character restore <account> <name>         Bring back a character the player deleted, into the account's
        [--apply]                            lowest free slot, with everything it had, and its old name if
                                             that is still free.
  character purge <account> <name>           Remove a deleted character for good (its progression, history
        [--apply]                            and kits). Live characters are refused.

  A <character> is an account with one character, an account with --slot N (0 to 2), or a character
  name. Accounts have up to three character slots. A character the player deletes in game keeps its
  row (shown as deleted, its name moved aside as <name>~<id>) until it is restored or purged; restore
  and purge take either name.

  voice list                                 Active voice mutes and bans: account, kind, end, operator, reason.
  voice mute <account>                       Stop relaying the account's in-match voice (both the channel and
  voice ban <account>                        the proximity copy). A mute and a ban act alike; the kind records
        [--for DURATION] [--reason TEXT]     intent. --for takes 30m, 12h, 7d or 2w; without it the sanction
        [--operator NAME] [--apply]          lasts until lifted. Imposing again replaces it. The running server
                                             applies it within 5 s, or at once through the admin API.
  voice unmute <account>                     Lift an active mute or ban.
  voice unban <account>
        [--reason TEXT] [--operator NAME] [--apply]

  character-store status                     The configured backend (MAG_CHARACTER_STORE), its schema, row counts,
                                             contents digest and last import.
  character-store migrate                    Apply pending PostgreSQL schema migrations.
  character-store import --from <json>       Copy a JSON store into PostgreSQL, then verify. Idempotent: an empty
        [--dry-run]                          database is filled, one already equal is left alone, and one that
        [--replace]                          differs is refused. --dry-run writes nothing. --replace replaces rows
        [--discard-postgres-changes]         unless the server wrote after the last import; adding
                                             --discard-postgres-changes replaces them anyway.
  character-store verify --json <json>       Compare a JSON store with PostgreSQL: counts, digests, and differences
        [--as-export]                        by record and field name. --as-export checks an export: settled
                                             history beyond the JSON bound may be missing, all else must match.
  character-store export --to <json>         Write PostgreSQL to a JSON store (rollback), read it back and verify
        [--replace]                          it; exit 1 if it is not a complete copy. --replace keeps a
                                             timestamped copy of an existing file and replaces it.
  character-store prune                      Run PostgreSQL reward-ledger retention now.

--config defaults to /config/main (the medius config directory). A write is refused while the
account has a live login (session markers in $MAG_ACTIVE_ACCOUNTS_DIR, default /tmp), and lowering
a level is refused. Applied changes first copy the store to .maintenance-backups/ beside it (the
PostgreSQL store journals them in mag.admin_changes instead). Voice commands work while the account
is online, and without --apply they are dry runs too.";

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
            MagAdminApiClient api;
            try { api = MagAdminApiClient.FromEnvironment(); }
            catch (ArgumentException e) { Console.Error.WriteLine("mag-admin: admin API configuration failed: " + e.Message); return 2; }
            return Run(rest.ToArray(), store, backups, markers, DateTime.UtcNow, Console.Out, Console.Error, api);
        }

        /// <summary>Exit codes: 0 success or dry run, 1 refused or failed, 2 usage.</summary>
        public static int Run(string[] args, LocalCharacterStore store, string backupDirectory, string markerDirectory,
            DateTime now, TextWriter output, TextWriter error, MagAdminApiClient api = null)
        {
            bool apply = args.Contains("--apply");
            string reason = null, operatorName = null, duration = null;
            int? slot = null;
            var words = new List<string>();
            for (int i = 0; i < args.Length; i++)
            {
                if (args[i] == "--apply") continue;
                if (args[i] == "--reason" || args[i] == "--operator" || args[i] == "--for")
                {
                    if (i + 1 >= args.Length) { error.WriteLine(Usage); return 2; }
                    if (args[i] == "--reason") reason = args[++i]; else if (args[i] == "--operator") operatorName = args[++i]; else duration = args[++i];
                    continue;
                }
                if (args[i] == "--slot")
                {
                    if (i + 1 >= args.Length || !byte.TryParse(args[++i], out byte parsed) || parsed >= LocalCharacterStore.CharacterSlots)
                    { error.WriteLine($"mag-admin: --slot must be a number from 0 to {LocalCharacterStore.CharacterSlots - 1}"); return 2; }
                    slot = parsed;
                    continue;
                }
                words.Add(args[i]);
            }
            bool slotCommand = words.Count >= 3 && words[0] == "character" && words[1] is "show" or "set-level" or "kill";
            if (slot != null && !slotCommand) { error.WriteLine(Usage); return 2; }
            bool killOptions = reason != null || operatorName != null;
            try
            {
                switch (words.ToArray())
                {
                    case ["voice", var action, var account] when action is "mute" or "ban" or "unmute" or "unban":
                        return Voice(store, action, account, apply, reason, operatorName, duration, backupDirectory, output, error, now);
                    case var _ when duration != null:
                        error.WriteLine(Usage);
                        return 2;
                    case ["character", "kill", var target]:
                        return Kill(api, store, target, slot, apply, reason, operatorName, now, output, error);
                    case var _ when killOptions:
                        error.WriteLine(Usage);
                        return 2;
                    case ["accounts", "list"]:
                        return List(store, markerDirectory, now, output);
                    case ["voice", "list"]:
                        return VoiceList(store, now, output);
                    case ["character", "show", var target]:
                        return Show(store, target, slot, markerDirectory, now, output, error);
                    case ["character", "set-level", var target, var levelText]:
                        if (!byte.TryParse(levelText, out byte level)) { error.WriteLine("mag-admin: level must be a number from 1 to 255"); return 2; }
                        return SetLevel(store, target, slot, level, apply, backupDirectory, markerDirectory, now, output, error);
                    case ["character", var action, var account, var name] when action is "restore" or "purge":
                        return RestoreOrPurge(store, action, account, name, apply, backupDirectory, output, error);
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
                        int version = database.SchemaVersion();
                        output.WriteLine($"schema:  {version} of {database.LatestSchemaVersion}");
                        if (version != database.LatestSchemaVersion) return 0;
                        var (tables, lastImport) = database.RowCounts();
                        output.WriteLine("rows:    " + string.Join(", ", tables.Select(t => $"{t.Table} {t.Rows}")));
                        output.WriteLine("contents: " + configured().Summarize().Describe());
                        output.WriteLine("last import: " + (lastImport is DateTime at ? at.ToString("yyyy-MM-ddTHH:mm:ssZ") : "never"));
                        return 0;
                    case "migrate":
                        int applied = database.Migrate();
                        output.WriteLine($"migrated: {applied} applied, schema {database.SchemaVersion()} of {database.LatestSchemaVersion}");
                        return 0;
                    case "import" when Option("--from") is string from:
                    {
                        database.RequireCurrentSchema();
                        var target = configured(); var source = json(from);
                        var options = new CharacterImportOptions(args.Contains("--replace"), args.Contains("--discard-postgres-changes"));
                        if (options.DiscardPostgresChanges && !options.Replace)
                        { error.WriteLine("mag-admin: --discard-postgres-changes needs --replace"); return 2; }
                        output.WriteLine($"target: {database.Target}");
                        if (args.Contains("--dry-run"))
                        {
                            var plan = target.PlanImport(source, options);
                            DescribePlan(plan, output);
                            if (plan.Action == CharacterImportAction.Refused) { error.WriteLine("mag-admin: import would be refused: " + plan.Reason); return 1; }
                            output.WriteLine("dry run: nothing written");
                            return 0;
                        }
                        var result = target.ImportFrom(source, options);
                        DescribePlan(result.Plan, output);
                        output.WriteLine((result.Plan.Action == CharacterImportAction.Unchanged ? "unchanged: " : "imported: ") + result.After.Describe());
                        return Report(target.CompareWith(source), output, error);
                    }
                    case "verify" when Option("--json") is string path:
                    {
                        database.RequireCurrentSchema();
                        var target = configured(); var source = json(path);
                        output.WriteLine("json:     " + source.Summarize().Describe());
                        output.WriteLine("postgres: " + target.Summarize().Describe());
                        if (args.Contains("--as-export")) return ReportExport(target.CompareExport(source), output, error,
                            "verified: the JSON store is a complete export of PostgreSQL",
                            "the JSON store is not a complete export of PostgreSQL (or PostgreSQL changed after it)");
                        return Report(target.CompareWith(source), output, error);
                    }
                    case "export" when Option("--to") is string to:
                    {
                        database.RequireCurrentSchema();
                        var result = configured().ExportTo(json(to), args.Contains("--replace"));
                        output.WriteLine("postgres: " + result.Source.Describe());
                        output.WriteLine("exported: " + result.Exported.Describe());
                        return ReportExport(result.Comparison, output, error,
                            "verified: read back, the export is a complete copy of what was read from PostgreSQL",
                            $"the written export {to} is not a complete copy of PostgreSQL; do not roll back with it");
                    }
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

        private static void DescribePlan(CharacterImportPlan plan, TextWriter output)
        {
            output.WriteLine("source:   " + plan.Source.Describe());
            output.WriteLine("database: " + (plan.Target?.Describe() ?? "empty"));
            output.WriteLine("last import: " + (plan.LastImportAt is DateTime at ? at.ToString("yyyy-MM-ddTHH:mm:ssZ") : "never") +
                (plan.Target != null && plan.Action != CharacterImportAction.Unchanged
                    ? $"; written since: {(plan.ChangedSinceImport ? "yes" : "no")}; {plan.Differences} record(s) differ" : ""));
            output.WriteLine("action:   " + plan.Action switch
            {
                CharacterImportAction.Import => "import into the empty database",
                CharacterImportAction.Unchanged => "none, the stores already agree",
                CharacterImportAction.Replace => "replace the database's rows with the JSON store",
                _ => "refuse",
            });
        }

        /// <summary>An export comparison: what the JSON bound left out, then any difference (exit 1).</summary>
        private static int ReportExport(CharacterExportComparison comparison, TextWriter output, TextWriter error, string verified, string failed)
        {
            output.WriteLine("trimmed:  " + comparison.DescribeTrimmed() + " kept only in PostgreSQL, beyond the JSON store's bound");
            if (comparison.Differences.Count == 0)
            {
                output.WriteLine(verified + " (characters, kits, progression, voice sanctions, open match rows and every kept receipt exactly)");
                return 0;
            }
            foreach (var difference in comparison.Differences) error.WriteLine("difference: " + difference);
            error.WriteLine($"mag-admin: {comparison.Differences.Count} difference(s): {failed}");
            return 1;
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
            output.WriteLine($"{"ACCOUNT",-20} {"SLOT",4} {"ID",4} {"NAME",-20} {"FACTION",7} {"LEVEL",5} {"XP",7} {"SKILLPTS",8}  SESSION");
            foreach (var c in MagAdminOperations.ListAccounts(store, markerDirectory, now))
                output.WriteLine($"{c.Account,-20} {c.Slot,4} {c.CharacterId,4} {c.Name,-20} {c.Faction,7} {c.Level,5} {c.Experience,7} {c.SkillPoints,8}  " +
                    (c.Deleted ? $"deleted {Time(c.DeletedAt)}" : c.Session));
            return 0;
        }

        private static int Show(LocalCharacterStore store, string target, int? slot, string markerDirectory, DateTime now, TextWriter output, TextWriter error)
        {
            var resolved = MagAdminOperations.Resolve(store, target, slot);
            if (resolved.Refusal == MagAdminRefusal.Ambiguous) { error.WriteLine("mag-admin: " + resolved.Message); return 1; }
            var c = resolved.Character == null ? null :
                MagAdminOperations.Show(store, resolved.Character.Account, markerDirectory, now, resolved.Character.Slot);
            if (c == null)
            {
                // An account whose characters are all deleted still lists them.
                var deleted = MagAdminOperations.Characters(store, target, markerDirectory, now);
                if (deleted.Count == 0) { error.WriteLine($"mag-admin: account {target} has no character" + (slot is int n ? $" in slot {n}" : "")); return 1; }
                output.WriteLine($"account:      {target} (no live character)");
                Characters(deleted, output);
                return 1;
            }
            output.WriteLine($"account:      {c.Account}");
            output.WriteLine($"character:    {c.CharacterId} {c.Name} (faction {c.Faction}, slot {c.Slot})");
            output.WriteLine($"level:        {c.Level}");
            output.WriteLine($"experience:   {c.Experience}");
            output.WriteLine($"skill points: {c.SkillPoints}");
            output.WriteLine($"respec:       {RespecSummary(c)}");
            if (c.HasProgressionCatalog)
                output.WriteLine(c.NextLevel != null
                    ? $"next level:   {c.NextLevel} at {c.NextLevelExperience} XP"
                    : "next level:   maximum level reached");
            output.WriteLine($"session:      {c.Session}");
            Characters(MagAdminOperations.Characters(store, c.Account, markerDirectory, now), output);
            return 0;
        }

        static void Characters(IReadOnlyList<MagAdminAccount> characters, TextWriter output)
        {
            output.WriteLine("characters:");
            foreach (var x in characters)
                output.WriteLine($"  slot {x.Slot}  {x.CharacterId,4} {x.Name,-20} faction {x.Faction}  level {x.Level,3}  respec points {x.RespecPoints,6}" +
                    (x.Deleted ? $"  deleted {Time(x.DeletedAt)}" : ""));
        }

        /// <summary>"3000 points (stored 0, floor applied), 1 taken, next costs 3000 (capped from 5000)".</summary>
        static string RespecSummary(MagAdminAccount c) =>
            $"{c.RespecPoints} points" + (c.RespecPoints != c.StoredRespecPoints ? $" (stored {c.StoredRespecPoints}, floor applied)" : "") +
            $", {c.RespecCount} taken" + (c.NextRespecFee is uint fee ? $", next costs {fee}" : "") +
            (c.AuthoredRespecFee is uint authored && authored != c.NextRespecFee ? $" (capped from {authored})" : "");

        private static int RestoreOrPurge(LocalCharacterStore store, string action, string account, string name, bool apply, string backupDirectory,
            TextWriter output, TextWriter error)
        {
            var result = action == "restore"
                ? MagAdminOperations.Restore(store, account, name, apply, backupDirectory)
                : MagAdminOperations.Purge(store, account, name, apply, backupDirectory);
            if (result.Refusal != MagAdminRefusal.None) { error.WriteLine($"mag-admin: refused ({result.Code}): {result.Message}"); return 1; }
            var change = result.Change;
            output.WriteLine((change.Applied ? "applied" : "dry run (add --apply to write)") +
                $": character {action} {change.Account} ({change.CharacterId} {change.Name}, level {change.Level})");
            output.WriteLine("  deleted " + Time(change.DeletedAt is long at ? DateTimeOffset.FromUnixTimeSeconds(at) : null));
            output.WriteLine(action == "restore"
                ? $"  restored into slot {change.Slot}"
                : "  removed for good with its progression, kits and match history");
            if (action == "restore")
                output.WriteLine(change.Name == change.PreviousName
                    ? $"  name    {change.Name} (its old name is taken, so it keeps its deleted name)"
                    : $"  name    {change.PreviousName} -> {change.Name}");
            if (change.BackupPath != null) output.WriteLine($"  backup  {change.BackupPath}");
            return 0;
        }

        /// <summary>Default operator name for the API's actor header and the worker's admin_kill row.</summary>
        static string DefaultOperator()
        {
            string user = Environment.GetEnvironmentVariable("USER") ?? Environment.GetEnvironmentVariable("LOGNAME");
            return string.IsNullOrWhiteSpace(user) ? "mag-admin" : "mag-admin:" + user.Trim();
        }

        private static int Kill(MagAdminApiClient api, LocalCharacterStore store, string target, int? slot, bool apply, string reason,
            string operatorName, DateTime now, TextWriter output, TextWriter error)
        {
            if (api == null)
            {
                error.WriteLine($"mag-admin: character kill acts on the running server through its admin API; set {MagAdminApiSettings.ListenVariable} " +
                    $"and {MagAdminApiSettings.SecretFileVariable} (chart adminApi.enabled) and run it in the main medius container");
                return 2;
            }
            string account = target;
            // A character name or a slot is resolved here; a bare account goes to the server as it is.
            if (slot != null || !MagDevelopmentAccountPolicy.ValidAccount(target))
            {
                var resolved = store == null ? null : MagAdminOperations.Resolve(store, target, slot);
                if (resolved?.Character == null) { error.WriteLine("mag-admin: " + (resolved?.Message ?? $"account {target} has no character")); return 1; }
                account = resolved.Character.Account; slot = resolved.Character.Slot;
            }
            if (!MagDevelopmentAccountPolicy.ValidAccount(account)) { error.WriteLine($"mag-admin: account {account} has no character"); return 1; }
            if (!MagAdminKill.ValidReason(reason))
            { error.WriteLine($"mag-admin: --reason must be printable ASCII of at most {MagAdminKill.MaximumReasonLength} characters"); return 2; }
            string actor = operatorName ?? DefaultOperator();
            if (actor.Length == 0 || actor.Length > MagAdminKill.MaximumOperatorLength || actor.Any(c => c < 0x20 || c > 0x7e))
            { error.WriteLine($"mag-admin: --operator must be 1 to {MagAdminKill.MaximumOperatorLength} printable ASCII characters"); return 2; }
            var body = slot is int named
                ? System.Text.Json.JsonSerializer.SerializeToUtf8Bytes(new { apply, reason, slot = named })
                : System.Text.Json.JsonSerializer.SerializeToUtf8Bytes(new { apply, reason });
            (int status, System.Text.Json.JsonElement? json) reply;
            try { reply = api.Send("POST", $"/admin/v1/characters/{account}/kill", body, actor, now); }
            catch (Exception e) when (e is System.Net.Sockets.SocketException || e is IOException || e is OperationCanceledException || e is InvalidDataException)
            {
                error.WriteLine("mag-admin: the admin API could not be reached: " + e.Message);
                return 1;
            }
            string Field(string name) =>
                reply.json is System.Text.Json.JsonElement root && root.ValueKind == System.Text.Json.JsonValueKind.Object &&
                root.TryGetProperty(name, out var value) && value.ValueKind != System.Text.Json.JsonValueKind.Null ? value.ToString() : null;
            string Where() => $"player {Field("player_id")} in world {Field("world_id")} on worker {Field("worker_id")}";
            switch (reply.status)
            {
                case 200 when Field("applied") == "True":
                    output.WriteLine($"applied: {account} {(Field("result") == "gave_up" ? "gave up its downed life" : "was killed")} ({Where()})");
                    output.WriteLine("  the player respawns with the next wave; the worker log records the admin_kill");
                    return 0;
                case 200:
                    output.WriteLine($"dry run (add --apply to kill): {account} is {Where()}");
                    return 0;
                case 404:
                    error.WriteLine($"mag-admin: refused (not_found): account {account} has no character");
                    return 1;
                default:
                    string code = Field("error") ?? "unknown";
                    string message = Field("message");
                    error.WriteLine($"mag-admin: refused ({code}): {message ?? $"the admin API answered {reply.status}"}");
                    return 1;
            }
        }

        static string Time(DateTimeOffset? at) => at?.UtcDateTime.ToString("yyyy-MM-ddTHH:mm:ssZ") ?? "until lifted";

        private static int VoiceList(LocalCharacterStore store, DateTime now, TextWriter output)
        {
            var active = MagVoiceModeration.Active(store, new DateTimeOffset(DateTime.SpecifyKind(now, DateTimeKind.Utc)));
            output.WriteLine($"{"ACCOUNT",-16} {"KIND",-4} {"ENDS",-20} {"SINCE",-20} {"OPERATOR",-24} REASON");
            foreach (var s in active)
                output.WriteLine($"{s.Account,-16} {VoiceSanction.Name(s.Kind),-4} {Time(s.ExpiresAt),-20} {Time(s.CreatedAt),-20} {s.Operator,-24} {s.Reason}");
            if (active.Count == 0) output.WriteLine("(no active voice mutes or bans)");
            return 0;
        }

        private static int Voice(LocalCharacterStore store, string action, string account, bool apply, string reason, string operatorName,
            string durationText, string backupDirectory, TextWriter output, TextWriter error, DateTime now)
        {
            bool lift = action.StartsWith("un", StringComparison.Ordinal);
            var kind = action.EndsWith("ban", StringComparison.Ordinal) ? VoiceSanctionKind.Ban : VoiceSanctionKind.Mute;
            TimeSpan? duration = null;
            if (durationText != null)
            {
                if (lift) { error.WriteLine($"mag-admin: voice {action} takes no --for"); return 2; }
                if (!MagVoiceModeration.TryParseDuration(durationText, out var parsed))
                {
                    error.WriteLine($"mag-admin: --for must be a whole number of minutes, hours, days or weeks (30m, 12h, 7d, 2w), " +
                        $"at most {LocalCharacterStore.MaximumVoiceSanctionDuration.TotalDays:0} days");
                    return 2;
                }
                duration = parsed;
            }
            var result = MagVoiceModeration.Change(store, account, kind, lift, reason, operatorName ?? DefaultOperator(), duration, apply, backupDirectory);
            if (result.Refusal == MagVoiceRefusal.InvalidRequest) { error.WriteLine("mag-admin: " + result.Message); return 2; }
            if (result.Refusal != MagVoiceRefusal.None) { error.WriteLine($"mag-admin: refused ({result.Code}): {result.Message}"); return 1; }
            var change = result.Change;
            string kindName = VoiceSanction.Name(kind);
            output.WriteLine((change.Applied ? "applied" : "dry run (add --apply to write)") + $": voice {action} {account}");
            output.WriteLine("  before  " + (change.Before == null ? $"no active {kindName}" : Describe(change.Before)));
            output.WriteLine("  after   " + (change.After == null ? $"no {kindName}" : Describe(change.After)));
            if (change.Journal != null) output.WriteLine($"  journal {change.Journal}");
            if (change.Applied && !lift)
                output.WriteLine("  the running server drops this account's voice within 5 s (both copies of every frame)");
            return 0;
        }

        static string Describe(VoiceSanction s) =>
            $"{VoiceSanction.Name(s.Kind)} {(s.ExpiresAt == null ? "until lifted" : "until " + Time(s.ExpiresAt))}, by {s.Operator}" +
            (s.Reason == null ? "" : $": {s.Reason}");

        private static int SetLevel(LocalCharacterStore store, string target, int? slot, byte level, bool apply, string backupDirectory,
            string markerDirectory, DateTime now, TextWriter output, TextWriter error)
        {
            var result = MagAdminOperations.SetLevel(store, target, level, apply, backupDirectory, markerDirectory, now, slot);
            // The CLI keeps reporting an unchanged level as a successful no-op.
            if (result.Refusal != MagAdminRefusal.None && result.Refusal != MagAdminRefusal.SameLevel)
            {
                error.WriteLine($"mag-admin: refused: {result.Message}");
                return 1;
            }
            var change = result.Change;
            if (result.OnlineWarning) output.WriteLine($"warning: {change.Account} is logged in; --apply will be refused until it logs out");
            string verb = change.Applied ? "applied" : result.Refusal == MagAdminRefusal.SameLevel ? "unchanged" : "dry run (add --apply to write)";
            output.WriteLine($"{verb}: {change.Account} ({change.CharacterId} {change.Name}, slot {change.Slot})");
            output.WriteLine($"  level        {change.PreviousLevel} -> {change.Level}");
            output.WriteLine($"  experience   {change.PreviousExperience} -> {change.Experience}");
            output.WriteLine($"  skill points {change.PreviousSkillPoints} -> {change.SkillPoints}");
            if (change.BackupPath != null) output.WriteLine($"  backup       {change.BackupPath}");
            return 0;
        }
    }
}
