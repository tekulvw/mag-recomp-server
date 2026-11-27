using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using Npgsql;

namespace Server.Medius
{
    /// <summary>Outcome of one ledger retention pass.</summary>
    public sealed record CharacterLedgerPruneResult(bool Skipped, int Rows, int FinalizedRows, int Receipts, int HistoryRows)
    {
        public string Describe() => Skipped
            ? "Character store ledger prune skipped: another process holds the prune lock."
            : $"Character store ledger pruned: {Rows} match rows ({FinalizedRows} finalized, {Rows - FinalizedRows} open; " +
              $"{Receipts} receipts), {HistoryRows} history rows written";
    }

    /// <summary>
    /// The PostgreSQL database behind <see cref="LocalCharacterStore.ForPostgres"/>: one connection pool,
    /// the schema migrations, ledger retention and the outage policy. Errors that mean the database
    /// cannot be used surface as <see cref="CharacterStoreUnavailableException"/> (an IOException), so
    /// callers defer exactly as for a failed file commit; after one, calls fail fast for a second.
    /// </summary>
    public sealed class PostgresCharacterDatabase : IDisposable
    {
        public static readonly TimeSpan DefaultLedgerRetention = TimeSpan.FromDays(7);
        public static readonly TimeSpan MinimumLedgerRetention = TimeSpan.FromDays(1);
        public static readonly TimeSpan MaximumLedgerRetention = TimeSpan.FromDays(90);
        /// <summary>Fail-fast period after the database was found unusable: one gameplay resend period.</summary>
        public static readonly TimeSpan UnavailableBackoff = TimeSpan.FromSeconds(1);
        private const long MigrationLockKey = 0x4d41_4743_5354_0001; // "MAGCST" + 1
        private const long PruneLockKey = 0x4d41_4743_5354_0002;
        private const string MigrationResourcePrefix = "MagCharacterStore.Migrations.";
        internal const int MaintenanceCommandTimeoutSeconds = 120;

        private readonly NpgsqlDataSource _source;
        private long _unavailableUntilTicks;
        public bool ReadOnly { get; }
        /// <summary>Host, port and database name; never credentials.</summary>
        public string Target { get; }
        public Func<DateTimeOffset> Clock { get; set; } = () => DateTimeOffset.UtcNow;

        public PostgresCharacterDatabase(string connectionString, bool readOnly = false)
        {
            var builder = new NpgsqlConnectionStringBuilder(connectionString);
            if (readOnly)
                builder.Options = string.IsNullOrWhiteSpace(builder.Options) ? "-c default_transaction_read_only=on"
                    : builder.Options + " -c default_transaction_read_only=on";
            ReadOnly = readOnly;
            Target = $"{builder.Host}:{builder.Port}/{builder.Database}";
            _source = NpgsqlDataSource.Create(builder.ConnectionString);
        }

        /// <summary>
        /// Builds a connection from a mounted CloudNativePG application secret (files host, port, dbname,
        /// user, password). With a CA file the server certificate is verified; otherwise TLS is required
        /// but unverified.
        /// </summary>
        public static string ConnectionStringFromSecretDirectory(string directory, string caFile)
        {
            if (string.IsNullOrWhiteSpace(directory) || !Path.IsPathFullyQualified(directory))
                throw new InvalidDataException("MAG_CHARACTER_DB_SECRET_DIR must be an absolute directory.");
            string Read(string name)
            {
                string file = Path.Combine(directory, name);
                if (!File.Exists(file)) throw new InvalidDataException($"MAG_CHARACTER_DB_SECRET_DIR has no '{name}' file.");
                return File.ReadAllText(file).Trim();
            }
            if (!int.TryParse(Read("port"), System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out int port) ||
                port < 1 || port > 65535)
                throw new InvalidDataException("MAG_CHARACTER_DB_SECRET_DIR 'port' is not a TCP port.");
            var builder = new NpgsqlConnectionStringBuilder
            {
                Host = Read("host"), Port = port, Database = Read("dbname"), Username = Read("user"), Password = Read("password"),
                ApplicationName = "mag-medius", Timeout = 2, CommandTimeout = 3, MaxPoolSize = 8,
                SslMode = SslMode.Require,
                // TLS protects the connection. Npgsql's default (Prefer) first tries GSSAPI encryption,
                // which the image has no library for: every process logged a libgssapi_krb5 load error.
                GssEncryptionMode = GssEncryptionMode.Disable
            };
            if (!string.IsNullOrWhiteSpace(caFile))
            {
                if (!Path.IsPathFullyQualified(caFile) || !File.Exists(caFile))
                    throw new InvalidDataException("MAG_CHARACTER_DB_CA_FILE must name an existing absolute file.");
                builder.SslMode = SslMode.VerifyFull;
                builder.RootCertificate = caFile;
            }
            return builder.ConnectionString;
        }

