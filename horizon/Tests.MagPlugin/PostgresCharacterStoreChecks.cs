using System.Diagnostics;
using System.Text.Json.Nodes;
using Npgsql;
using RT.Models.MAG;
using Server.Medius;

// PostgreSQL character store checks. Every account, character and reward is invented here; each
// check runs in its own throwaway database on the server named by MAG_TEST_POSTGRES (an Npgsql
// connection string with CREATE DATABASE rights). Without it these checks report that they did
// not run, unless MAG_REQUIRE_POSTGRES_CHECKS=1 (CI), where that is a failure.
internal static partial class LeadershipProgressionChecks
{
    private static readonly DateTimeOffset PostgresNow = new DateTimeOffset(2026, 11, 25, 12, 0, 0, TimeSpan.Zero);

    private static void WithPostgres(string check, Action<PostgresCharacterDatabase, string> test)
    {
        string admin = Environment.GetEnvironmentVariable("MAG_TEST_POSTGRES");
        if (string.IsNullOrWhiteSpace(admin))
        {
            if (Environment.GetEnvironmentVariable("MAG_REQUIRE_POSTGRES_CHECKS") == "1")
                throw new Exception("MAG_TEST_POSTGRES is required for the PostgreSQL character store checks");
            Console.WriteLine($"NOT RUN {check}: MAG_TEST_POSTGRES is not set");
            return;
        }
        string name = "mag_check_" + Guid.NewGuid().ToString("N")[..16];
        void Admin(string sql)
        {
            using var connection = new NpgsqlConnection(admin);
            connection.Open();
            using var command = new NpgsqlCommand(sql, connection);
            command.ExecuteNonQuery();
        }
        Admin($"CREATE DATABASE {name}");
        string connectionString = new NpgsqlConnectionStringBuilder(admin) { Database = name, Timeout = 5, CommandTimeout = 30 }.ConnectionString;
        try
        {
            using var database = new PostgresCharacterDatabase(connectionString) { Clock = () => PostgresNow };
            Check(database.Migrate() == database.LatestSchemaVersion, "Initial migrations were not applied");
            test(database, connectionString);
        }
        finally
        {
            NpgsqlConnection.ClearAllPools();
            Admin($"DROP DATABASE IF EXISTS {name} WITH (FORCE)");
        }
    }

    private static void Sql(string connectionString, string sql)
    {
        using var connection = new NpgsqlConnection(connectionString);
        connection.Open();
        using var command = new NpgsqlCommand(sql, connection);
        command.ExecuteNonQuery();
    }

    private static long Scalar(string connectionString, string sql)
    {
        using var connection = new NpgsqlConnection(connectionString);
        connection.Open();
        using var command = new NpgsqlCommand(sql, connection);
        return Convert.ToInt64(command.ExecuteScalar());
    }

    private sealed record ParityCatalogs(MagProgressionCatalog Progression, MagLoadoutCatalog Armory, MagSkillCatalog Skills);

    private static ParityCatalogs ParityCatalogsIn(string directory)
    {
        var (armory, skills) = LoadoutPreferenceStoreChecks.RetailSkillCatalogs(directory);
        return new ParityCatalogs(Load(directory, Catalog()), armory, skills);
    }

    private static LocalCharacterStore Configure(LocalCharacterStore store, ParityCatalogs catalogs)
    {
        store.ProgressionCatalog = catalogs.Progression;
        store.LoadoutCatalog = catalogs.Armory;
        store.SkillCatalog = catalogs.Skills;
        store.Clock = () => PostgresNow;
        return store;
    }

