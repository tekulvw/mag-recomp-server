using Npgsql;
using RT.Models.MAG;
using Server.Medius;

// Optional schema-contract test uses the real checked-out portal migrations in a fresh database.
// It never reads a deployed portal or its accounts. CI can require it explicitly.
internal static class PortalAccountPostgresChecks
{
    static void Check(bool value, string reason) { if (!value) throw new Exception(reason); }
    public static void Run() => RunAsync().GetAwaiter().GetResult();
    static async Task RunAsync()
    {
        string admin = Environment.GetEnvironmentVariable("MAG_TEST_POSTGRES");
        string migrations = Environment.GetEnvironmentVariable("MAG_TEST_PORTAL_MIGRATIONS");
        if (string.IsNullOrWhiteSpace(admin) || string.IsNullOrWhiteSpace(migrations))
        {
            if (Environment.GetEnvironmentVariable("MAG_REQUIRE_PORTAL_ACCOUNT_CHECKS") == "1")
                throw new Exception("Portal owner schema checks require MAG_TEST_POSTGRES and MAG_TEST_PORTAL_MIGRATIONS.");
            Console.WriteLine("NOT RUN portal owner schema contract: explicit synthetic database and portal migrations required");
            return;
        }
        string database = "mag_portal_owner_" + Guid.NewGuid().ToString("N")[..12];
        async Task Sql(string connectionString, string sql)
        {
            await using var connection = new NpgsqlConnection(connectionString);
            await connection.OpenAsync();
            using var command = new NpgsqlCommand(sql, connection);
            await command.ExecuteNonQueryAsync();
        }
        await Sql(admin, $"CREATE DATABASE {database}");
        string connectionString = new NpgsqlConnectionStringBuilder(admin) { Database = database }.ConnectionString;
        string file = Path.Combine(Path.GetTempPath(), "mag-owner-db-" + Guid.NewGuid().ToString("N"));
        File.WriteAllText(file, connectionString);
        try
        {
            foreach (string migration in Directory.GetFiles(migrations, "*.sql").OrderBy(p => p, StringComparer.Ordinal))
                if (Path.GetFileName(migration).CompareTo("0009") < 0) await Sql(connectionString, File.ReadAllText(migration));
            await Sql(connectionString, "INSERT INTO accounts(provider,subject,player_name) VALUES ('synthetic','one','OldPublic'); INSERT INTO accounts(provider,subject,player_name,kind) VALUES ('service','two','ServicePublic','service'); INSERT INTO accounts(provider,subject) VALUES ('synthetic','unnamed');");
            await using var cache = MagPortalAccountCache.FromConnectionFile(file);
            try { await cache.RefreshAsync(); throw new Exception("Pre-migration owner schema accepted"); }
            catch (InvalidOperationException error) { Check(error.InnerException == null && !error.ToString().Contains(connectionString), "Startup failure leaked connection details"); }
            await Sql(connectionString, File.ReadAllText(Path.Combine(migrations, "0009_player_names.sql")));
            await cache.RefreshAsync();
            Check(cache.TryResolve(1, out var owner) && owner == "oldpublic", "Actual migration did not backfill stable lowercase owner");
            Check(cache.TryResolve(2, out owner) && owner == "servicepublic" && !cache.TryResolve(3, out _), "Service/unnamed contract");
            await Sql(connectionString, "UPDATE accounts SET player_name='NewPublic' WHERE id=1");
            await cache.RefreshAsync();
            Check(cache.TryResolve(1, out owner) && owner == "oldpublic", "Actual rename moved owner");
            try { await Sql(connectionString, "UPDATE accounts SET game_account_name='stolen' WHERE id=1"); throw new Exception("Owner mutation accepted"); }
            catch (PostgresException error) when (error.SqlState == "23514") { }
            await Sql(connectionString, "UPDATE accounts SET player_name='Claimed' WHERE id=3; UPDATE accounts SET banned_at=now() WHERE id=1;");
            await cache.RefreshAsync();
            Check(!cache.TryResolve(1, out _) && cache.TryResolve(3, out owner) && owner == "claimed", "Ban refresh / first claim mapping");
            await Sql(connectionString, "DELETE FROM accounts WHERE id=2");
            await cache.RefreshAsync();
            Check(!cache.TryResolve(2, out _), "Deletion remained in snapshot");
            using var characters = new PostgresCharacterDatabase(connectionString);
            characters.Migrate();
            var store = LocalCharacterStore.ForPostgres(characters);
            var np = new byte[36]; System.Text.Encoding.ASCII.GetBytes("NewPublic").CopyTo(np, 0);
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "NewPublic!0", NpId = np, Faction = 1, HeadId = 2, VoiceId = 3, UniverseId = 26 }, "oldpublic", "NewPublic").Result == MagNetResult.Success,
                "PostgreSQL creation rejected public NP identity");
            Check(StoredCharacter.DisplayNameFor("", store.ReadForAccount("oldpublic").Single().NpId) == "oldpublic" && store.ReadForAccount("newpublic").Count == 0, "PostgreSQL owner/NP persistence mismatch");
            await Sql(connectionString, "ALTER TABLE accounts RENAME COLUMN game_account_name TO unavailable_owner");
            try { await cache.RefreshAsync(); throw new Exception("Missing immutable owner column accepted"); } catch (InvalidOperationException) { }
            Check(!cache.TryResolve(3, out _), "Schema outage retained admission");
        }
        finally
        {
            File.Delete(file);
            NpgsqlConnection.ClearAllPools();
            await Sql(admin, $"DROP DATABASE {database} WITH (FORCE)");
        }
    }
}