        /// <summary>Fail fast on a retention outside whole days from 1 to 90; unset is the default.</summary>
        public static TimeSpan ParseLedgerRetention(string value)
        {
            if (string.IsNullOrWhiteSpace(value)) return DefaultLedgerRetention;
            int minimum = (int)MinimumLedgerRetention.TotalDays, maximum = (int)MaximumLedgerRetention.TotalDays;
            if (!int.TryParse(value.Trim(), System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out int days) ||
                days < minimum || days > maximum)
                throw new InvalidDataException($"MAG_REWARD_LEDGER_RETENTION_DAYS must be a whole number of days between {minimum} and {maximum}.");
            return TimeSpan.FromDays(days);
        }

        internal NpgsqlConnection OpenConnection()
        {
            if (Environment.TickCount64 < Interlocked.Read(ref _unavailableUntilTicks))
                throw new CharacterStoreUnavailableException("Character database unavailable (recent failure; retrying shortly).");
            try { return _source.OpenConnection(); }
            catch (Exception e) { throw Translate(e); }
        }

        /// <summary>Maps a driver or server failure onto the store's exception contract.</summary>
        internal Exception Translate(Exception e)
        {
            switch (e)
            {
                case CharacterStoreUnavailableException or CharacterStoreConflictException or InvalidDataException:
                    return e;
                case PostgresException pg when pg.SqlState == PostgresErrorCodes.UniqueViolation ||
                    pg.SqlState == PostgresErrorCodes.SerializationFailure || pg.SqlState == PostgresErrorCodes.DeadlockDetected:
                    return new CharacterStoreConflictException("Character database write lost a concurrent race.", e);
                case PostgresException pg when pg.SqlState.StartsWith("23", StringComparison.Ordinal) ||
                    pg.SqlState.StartsWith("22", StringComparison.Ordinal):
                    // Integrity or data errors: the change itself is invalid, not the connection.
                    return new InvalidDataException($"Character database refused the change ({pg.SqlState}).", e);
                case PostgresException pg when pg.SqlState.StartsWith("08", StringComparison.Ordinal) ||
                    pg.SqlState.StartsWith("53", StringComparison.Ordinal) || pg.SqlState.StartsWith("57", StringComparison.Ordinal) ||
                    pg.SqlState == PostgresErrorCodes.ReadOnlySqlTransaction:
                    Trip();
                    return new CharacterStoreUnavailableException($"Character database unavailable ({pg.SqlState}).", e);
                case PostgresException pg:
                    return new CharacterStoreUnavailableException($"Character database error ({pg.SqlState}).", e);
                case NpgsqlException or TimeoutException or System.Net.Sockets.SocketException or InvalidOperationException:
                    Trip();
                    return new CharacterStoreUnavailableException("Character database unavailable: " + e.GetType().Name + ".", e);
                default:
                    return e;
            }
        }