    /// <summary>Runs one fixed sequence of operations and records every outcome.</summary>
    private static List<string> RunParityScript(LocalCharacterStore store, string backupDirectory)
    {
        var log = new List<string>();
        void Created(string account, string name)
        {
            var request = new MagCreateCharacterRequest { AccountName = name, Faction = 0, UniverseId = 26, NpId = new byte[36], HeadId = 4, VoiceId = 2 };
            var outcome = store.CreateForAccount(request, account);
            log.Add($"create {account} {name}: {outcome.Result} {outcome.Reason}");
        }
        void Reward(string account, uint id, string reward, string match, string kind, GameplayLeadershipResult leadership = null)
        {
            var o = store.ApplyRewardForAccount(account, id, reward, match, kind, 1, leadership);
            log.Add($"reward {account} {reward} {kind}: {o.Result} {o.Reason} dup={o.Duplicate} xp={o.Experience} lvl={o.Level} " +
                $"sp={o.SkillPoints} awarded={o.ExperienceAwarded} match={o.MatchExperience} frac={o.MatchFraction} lp={o.Leadership?.LeadershipPoints}");
        }
        void Outcome(string what, CharacterCreationOutcome o) => log.Add($"{what}: {o.Result} {o.Reason}");

        Created("alpha", "Alpha");
        Created("bravo", "Bravo");
        Created("charlie", "ALPHA");          // name taken, case-insensitively
        Created("alpha", "Alpha");            // exact retry
        Created("alpha", "Other");            // a second slot
        Reward("alpha", 1, "r1", "world-1-m1", "enemy_killed");
        Reward("alpha", 1, "r1", "world-1-m1", "enemy_killed");    // replay
        Reward("alpha", 1, "r1", "world-1-m1", "teammate_healed"); // receipt mismatch
        Reward("alpha", 1, "r2", "world-1-m1", "enemy_killed");
        Reward("alpha", 1, "r3", "world-1-m1", "enemy_killed");
        Reward("bravo", 2, "r4", "world-1-m1", "enemy_killed");
        Reward("bravo", 2, "r1", "world-1-m1", "enemy_killed");    // another character's receipt
        Reward("alpha", 1, "r5", "world-1-m1", "match_major_victory", new GameplayLeadershipResult(1, new GameplayLeadershipBaseline(1, 0, 0)));
        Reward("alpha", 1, "r5", "world-1-m1", "match_major_victory", new GameplayLeadershipResult(1, new GameplayLeadershipBaseline(1, 0, 0)));
        Reward("alpha", 1, "r6", "world-1-m1", "enemy_killed");    // after the match result
        Reward("alpha", 1, "r7", "world-2-m2", "enemy_killed");
        Reward("alpha", 9, "r8", "world-2-m2", "enemy_killed");    // not owned
        Outcome("nominations", store.SaveNominationsForAccount("alpha", 1, MagRetailNominationsUpdate.KnownBits));
        Outcome("nominations again", store.SaveNominationsForAccount("alpha", 1, MagRetailNominationsUpdate.KnownBits));
        Outcome("purchase", store.PurchaseSkillForAccount("alpha", 1, 3));
        Outcome("purchase again", store.PurchaseSkillForAccount("alpha", 1, 3));
        var kit = new MagLoadout { UpdateId = 2 };
        kit.Items[0] = 1026; kit.Items[14] = 1028;
        Outcome("save kit", store.SaveLoadoutForAccount("alpha", 1, 1, kit));
        var unowned = new MagLoadout { UpdateId = 2 };
        unowned.Items[0] = 1026; unowned.Items[1] = 1030;
        Outcome("save unowned kit", store.SaveLoadoutForAccount("alpha", 1, 2, unowned));
        Outcome("select kit", store.SaveLastUsedLoadoutForAccount("alpha", 1, 1));
        Outcome("delete kit", store.DeleteLoadoutForAccount("alpha", 1, 0));
        Outcome("stale select", store.SaveLastUsedLoadoutForAccount("alpha", 1, 0, 0));
        var dry = store.RaiseLevelForAdmin("bravo", 3, false, backupDirectory);
        log.Add($"set-level dry: {dry.Reason} {dry.Level} {dry.Experience} {dry.SkillPoints}");
        var applied = store.RaiseLevelForAdmin("bravo", 3, true, backupDirectory);
        log.Add($"set-level: {applied.Reason} {applied.Applied} {applied.Level} {applied.Experience} {applied.SkillPoints}");
        foreach (var account in new[] { "alpha", "bravo", "charlie" })
        {
            // An account holds up to three characters (alpha has two here); log every one, in slot order.
            var rows = store.ReadForAccount(account);
            if (rows.Count == 0) log.Add($"read {account}: none");
            foreach (var row in rows)
            {
                var r = row.ToRecord();
                log.Add($"read {account} slot {row.Slot}: id={r.AccountId} name={row.Name} xp={r.Experience} lvl={r.Level} sp={r.SkillPoints} " +
                    $"lead={r.LeadershipLevel}/{r.LeadershipPoints}/{r.LeadershipNominations} last={r.LastUsedLoadout} " +
                    $"kits={string.Join("|", r.Loadouts.Select(l => l.UpdateId + ":" + string.Join(",", l.Items)))} " +
                    $"items={string.Join(",", r.AvailableItems)} skills={string.Join(",", r.Skills)}");
            }
        }
        log.Add("all: " + string.Join(",", store.ReadAll().Select(c => c.Account + "=" + c.Id)));
        return log;
    }

    public static void PostgresMatchesJsonStore() => WithDirectory(directory => WithPostgres("postgres parity", (database, _) =>
    {
        var catalogs = ParityCatalogsIn(directory);
        var json = Configure(new LocalCharacterStore(Path.Combine(directory, "characters.json")), catalogs);
        var pg = Configure(LocalCharacterStore.ForPostgres(database), catalogs);
        var expected = RunParityScript(json, Path.Combine(directory, "json-backups"));
        var actual = RunParityScript(pg, Path.Combine(directory, "pg-backups"));
        for (int i = 0; i < Math.Max(expected.Count, actual.Count); i++)
            Check(i < expected.Count && i < actual.Count && expected[i] == actual[i],
                $"Backends disagree at step {i}: json '{(i < expected.Count ? expected[i] : "")}' postgres '{(i < actual.Count ? actual[i] : "")}'");
        Check(expected.Any(line => line.Contains("reward-committed dup=False")) && expected.Any(line => line.Contains("dup=True")) &&
            expected.Any(line => line.Contains("reward-receipt-mismatch")) && expected.Any(line => line.Contains("match-reward-finalized")) &&
            expected.Any(line => line.StartsWith("save kit: Success")) && expected.Any(line => line.StartsWith("set-level: applied")),
            "Parity script did not exercise its cases");
        var differences = pg.CompareWith(json);
        Check(differences.Count == 0, "Stores differ after the same operations: " + string.Join("; ", differences));
        Check(pg.Summarize() == json.Summarize(), "Store digests differ");
        Check(pg.BackendName == "postgres" && json.BackendName == "json", "Backend names");
    }));