        private void Trip() =>
            Interlocked.Exchange(ref _unavailableUntilTicks, Environment.TickCount64 + (long)UnavailableBackoff.TotalMilliseconds);

        // ---- Migrations ----

        internal sealed record Migration(int Version, string Name, string Sql, string Sha256);

        internal static IReadOnlyList<Migration> KnownMigrations { get; } = LoadMigrations();

        private static IReadOnlyList<Migration> LoadMigrations()
        {
            var assembly = typeof(PostgresCharacterDatabase).Assembly;
            var list = new List<Migration>();
            foreach (string resource in assembly.GetManifestResourceNames().Where(n => n.StartsWith(MigrationResourcePrefix, StringComparison.Ordinal)))
            {
                string name = resource.Substring(MigrationResourcePrefix.Length);
                int underscore = name.IndexOf('_');
                if (underscore != 4 || !name.EndsWith(".sql", StringComparison.Ordinal) || !int.TryParse(name.AsSpan(0, 4), out int version))
                    throw new InvalidDataException($"Malformed character-store migration name {name}.");
                using var stream = assembly.GetManifestResourceStream(resource);
                using var memory = new MemoryStream();
                stream.CopyTo(memory);
                byte[] bytes = memory.ToArray();
                list.Add(new Migration(version, name, Encoding.UTF8.GetString(bytes), Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant()));
            }
            list.Sort((a, b) => a.Version.CompareTo(b.Version));
            for (int i = 0; i < list.Count; i++)
                if (list[i].Version != i + 1) throw new InvalidDataException("Character-store migrations must be numbered 0001 upward without gaps.");
            return list;
        }

        public int LatestSchemaVersion => KnownMigrations.Count;

        private static Dictionary<int, (string Name, string Sha)> ReadApplied(NpgsqlConnection connection)
        {
            var applied = new Dictionary<int, (string, string)>();
            using var exists = new NpgsqlCommand("SELECT to_regclass('mag.schema_migrations') IS NOT NULL", connection);
            if (!(bool)exists.ExecuteScalar()) return applied;
            using var command = new NpgsqlCommand("SELECT version, name, sha256 FROM mag.schema_migrations ORDER BY version", connection);
            using var reader = command.ExecuteReader();
            while (reader.Read()) applied[reader.GetInt32(0)] = (reader.GetString(1), reader.GetString(2));
            return applied;
        }

        private static void CheckApplied(Dictionary<int, (string Name, string Sha)> applied)
        {
            foreach (var (version, row) in applied.OrderBy(e => e.Key))
            {
                if (version > KnownMigrations.Count)
                    throw new InvalidDataException($"Character database schema version {version} is newer than this build ({KnownMigrations.Count}); " +
                        "switch MAG_CHARACTER_STORE back to json (after an export) instead of running an older build.");
                var known = KnownMigrations[version - 1];
                if (row.Sha != known.Sha256)
                    throw new InvalidDataException($"Character database migration {version} ({row.Name}) differs from this build's copy.");
            }
        }

        /// <summary>Applies every pending migration under an advisory lock; returns how many were applied.</summary>
        public int Migrate()
        {
            try
            {
                using var connection = OpenConnection();
                using (var lockCommand = new NpgsqlCommand("SELECT pg_advisory_lock(@key)", connection))
                {
                    lockCommand.Parameters.AddWithValue("key", MigrationLockKey);
                    lockCommand.CommandTimeout = MaintenanceCommandTimeoutSeconds;
                    lockCommand.ExecuteNonQuery();
                }
                try
                {
                    using (var setup = new NpgsqlCommand(
                        "CREATE SCHEMA IF NOT EXISTS mag; CREATE TABLE IF NOT EXISTS mag.schema_migrations (" +
                        "version integer PRIMARY KEY, name text NOT NULL, sha256 text NOT NULL, applied_at timestamptz NOT NULL DEFAULT now())",
                        connection))
                        setup.ExecuteNonQuery();
                    var applied = ReadApplied(connection);
                    CheckApplied(applied);
                    int count = 0;
                    foreach (var migration in KnownMigrations.Where(m => !applied.ContainsKey(m.Version)))
                    {
                        using var transaction = connection.BeginTransaction();
                        using (var apply = new NpgsqlCommand(migration.Sql, connection, transaction) { CommandTimeout = MaintenanceCommandTimeoutSeconds })
                            apply.ExecuteNonQuery();
                        using (var record = new NpgsqlCommand(
                            "INSERT INTO mag.schema_migrations (version, name, sha256) VALUES (@version, @name, @sha)", connection, transaction))
                        {
                            record.Parameters.AddWithValue("version", migration.Version);
                            record.Parameters.AddWithValue("name", migration.Name);
                            record.Parameters.AddWithValue("sha", migration.Sha256);
                            record.ExecuteNonQuery();
                        }
                        transaction.Commit();
                        count++;
                    }
                    return count;
                }
                finally
                {
                    using var unlock = new NpgsqlCommand("SELECT pg_advisory_unlock(@key)", connection);
                    unlock.Parameters.AddWithValue("key", MigrationLockKey);
                    unlock.ExecuteNonQuery();
                }
            }
            catch (Exception e) { throw Translate(e); }
        }

        /// <summary>Applied schema version (0 when none); refuses unknown or altered migrations.</summary>
        public int SchemaVersion()
        {
            try
            {
                using var connection = OpenConnection();
                var applied = ReadApplied(connection);
                CheckApplied(applied);
                return applied.Count == 0 ? 0 : applied.Keys.Max();
            }
            catch (Exception e) { throw Translate(e); }
        }

        /// <summary>Refuses to run against a schema this build does not exactly match.</summary>
        public void RequireCurrentSchema()
        {
            int version = SchemaVersion();
            if (version != KnownMigrations.Count)
                throw new InvalidDataException($"Character database schema is at version {version}; this build needs {KnownMigrations.Count}. " +
                    "Run 'mag-admin character-store migrate' or start main Medius.");
        }

        /// <summary>Tables reported by <c>character-store status</c>, in schema order.</summary>
        internal static readonly string[] StatusTables =
            { "characters", "character_loadouts", "match_progress", "reward_receipts", "match_history", "admin_changes", "voice_sanctions" };

        /// <summary>Row count of each table in <see cref="StatusTables"/>, and when the last import wrote (null if never).</summary>
        public (IReadOnlyList<(string Table, long Rows)> Tables, DateTime? LastImportAt) RowCounts()
        {
            try
            {
                using var connection = OpenConnection();
                var tables = new List<(string, long)>();
                foreach (string table in StatusTables)
                {
                    using var count = new NpgsqlCommand($"SELECT count(*) FROM mag.{table}", connection) { CommandTimeout = MaintenanceCommandTimeoutSeconds };
                    tables.Add((table, (long)count.ExecuteScalar()));
                }
                using var marker = new NpgsqlCommand("SELECT max(at) FROM mag.admin_changes WHERE operation = @operation", connection);
                marker.Parameters.AddWithValue("operation", LocalCharacterStore.ImportOperation);
                object at = marker.ExecuteScalar();
                return (tables, at is DateTime value ? DateTime.SpecifyKind(value, DateTimeKind.Utc) : null);
            }
            catch (Exception e) { throw Translate(e); }
        }

        // ---- Retention ----