    public static void PostgresMigrations() => WithPostgres("postgres migrations", (database, connection) =>
    {
        Check(database.Migrate() == 0, "A second migration run applied something");
        Check(database.SchemaVersion() == database.LatestSchemaVersion, "Schema version");
        database.RequireCurrentSchema();
        Sql(connection, "UPDATE mag.schema_migrations SET sha256 = 'altered' WHERE version = 1");
        try { database.RequireCurrentSchema(); throw new Exception("An altered migration was accepted"); }
        catch (InvalidDataException e) { Check(e.Message.Contains("migration 1"), "Altered migration not named: " + e.Message); }
        try { database.Migrate(); throw new Exception("Migrate accepted an altered migration"); }
        catch (InvalidDataException) { }
        Sql(connection, "UPDATE mag.schema_migrations SET sha256 = '" + Sha(database) + "' WHERE version = 1");
        Sql(connection, "INSERT INTO mag.schema_migrations (version, name, sha256) VALUES (99, '0099_future.sql', 'x')");
        try { database.RequireCurrentSchema(); throw new Exception("A newer schema was accepted"); }
        catch (InvalidDataException e) { Check(e.Message.Contains("99"), "Newer schema not named: " + e.Message); }
    });

    private static string Sha(PostgresCharacterDatabase database)
    {
        // Recompute the embedded migration digest the way the runner does.
        var assembly = typeof(PostgresCharacterDatabase).Assembly;
        using var stream = assembly.GetManifestResourceStream("MagCharacterStore.Migrations.0001_initial.sql")!;
        using var memory = new MemoryStream();
        stream.CopyTo(memory);
        return Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(memory.ToArray())).ToLowerInvariant();
    }

    public static void PostgresConcurrentCommits() => WithDirectory(directory => WithPostgres("postgres concurrency", (database, connection) =>
    {
        var catalog = Load(directory, Catalog());
        LocalCharacterStore Store()
        {
            var store = LocalCharacterStore.ForPostgres(database);
            store.ProgressionCatalog = catalog;
            return store;
        }
        Create(Store(), "alpha", "Alpha");
        Create(Store(), "bravo", "Bravo");

        // One reward delivered eight times at once is credited once; the rest replay it.
        var same = new ProgressionRewardOutcome[8];
        Parallel.For(0, same.Length, i => same[i] = Store().ApplyRewardForAccount("alpha", 1, "same", "world-1-m", "enemy_killed", 1));
        Check(same.All(o => o.Result == MagNetResult.Success) && same.Count(o => !o.Duplicate) == 1 && same.All(o => o.Experience == 7),
            "A concurrently redelivered reward was not credited exactly once");

        // Distinct rewards for one character serialize on its row and all land.
        var distinct = new ProgressionRewardOutcome[8];
        Parallel.For(0, distinct.Length, i => distinct[i] = Store().ApplyRewardForAccount("alpha", 1, "kill-" + i, "world-1-m", "enemy_killed", 1));
        Check(distinct.All(o => o.Result == MagNetResult.Success && !o.Duplicate), "Concurrent distinct rewards were refused");
        var alpha = Store().ReadForAccount("alpha").Single();
        Check(alpha.Experience == 63 && Scalar(connection, "SELECT count(*) FROM mag.reward_receipts") == 9,
            "Concurrent distinct rewards lost or doubled XP");

        // The same reward id for two characters: one commits, the other is a receipt mismatch.
        var shared = new ProgressionRewardOutcome[2];
        Parallel.For(0, 2, i => shared[i] = Store().ApplyRewardForAccount(i == 0 ? "alpha" : "bravo", (uint)(i + 1), "shared", "world-1-m", "enemy_killed", 1));
        Check(shared.Count(o => o.Result == MagNetResult.Success && !o.Duplicate) == 1 &&
            shared.Count(o => o.Reason == "reward-receipt-mismatch") == 1, "A reward id shared by two characters was not refused once");

        // Concurrent creations with one name: exactly one character gets it.
        var creates = new CharacterCreationOutcome[6];
        Parallel.For(0, creates.Length, i => creates[i] = Store().CreateForAccount(
            new MagCreateCharacterRequest { AccountName = "Same", Faction = 0, UniverseId = 26, NpId = new byte[36] }, "acct" + i));
        Check(creates.Count(o => o.Result == MagNetResult.Success) == 1 && creates.Count(o => o.Reason == "character-name-taken") == 5,
            "Concurrent creation with one name: " + string.Join(",", creates.Select(o => o.Reason)));
    }));

    public static void PostgresOutageDefers() => WithDirectory(directory =>
    {
        // A closed port on the loopback interface stands in for a database that is down.
        using var database = new PostgresCharacterDatabase("Host=127.0.0.1;Port=1;Username=nobody;Database=none;Timeout=2");
        var store = LocalCharacterStore.ForPostgres(database);
        store.ProgressionCatalog = Load(directory, Catalog());
        try { store.ApplyRewardForAccount("alpha", 1, "r", "m", "enemy_killed", 1); throw new Exception("An unreachable database committed"); }
        catch (CharacterStoreUnavailableException e) { Check(e is IOException, "Unavailable is not an IOException, so rewards would not defer"); }
        var timer = Stopwatch.StartNew();
        try { store.ReadForAccount("alpha"); throw new Exception("An unreachable database answered"); }
        catch (CharacterStoreUnavailableException e)
        { Check(timer.ElapsedMilliseconds < 500 && e.Message.Contains("recent failure"), "The breaker did not fail fast"); }
    });

    public static void PostgresLedgerRetention() => WithDirectory(directory => WithPostgres("postgres retention", (database, connection) =>
    {
        var store = LocalCharacterStore.ForPostgres(database);
        store.ProgressionCatalog = Load(directory, Catalog());
        Create(store, "alpha", "Alpha");
        DateTimeOffset now = PostgresNow;
        database.Clock = () => now;
        store.Clock = () => now - TimeSpan.FromDays(10);
        Check(store.ApplyRewardForAccount("alpha", 1, "old-1", "world-1-old", "enemy_killed", 1).Result == MagNetResult.Success, "Old reward");
        Check(store.ApplyRewardForAccount("alpha", 1, "old-2", "world-1-old", "enemy_killed", 1).Result == MagNetResult.Success, "Old reward");
        store.Clock = () => now - TimeSpan.FromDays(6);
        Check(store.ApplyRewardForAccount("alpha", 1, "recent", "world-1-recent", "enemy_killed", 1).Result == MagNetResult.Success, "Recent reward");
        var pruned = database.PruneLedger(TimeSpan.FromDays(7));
        Check(!pruned.Skipped && pruned.Rows == 1 && pruned.Receipts == 2 && pruned.HistoryRows == 1 && pruned.FinalizedRows == 0,
            "Retention did not prune exactly the old row with its receipts: " + pruned.Describe());
        Check(Scalar(connection, "SELECT count(*) FROM mag.reward_receipts WHERE match_key = 'world-1-old'") == 0 &&
            Scalar(connection, "SELECT count(*) FROM mag.reward_receipts WHERE match_key = 'world-1-recent'") == 1 &&
            Scalar(connection, "SELECT rewards FROM mag.match_history WHERE match_key = 'world-1-old'") == 2 &&
            Scalar(connection, "SELECT count(*) FROM mag.character_matches") == 2, "Retention history");
        Check(Scalar(connection, "SELECT count(*) FROM mag.match_progress p LEFT JOIN mag.reward_receipts r " +
            "ON r.character_id = p.character_id AND r.match_key = p.match_key WHERE r.reward_id IS NULL") == 0,
            "A match row survived without its receipts");
        // A late reward for the pruned match is credited once, then replays.
        store.Clock = () => now;
        int before = store.ReadForAccount("alpha").Single().Experience ?? 0;
        var late = store.ApplyRewardForAccount("alpha", 1, "late", "world-1-old", "enemy_killed", 1);
        var replay = store.ApplyRewardForAccount("alpha", 1, "late", "world-1-old", "enemy_killed", 1);
        Check(late.Result == MagNetResult.Success && !late.Duplicate && late.Experience == before + 7 && late.MatchExperience == 7 &&
            replay.Duplicate && replay.Experience == late.Experience, "A late reward after pruning was not credited exactly once");
        // A second pass merges into the existing history row.
        now += TimeSpan.FromDays(8);
        var second = database.PruneLedger(TimeSpan.FromDays(7));
        Check(second.Rows == 2 && Scalar(connection, "SELECT count(*) FROM mag.match_progress") == 0, "Second retention pass: " + second.Describe());
        Check(Scalar(connection, "SELECT rewards FROM mag.match_history WHERE match_key = 'world-1-old'") == 3 &&
            Scalar(connection, "SELECT experience FROM mag.match_history WHERE match_key = 'world-1-old'") == 21, "History merge");
    }));

    public static void PostgresImportVerifyExport() => WithDirectory(directory => WithPostgres("postgres import and export", (database, connection) =>
    {
        var catalogs = ParityCatalogsIn(directory);
        string jsonPath = Path.Combine(directory, "characters.json");
        var json = Configure(new LocalCharacterStore(jsonPath), catalogs);
        RunParityScript(json, Path.Combine(directory, "backups"));   // three characters: alpha (slots 0 and 1) and bravo
        var pg = Configure(LocalCharacterStore.ForPostgres(database), catalogs);

        // Unknown fields are refused, and nothing is written.
        var extended = JsonNode.Parse(File.ReadAllText(jsonPath))!.AsObject();
        extended["characters"]![0]!["futureField"] = "kept by older builds";
        string extendedPath = Path.Combine(directory, "extended.json");
        File.WriteAllText(extendedPath, extended.ToJsonString());
        try { pg.ImportFrom(Configure(new LocalCharacterStore(extendedPath), catalogs)); throw new Exception("Unknown fields were dropped silently"); }
        catch (InvalidDataException) { }
        Check(Scalar(connection, "SELECT count(*) FROM mag.characters") == 0, "A refused import wrote rows");

        // A dry run reports what an import would do and writes nothing.
        var planned = pg.PlanImport(json);
        Check(planned.Action == CharacterImportAction.Import && planned.Target == null && planned.Source == json.Summarize() &&
            planned.LastImportAt == null, "Dry run of the first import: " + planned);
        Check(Scalar(connection, "SELECT count(*) FROM mag.characters") == 0 && Scalar(connection, "SELECT count(*) FROM mag.admin_changes") == 0,
            "A dry run wrote rows");

        var first = pg.ImportFrom(json);
        var imported = first.After;
        Check(first.Plan.Action == CharacterImportAction.Import && imported == json.Summarize() && imported.Characters == 3 &&
            imported.Receipts == 6 && imported.MatchRows == 3, "Import summary differs from the JSON store: " + imported.Describe());
        Check(pg.CompareWith(json).Count == 0, "Import does not verify");
        const string imports = "SELECT count(*) FROM mag.admin_changes WHERE operation = 'character-store-import'";
        Check(Scalar(connection, imports) == 1, "The import was not recorded");

        // Idempotent: importing the same store again writes nothing.
        var again = pg.ImportFrom(json);
        Check(again.Plan.Action == CharacterImportAction.Unchanged && again.After == imported && Scalar(connection, imports) == 1 &&
            Scalar(connection, "SELECT count(*) FROM mag.characters WHERE revision > 1") == 0, "A repeated import was not a no-op: " + again.Plan);

        // The JSON store moves on (a reward committed after the import, before the switch). Without --replace the
        // import refuses to merge; with it, the rows are replaced, since the server has not used the database.
        Check(json.ApplyRewardForAccount("alpha", 1, "late-1", "world-3-m3", "enemy_killed", 1).Result == MagNetResult.Success, "Late JSON reward");
        var refused = pg.PlanImport(json);
        Check(refused.Action == CharacterImportAction.Refused && !refused.ChangedSinceImport && refused.Differences > 0 &&
            refused.Reason.Contains("--replace"), "A differing database was not refused: " + refused);
        try { pg.ImportFrom(json); throw new Exception("An import merged into a differing store"); }
        catch (InvalidDataException) { }
        var replaced = pg.ImportFrom(json, new CharacterImportOptions(Replace: true));
        Check(replaced.Plan.Action == CharacterImportAction.Replace && replaced.After == json.Summarize() && pg.CompareWith(json).Count == 0 &&
            Scalar(connection, imports) == 2, "Replacing an unused import failed: " + replaced.Plan);

        // The imported store keeps working: replays, new rewards and new characters (IDs continue).
        var replay = pg.ApplyRewardForAccount("alpha", 1, "r2", "world-1-m1", "enemy_killed", 1);
        Check(replay.Result == MagNetResult.Success && replay.Duplicate, "An imported receipt does not replay");
        Check(pg.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Delta", Faction = 1, UniverseId = 26, NpId = new byte[36] },
            "delta").Result == MagNetResult.Success && pg.ReadForAccount("delta").Single().Id == 4, "Character IDs do not continue after import");
        Check(Scalar(connection, "SELECT count(*) FROM mag.match_progress WHERE result_kind = 'match_major_victory' AND finalized") == 1,
            "The match result kind was not recorded");

        // Export (the rollback path) writes a JSON store that verifies against PostgreSQL. Everything fits the
        // JSON bound, so nothing is trimmed and the copy is exact.
        string exportPath = Path.Combine(directory, "exported.json");
        var exported = Configure(new LocalCharacterStore(exportPath), catalogs);
        var export = pg.ExportTo(exported, false);
        Check(export.Source == pg.Summarize() && export.Exported == export.Source && export.Comparison.Differences.Count == 0 &&
            export.Comparison.TrimmedMatchRows == 0 && export.Comparison.TrimmedReceipts == 0 && exported.CompareWith(pg).Count == 0 &&
            pg.CompareExport(exported).Differences.Count == 0, "Export does not verify");
        try { pg.ExportTo(exported, false); throw new Exception("Export replaced a store without --replace"); }
        catch (IOException) { }
        pg.ExportTo(exported, true);
        Check(Directory.GetFiles(directory, "exported.json.before-export-*").Length == 1, "Export did not keep the replaced store");
        Check(json.CompareWith(pg).Any(d => d.StartsWith("character delta", StringComparison.Ordinal)), "Verify missed a difference");

        // Once the server has written to the database, --replace refuses to discard that, unless told to.
        var used = pg.PlanImport(json, new CharacterImportOptions(Replace: true));
        Check(used.Action == CharacterImportAction.Refused && used.ChangedSinceImport && used.Reason.Contains("--discard-postgres-changes"),
            "Replace over server writes was not refused: " + used);
        try { pg.ImportFrom(json, new CharacterImportOptions(Replace: true)); throw new Exception("Replace discarded server writes"); }
        catch (InvalidDataException) { }
        Check(Scalar(connection, "SELECT count(*) FROM mag.characters") == 4, "A refused replace changed rows");
        var discarded = pg.ImportFrom(json, new CharacterImportOptions(Replace: true, DiscardPostgresChanges: true));
        Check(discarded.Plan.Action == CharacterImportAction.Replace && pg.CompareWith(json).Count == 0 &&
            Scalar(connection, "SELECT count(*) FROM mag.characters") == 3 && Scalar(connection, imports) == 3,
            "Replace with --discard-postgres-changes failed: " + discarded.Plan);
        Check(pg.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Echo", Faction = 1, UniverseId = 26, NpId = new byte[36] },
            "echo").Result == MagNetResult.Success && pg.ReadForAccount("echo").Single().Id == 4, "Character IDs do not continue after a replace");
    }));

    public static void PostgresSecretDirectoryConnection() => WithDirectory(directory =>
    {
        foreach (var (name, value) in new[] { ("host", "db-rw"), ("port", "5432\n"), ("dbname", "mag"), ("user", "mag"), ("password", "invented") })
            File.WriteAllText(Path.Combine(directory, name), value);
        var builder = new NpgsqlConnectionStringBuilder(PostgresCharacterDatabase.ConnectionStringFromSecretDirectory(directory, null));
        Check(builder.Host == "db-rw" && builder.Port == 5432 && builder.Database == "mag" && builder.SslMode == SslMode.Require,
            "Secret directory connection settings");
        Check(builder.GssEncryptionMode == GssEncryptionMode.Disable, "GSSAPI encryption is attempted (libgssapi_krb5 load errors)");
    });

    public static void PostgresImportCommandLine() => WithDirectory(directory => WithPostgres("postgres import command line", (database, connection) =>
    {
        var catalogs = ParityCatalogsIn(directory);
        string jsonPath = Path.Combine(directory, "characters.json");
        RunParityScript(Configure(new LocalCharacterStore(jsonPath), catalogs), Path.Combine(directory, "backups"));
        var pg = Configure(LocalCharacterStore.ForPostgres(database), catalogs);
        (int Code, string Output, string Error) Run(params string[] args)
        {
            var output = new StringWriter(); var error = new StringWriter();
            int code = MagAdminCli.RunCharacterStore(args, database, () => pg, path => Configure(new LocalCharacterStore(path), catalogs), output, error);
            string all = output.ToString() + error.ToString();
            // Differences name their records (docs/admin-cli.md); everything else is counts and digests only.
            if (!all.Contains("difference: ", StringComparison.Ordinal))
                foreach (var secret in new[] { "alpha", "Alpha", "bravo", "Bravo", "world-1-m1" })
                    Check(!all.Contains(secret, StringComparison.Ordinal), $"character-store {args[0]} printed player data ({secret})");
            return (code, output.ToString(), error.ToString());
        }
        string digest = Configure(new LocalCharacterStore(jsonPath), catalogs).Summarize().Digest;

        var dry = Run("import", "--from", jsonPath, "--dry-run");
        Check(dry.Code == 0 && dry.Output.Contains("database: empty") && dry.Output.Contains("dry run: nothing written") &&
            dry.Output.Contains(digest), "import --dry-run: " + dry);
        Check(Scalar(connection, "SELECT count(*) FROM mag.characters") == 0, "import --dry-run wrote rows");
        var import = Run("import", "--from", jsonPath);
        Check(import.Code == 0 && import.Output.Contains("imported: 3 characters, 6 reward receipts, 3 match rows, digest " + digest) &&
            import.Output.Contains("verified"), "import: " + import);
        var repeat = Run("import", "--from", jsonPath);
        Check(repeat.Code == 0 && repeat.Output.Contains("unchanged: "), "repeated import: " + repeat);
        var verify = Run("verify", "--json", jsonPath);
        Check(verify.Code == 0 && verify.Output.Contains("json:     3 characters") && verify.Output.Contains("postgres: 3 characters") &&
            verify.Output.Contains(digest), "verify: " + verify);
        var status = Run("status");
        Check(status.Code == 0 && status.Output.Contains("characters 3, character_loadouts 5") && status.Output.Contains("reward_receipts 6") &&
            status.Output.Contains("digest " + digest) && !status.Output.Contains("last import: never"), "status: " + status);
        Check(Run("import", "--from", jsonPath, "--discard-postgres-changes").Code == 2, "--discard-postgres-changes without --replace was accepted");

        // A JSON change after the import: the dry run predicts the refusal, verify names the record, and the exit codes say so.
        Check(Configure(new LocalCharacterStore(jsonPath), catalogs).ApplyRewardForAccount("bravo", 2, "late-2", "world-3-m3", "enemy_killed", 1)
            .Result == MagNetResult.Success, "Late JSON reward");
        var differs = Run("verify", "--json", jsonPath);
        Check(differs.Code == 1 && differs.Error.Contains("difference(s)") && differs.Error.Contains("receipt late-2: only in the json store"),
            "verify after a JSON change: " + differs);
        var predicted = Run("import", "--from", jsonPath, "--dry-run");
        Check(predicted.Code == 1 && predicted.Error.Contains("would be refused"), "import --dry-run of a differing store: " + predicted);
        var replace = Run("import", "--from", jsonPath, "--replace", "--dry-run");
        Check(replace.Code == 0 && replace.Output.Contains("action:   replace"), "import --replace --dry-run: " + replace);
        Check(Run("import", "--from", jsonPath, "--replace").Code == 0 && Run("verify", "--json", jsonPath).Code == 0, "import --replace");
    }));
    /// <summary>
    /// PostgreSQL keeps more settled history than the JSON store's bound (a week of match rows against
    /// 1 MiB and 4096 receipts or rows). The export keeps all it can, leaves out only the least recently
    /// rewarded settled rows, fails rather than leave out a row the JSON store still needs, and verifies
    /// what it wrote; verify --as-export excuses exactly that trimmed history and nothing else.
    /// </summary>
    public static void PostgresExportBeyondJsonBound() => WithDirectory(directory => WithPostgres("postgres export beyond the JSON bound", (database, connection) =>
    {
        var catalog = Load(directory, Catalog());
        DateTimeOffset now = PostgresNow;
        var pg = LocalCharacterStore.ForPostgres(database);
        pg.ProgressionCatalog = catalog;
        LocalCharacterStore Json(string path) => new LocalCharacterStore(path) { ProgressionCatalog = catalog, Clock = () => now };
        Create(pg, "alpha", "Alpha");
        Create(pg, "bravo", "Bravo");
        void Reward(string account, uint id, string reward, string match, string kind, TimeSpan ago)
        {
            pg.Clock = () => now - ago;
            Check(pg.ApplyRewardForAccount(account, id, reward, match, kind, 1).Result == MagNetResult.Success, "Synthetic reward " + reward);
        }
        // Abandoned (open, no reward for longer than the 24 h TTL), settled, and an open match still running.
        Reward("bravo", 2, "abandoned-1", "abandoned-open", "enemy_killed", TimeSpan.FromDays(3));
        Reward("bravo", 2, "abandoned-2", "abandoned-open", "enemy_killed", TimeSpan.FromDays(3));
        Reward("alpha", 1, "template-1", "template-final", "enemy_killed", TimeSpan.FromHours(1));
        Reward("alpha", 1, "template-2", "template-final", "enemy_killed", TimeSpan.FromHours(1));
        Reward("alpha", 1, "template-3", "template-final", "match_minor_defeat", TimeSpan.FromHours(1));
        Reward("alpha", 1, "active-1", "active-open", "enemy_killed", TimeSpan.FromMinutes(10));
        Reward("bravo", 2, "active-2", "active-open", "enemy_killed", TimeSpan.FromMinutes(10));
        pg.Clock = () => now;
        // 1200 more settled rows like template-final, each older than it (settled-0001 the oldest), with its
        // three receipts: about three times what the JSON store holds.
        const int clones = 1200;
        Sql(connection, $@"
INSERT INTO mag.match_progress (character_id, match_key, experience, fraction, frago_points, finalized, result_kind, first_reward_at, last_reward_at)
SELECT p.character_id, 'settled-' || lpad(g::text, 4, '0'), p.experience, p.fraction, p.frago_points, true, p.result_kind,
       p.first_reward_at - make_interval(mins => {clones + 1} - g), p.last_reward_at - make_interval(mins => {clones + 1} - g)
  FROM mag.match_progress p, generate_series(1, {clones}) g WHERE p.match_key = 'template-final';
INSERT INTO mag.reward_receipts (reward_id, character_id, match_key, kind, units, leadership, frago, outcome, committed_at)
SELECT 'settled-' || lpad(g::text, 4, '0') || '-' || r.reward_id, r.character_id, 'settled-' || lpad(g::text, 4, '0'), r.kind, r.units,
       r.leadership, r.frago, r.outcome, r.committed_at
  FROM mag.reward_receipts r, generate_series(1, {clones}) g WHERE r.match_key = 'template-final';");
        var full = pg.Summarize();
        Check(full.MatchRows == clones + 4 && full.Receipts == 3 * clones + 7, "Synthetic history: " + full.Describe());

        (int Code, string Output, string Error) Run(params string[] args)
        {
            var output = new StringWriter(); var error = new StringWriter();
            int code = MagAdminCli.RunCharacterStore(args, database, () => pg, Json, output, error);
            return (code, output.ToString(), error.ToString());
        }

        // The export keeps history up to the hard bound, not the live store's compaction target.
        string exportPath = Path.Combine(directory, "exported.json");
        var exported = Json(exportPath);
        var export = pg.ExportTo(exported, false);
        long size = new FileInfo(exportPath).Length;
        Check(export.Comparison.Differences.Count == 0, "A trimmed export does not verify: " + string.Join("; ", export.Comparison.Differences));
        Check(size <= LocalCharacterStore.MaximumStoreBytes && size > LocalCharacterStore.LedgerCompactionTriggerBytes,
            $"The export is {size} bytes: compacted to the live target, or over the hard bound");
        var copy = Json(exportPath);
        var kept = copy.Summarize();
        Check(export.Source == full && export.Exported == kept && kept.Characters == 2 &&
            export.Comparison.TrimmedMatchRows == full.MatchRows - kept.MatchRows && export.Comparison.TrimmedReceipts == full.Receipts - kept.Receipts &&
            export.Comparison.TrimmedMatchRows > 1, "Trimmed counts: " + export.Comparison.DescribeTrimmed() + "; kept " + kept.Describe());
        var rows = JsonNode.Parse(File.ReadAllText(exportPath))!["matchRewardProgress"]!.AsObject()
            .Select(e => e.Value!["matchKey"]!.GetValue<string>()).ToHashSet();
        int oldestKept = Enumerable.Range(1, clones).First(g => rows.Contains($"settled-{g:D4}"));
        Check(rows.Contains("active-open") && rows.Contains("template-final") && !rows.Contains("abandoned-open") &&
            Enumerable.Range(oldestKept, clones - oldestKept + 1).All(g => rows.Contains($"settled-{g:D4}")) && oldestKept > 1,
            "The export did not leave out only the least recently rewarded settled rows");
        // Exactly the trimmed rows differ; every character, kit, open row and kept receipt matches.
        var exact = copy.CompareWith(pg);
        Check(exact.Count == export.Comparison.TrimmedMatchRows + export.Comparison.TrimmedReceipts &&
            exact.All(d => d.EndsWith(": only in the postgres store", StringComparison.Ordinal) &&
                (d.StartsWith("match ", StringComparison.Ordinal) || d.StartsWith("receipt ", StringComparison.Ordinal))),
            "The exact comparison found more than trimmed history");
        Check(Json(exportPath).ReadForAccount("alpha").Single().Experience == pg.ReadForAccount("alpha").Single().Experience,
            "The rollback copy lost progression");

        // From the command line: export verifies, plain verify is exact and fails, verify --as-export passes.
        string cliPath = Path.Combine(directory, "cli.json");
        var cli = Run("export", "--to", cliPath);
        Check(cli.Code == 0 && cli.Output.Contains("exported: 2 characters") && cli.Output.Contains("trimmed:  " + export.Comparison.DescribeTrimmed()) &&
            cli.Output.Contains("verified: read back"), "export: " + cli);
        var strict = Run("verify", "--json", cliPath);
        Check(strict.Code == 1 && strict.Error.Contains("difference(s) between the stores"), "Plain verify excused trimmed history: " + strict.Code);
        var asExport = Run("verify", "--json", cliPath, "--as-export");
        Check(asExport.Code == 0 && asExport.Output.Contains("trimmed:  " + export.Comparison.DescribeTrimmed()) &&
            asExport.Output.Contains("verified: the JSON store is a complete export"), "verify --as-export: " + asExport);

        // An open row missing from the JSON store is never excused.
        var edited = JsonNode.Parse(File.ReadAllText(cliPath))!.AsObject();
        edited["matchRewardProgress"]!.AsObject().Remove("bravo:2:active-open");
        edited["rewardReceipts"]!.AsObject().Remove("active-2");
        string editedPath = Path.Combine(directory, "edited.json");
        File.WriteAllText(editedPath, edited.ToJsonString());
        var missing = Run("verify", "--json", editedPath, "--as-export");
        Check(missing.Code == 1 && missing.Error.Contains("match bravo:2:active-open: only in the postgres store") &&
            missing.Error.Contains("receipt active-2: only in the postgres store") && missing.Error.Contains("not a complete export"),
            "verify --as-export excused a missing open row: " + missing);
        // Nor is a write to PostgreSQL after the export.
        Reward("alpha", 1, "active-3", "active-open", "enemy_killed", TimeSpan.Zero);
        var later = Run("verify", "--json", cliPath, "--as-export");
        Check(later.Code == 1 && later.Error.Contains("character alpha: ") && later.Error.Contains("receipt active-3: only in the postgres store"),
            "verify --as-export missed a later write: " + later);

        // Open rows that do not fit fail the export, and no file is written.
        Sql(connection, "UPDATE mag.match_progress SET finalized = false, last_reward_at = '2026-11-25T11:59:00Z' WHERE match_key LIKE 'settled-%'");
        string openPath = Path.Combine(directory, "open.json");
        var open = Run("export", "--to", openPath);
        Check(open.Code == 1 && open.Error.Contains("exceeds local bound") && !File.Exists(openPath), "An export left out open rows: " + open);

        // The receipt and row counts bound the export too: 4096 rows without receipts fit well under 1 MiB.
        Sql(connection, $@"
DELETE FROM mag.match_progress WHERE match_key LIKE 'settled-%';
INSERT INTO mag.match_progress (character_id, match_key, experience, fraction, frago_points, finalized, result_kind, first_reward_at, last_reward_at)
SELECT p.character_id, 'n' || lpad(g::text, 4, '0'), p.experience, p.fraction, p.frago_points, true, p.result_kind,
       p.first_reward_at - make_interval(mins => 5000 - g), p.last_reward_at - make_interval(mins => 5000 - g)
  FROM mag.match_progress p, generate_series(1, 4200) g WHERE p.match_key = 'template-final';");
        string countedPath = Path.Combine(directory, "counted.json");
        var counted = pg.ExportTo(Json(countedPath), false);
        var countedKept = Json(countedPath).Summarize();
        Check(counted.Comparison.Differences.Count == 0 && countedKept.MatchRows == LocalCharacterStore.MaximumRewardReceipts &&
            counted.Comparison.TrimmedMatchRows == counted.Source.MatchRows - LocalCharacterStore.MaximumRewardReceipts &&
            new FileInfo(countedPath).Length < LocalCharacterStore.MaximumStoreBytes, "The row count bound: " + countedKept.Describe());
    }));
}