        /// <summary>
        /// Prunes match rows whose last reward is older than <paramref name="retention"/>, each together
        /// with all its receipts, and keeps a summary in mag.match_history. Rows and receipts are never
        /// pruned apart: a finalized row without its receipts would refuse a redelivered match result
        /// forever. Only one process prunes at a time; the others skip.
        /// </summary>
        public CharacterLedgerPruneResult PruneLedger(TimeSpan retention, int batchSize = 1000)
        {
            if (retention < MinimumLedgerRetention || retention > MaximumLedgerRetention)
                throw new ArgumentOutOfRangeException(nameof(retention));
            DateTime cutoff = (Clock() - retention).UtcDateTime;
            int rows = 0, finalized = 0, receipts = 0, history = 0;
            try
            {
                while (true)
                {
                    using var connection = OpenConnection();
                    using var transaction = connection.BeginTransaction();
                    using (var lockCommand = new NpgsqlCommand("SELECT pg_try_advisory_xact_lock(@key)", connection, transaction))
                    {
                        lockCommand.Parameters.AddWithValue("key", PruneLockKey);
                        if (!(bool)lockCommand.ExecuteScalar()) return new CharacterLedgerPruneResult(true, rows, finalized, receipts, history);
                    }
                    using var prune = new NpgsqlCommand(@"
WITH doomed AS (
  SELECT character_id, match_key FROM mag.match_progress
   WHERE last_reward_at < @cutoff ORDER BY last_reward_at LIMIT @batch FOR UPDATE SKIP LOCKED),
counted AS (
  SELECT d.character_id, d.match_key,
         (SELECT count(*) FROM mag.reward_receipts r
           WHERE r.character_id = d.character_id AND r.match_key = d.match_key)::integer AS rewards
    FROM doomed d),
history AS (
  INSERT INTO mag.match_history (character_id, match_key, experience, frago_points, finalized, result_kind, rewards,
                                 first_reward_at, last_reward_at)
  SELECT p.character_id, p.match_key, p.experience, p.frago_points, p.finalized, p.result_kind, k.rewards,
         p.first_reward_at, p.last_reward_at
    FROM mag.match_progress p JOIN counted k ON k.character_id = p.character_id AND k.match_key = p.match_key
  ON CONFLICT (character_id, match_key) DO UPDATE SET
    experience = mag.match_history.experience + EXCLUDED.experience,
    frago_points = CASE WHEN mag.match_history.frago_points IS NULL AND EXCLUDED.frago_points IS NULL THEN NULL
                        ELSE coalesce(mag.match_history.frago_points, 0) + coalesce(EXCLUDED.frago_points, 0) END,
    finalized = mag.match_history.finalized OR EXCLUDED.finalized,
    result_kind = coalesce(EXCLUDED.result_kind, mag.match_history.result_kind),
    rewards = mag.match_history.rewards + EXCLUDED.rewards,
    first_reward_at = least(mag.match_history.first_reward_at, EXCLUDED.first_reward_at),
    last_reward_at = greatest(mag.match_history.last_reward_at, EXCLUDED.last_reward_at)
  RETURNING 1),
gone AS (
  DELETE FROM mag.match_progress p USING doomed d
   WHERE p.character_id = d.character_id AND p.match_key = d.match_key
  RETURNING p.finalized)
SELECT (SELECT count(*) FROM gone)::integer, (SELECT count(*) FROM gone WHERE finalized)::integer,
       (SELECT coalesce(sum(rewards), 0) FROM counted)::integer, (SELECT count(*) FROM history)::integer", connection, transaction)
                    { CommandTimeout = MaintenanceCommandTimeoutSeconds };
                    prune.Parameters.AddWithValue("cutoff", cutoff);
                    prune.Parameters.AddWithValue("batch", batchSize);
                    int batchRows;
                    using (var reader = prune.ExecuteReader())
                    {
                        reader.Read();
                        batchRows = reader.GetInt32(0);
                        rows += batchRows; finalized += reader.GetInt32(1); receipts += reader.GetInt32(2); history += reader.GetInt32(3);
                    }
                    transaction.Commit();
                    if (batchRows < batchSize) return new CharacterLedgerPruneResult(false, rows, finalized, receipts, history);
                }
            }
            catch (Exception e) { throw Translate(e); }
        }

        public void Dispose() => _source.Dispose();
    }
}
