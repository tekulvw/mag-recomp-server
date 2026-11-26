using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Npgsql;
using NpgsqlTypes;

namespace Server.Medius
{
    /// <summary>Counts and a digest of one store's canonical contents; never player data.</summary>
    public sealed record CharacterStoreSummary(int Characters, int Receipts, int MatchRows, string Digest)
    {
        public string Describe() => $"{Characters} characters, {Receipts} reward receipts, {MatchRows} match rows, digest {Digest}";
    }

    public sealed partial class LocalCharacterStore
    {
        /// <summary>A store over PostgreSQL. Rules, catalogs and policy properties behave exactly as for the
        /// JSON file; see docs/superpowers/specs/2026-11-25-postgres-character-store-design.md.</summary>
        public static LocalCharacterStore ForPostgres(PostgresCharacterDatabase database) => new LocalCharacterStore(database);

        private LocalCharacterStore(PostgresCharacterDatabase database)
        {
            _path = null;
            _committer = new AtomicCharacterStoreCommitter();
            _backend = new PostgresBackend(this, database ?? throw new ArgumentNullException(nameof(database)));
        }

        /// <summary>"json" or "postgres".</summary>
        public string BackendName => _backend is PostgresBackend ? "postgres" : "json";

        private const string CharacterColumns = "id, account, name, faction, head_id, voice_id, flags, universe_id, np_id, experience, level, " +
            "skill_points, owned_skills, unlocked_items, last_used_loadout, loadout_layout_revision, leadership_level, nominations, " +
            "matches_since_last_command, leadership_points, leadership_level_floor";

        private sealed class PostgresBackend : ICharacterBackend
        {
            internal readonly LocalCharacterStore Store;
            internal readonly PostgresCharacterDatabase Database;
            public PostgresBackend(LocalCharacterStore store, PostgresCharacterDatabase database) { Store = store; Database = database; }

            public IReadOnlyList<StoredCharacter> ReadAll() => Read("SELECT " + CharacterColumns + " FROM mag.characters ORDER BY id", null);
            public IReadOnlyList<StoredCharacter> ReadForAccount(string account) =>
                Read("SELECT " + CharacterColumns + " FROM mag.characters WHERE account = @account ORDER BY id", account);

            private IReadOnlyList<StoredCharacter> Read(string sql, string account)
            {
                try
                {
                    using var connection = Database.OpenConnection();
                    using var command = new NpgsqlCommand(sql, connection);
                    if (account != null) command.Parameters.Add(Text("account", account));
                    var characters = ReadCharacters(command);
                    LoadLoadouts(connection, null, characters);
                    foreach (var character in characters) Store.PrepareLoaded(character);
                    return characters;
                }
                catch (Exception e) { throw Database.Translate(e); }
            }

            public StoreSession Open(StoreScope scope) => new PostgresSession(this, scope);
        }

        private void PrepareLoaded(StoredCharacter character)
        {
            if (!Valid(character, 3)) throw new InvalidDataException("Character-store row violates the local account policy.");
            PrepareCharacter(character);
        }

        /// <summary>One transaction. The account's character row is locked, so all writes for one character
        /// serialize in the database while other characters proceed in parallel.</summary>
        private sealed class PostgresSession : StoreSession
        {
            private readonly PostgresBackend _backend;
            private readonly NpgsqlConnection _connection;
            private readonly NpgsqlTransaction _transaction;
            private readonly HashSet<uint> _loadedCharacters = new();
            private readonly HashSet<string> _loadedReceipts = new(StringComparer.Ordinal);
            private (string Operation, string Account, string Before)? _audit;
            private bool _committed;

            public PostgresSession(PostgresBackend backend, StoreScope scope)
            {
                _backend = backend;
                var database = backend.Database;
                _connection = database.OpenConnection();
                try
                {
                    _transaction = _connection.BeginTransaction();
                    using var characters = new NpgsqlCommand("SELECT " + CharacterColumns + " FROM mag.characters " +
                        "WHERE account = @account OR (@name::text IS NOT NULL AND lower(name) = lower(@name::text)) ORDER BY id FOR UPDATE",
                        _connection, _transaction);
                    characters.Parameters.Add(Text("account", scope.Account ?? ""));
                    characters.Parameters.Add(Text("name", scope.Name));
                    var list = ReadCharacters(characters);
                    LoadLoadouts(_connection, _transaction, list);
                    foreach (var character in list) { backend.Store.PrepareLoaded(character); _loadedCharacters.Add(character.Id); }
                    var receipts = new Dictionary<string, RewardReceipt>(StringComparer.Ordinal);
                    var progress = new Dictionary<string, MatchRewardProgress>(StringComparer.Ordinal);
                    if (scope.RewardId != null)
                    {
                        using var receipt = new NpgsqlCommand("SELECT r.reward_id, c.account, r.character_id, r.match_key, r.kind, r.units, " +
                            "r.leadership::text, r.frago, r.outcome::text FROM mag.reward_receipts r JOIN mag.characters c ON c.id = r.character_id " +
                            "WHERE r.reward_id = @reward", _connection, _transaction);
                        receipt.Parameters.Add(Text("reward", scope.RewardId));
                        foreach (var (id, row) in ReadReceipts(receipt)) { receipts[id] = row; _loadedReceipts.Add(id); }
                    }
                    var owner = list.SingleOrDefault(c => c.Account == scope.Account && c.Id == scope.CharacterId);
                    if (scope.MatchKey != null && owner != null)
                    {
                        using var match = new NpgsqlCommand("SELECT character_id, match_key, experience, fraction, frago_points, finalized, last_reward_at " +
                            "FROM mag.match_progress WHERE character_id = @character AND match_key = @match FOR UPDATE", _connection, _transaction);
                        match.Parameters.Add(new NpgsqlParameter("character", NpgsqlDbType.Bigint) { Value = (long)owner.Id });
                        match.Parameters.Add(Text("match", scope.MatchKey));
                        foreach (var row in ReadProgress(match, id => owner.Account))
                            progress[MatchProgressKey(row.Account, row.CharacterId, row.MatchKey)] = row;
                    }
                    Document = new Document { SchemaVersion = 3, Account = "player", DisplayName = "Player", Characters = list,
                        RewardReceipts = receipts, MatchRewardProgress = progress };
                }
                catch (Exception e)
                {
                    _transaction?.Dispose();
                    _connection.Dispose();
                    throw database.Translate(e);
                }
            }

            public override int? CharacterLimit => null;

            public override uint NextCharacterId()
            {
                try
                {
                    using var command = new NpgsqlCommand("SELECT nextval('mag.character_ids')", _connection, _transaction);
                    return checked((uint)(long)command.ExecuteScalar());
                }
                catch (Exception e) { throw _backend.Database.Translate(e); }
            }

            public override string Backup(string backupDirectory, string operation, string account, byte level)
            {
                var before = Document.Characters.Single(c => c.Account == account);
                _audit = (operation, account, JsonSerializer.Serialize(before, JsonOptions));
                return "mag.admin_changes";
            }

            public override void Commit(string touchedProgress = null)
            {
                if (_committed) throw new InvalidOperationException("Character-store session already committed.");
                var store = _backend.Store;
                try
                {
                    foreach (var character in Document.Characters)
                    {
                        if (store.ProgressionCatalog != null) { character.Experience ??= 0; character.Level ??= 1; }
                        WriteCharacter(_connection, _transaction, character, insert: !_loadedCharacters.Contains(character.Id));
                    }
                    var newReceipts = Document.RewardReceipts.Where(e => !_loadedReceipts.Contains(e.Key)).ToList();
                    if (touchedProgress != null && Document.MatchRewardProgress.TryGetValue(touchedProgress, out var row))
                    {
                        string resultKind = newReceipts.Select(e => e.Value)
                            .Where(r => r.CharacterId == row.CharacterId && r.MatchKey == row.MatchKey && MagProgressionCatalog.IsFinalRewardKind(r.Kind))
                            .Select(r => r.Kind).LastOrDefault();
                        WriteProgress(_connection, _transaction, row, resultKind, store.Clock());
                    }
                    foreach (var (id, receipt) in newReceipts)
                        if (!InsertReceipt(_connection, _transaction, id, receipt, null))
                            throw new CharacterStoreConflictException("Reward receipt committed concurrently.");
                    if (_audit is { } change)
                    {
                        var (operation, account, before) = change;
                        using var audit = new NpgsqlCommand("INSERT INTO mag.admin_changes (operation, account, before, after) " +
                            "VALUES (@operation, @account, @before, @after)", _connection, _transaction);
                        audit.Parameters.Add(Text("operation", operation));
                        audit.Parameters.Add(Text("account", account));
                        audit.Parameters.Add(new NpgsqlParameter("before", NpgsqlDbType.Jsonb) { Value = before });
                        audit.Parameters.Add(new NpgsqlParameter("after", NpgsqlDbType.Jsonb)
                            { Value = JsonSerializer.Serialize(Document.Characters.Single(c => c.Account == account), JsonOptions) });
                        audit.ExecuteNonQuery();
                    }
                    _transaction.Commit();
                    _committed = true;
                }
                catch (Exception e) { throw _backend.Database.Translate(e); }
            }

            public override void Dispose()
            {
                _transaction.Dispose(); // rolls back unless committed
                _connection.Dispose();
            }
        }

        // ---- Row mapping ----

        private static NpgsqlParameter Text(string name, string value) =>
            new NpgsqlParameter(name, NpgsqlDbType.Text) { Value = (object)value ?? DBNull.Value };
        private static NpgsqlParameter Typed(string name, NpgsqlDbType type, object value) =>
            new NpgsqlParameter(name, type) { Value = value ?? DBNull.Value };

        private static List<StoredCharacter> ReadCharacters(NpgsqlCommand command)
        {
            var list = new List<StoredCharacter>();
            using var r = command.ExecuteReader();
            while (r.Read())
            {
                byte? Small(int i) => r.IsDBNull(i) ? null : checked((byte)r.GetInt16(i));
                uint[] Mask(int i) => r.IsDBNull(i) ? null : r.GetFieldValue<long[]>(i).Select(v => checked((uint)v)).ToArray();
                list.Add(new StoredCharacter
                {
                    Id = checked((uint)r.GetInt64(0)), Account = r.GetString(1), Name = r.GetString(2),
                    Faction = Small(3).Value, HeadId = Small(4).Value, VoiceId = Small(5).Value, Flags = Small(6).Value,
                    UniverseId = checked((uint)r.GetInt64(7)), NpId = r.GetFieldValue<byte[]>(8),
                    Experience = r.IsDBNull(9) ? null : r.GetInt32(9), Level = Small(10), SkillPoints = Small(11),
                    OwnedSkills = Mask(12), UnlockedItems = Mask(13), LastUsedLoadout = Small(14).Value,
                    LoadoutLayoutRevision = decimal.ToUInt64(r.GetDecimal(15)),
                    Leadership = r.IsDBNull(16) ? null : new StoredLeadershipRecord
                    {
                        LeadershipLevel = Small(16).Value, Nominations = Small(17).Value, MatchesSinceLastCommand = Small(18).Value,
                        LeadershipPoints = r.IsDBNull(19) ? null : checked((uint)r.GetInt64(19)), LeadershipLevelFloor = Small(20)
                    }
                });
            }
            return list;
        }

        private static void LoadLoadouts(NpgsqlConnection connection, NpgsqlTransaction transaction, List<StoredCharacter> characters)
        {
            if (characters.Count == 0) return;
            var byId = characters.ToDictionary(c => c.Id);
            using var command = new NpgsqlCommand("SELECT character_id, slot, update_id, items FROM mag.character_loadouts " +
                "WHERE character_id = ANY(@ids) ORDER BY character_id, slot", connection, transaction);
            command.Parameters.Add(new NpgsqlParameter("ids", NpgsqlDbType.Array | NpgsqlDbType.Bigint) { Value = characters.Select(c => (long)c.Id).ToArray() });
            var rows = new Dictionary<uint, StoredLoadout[]>();
            using (var r = command.ExecuteReader())
                while (r.Read())
                {
                    uint id = checked((uint)r.GetInt64(0));
                    if (!rows.TryGetValue(id, out var kits)) rows[id] = kits = new StoredLoadout[5];
                    kits[r.GetInt16(1)] = new StoredLoadout { UpdateId = checked((byte)r.GetInt16(2)), Items = r.GetFieldValue<short[]>(3) };
                }
            foreach (var (id, kits) in rows)
            {
                if (kits.Any(k => k == null)) throw new InvalidDataException("Character-store loadout rows are incomplete.");
                byId[id].Loadouts = kits;
            }
        }

        private static void WriteCharacter(NpgsqlConnection connection, NpgsqlTransaction transaction, StoredCharacter c, bool insert)
        {
            string sql = insert
                ? "INSERT INTO mag.characters (" + CharacterColumns + ") VALUES (@id, @account, @name, @faction, @head, @voice, @flags, @universe, " +
                  "@np, @experience, @level, @skill_points, @owned_skills, @unlocked_items, @last_used, @layout, @leadership_level, @nominations, " +
                  "@recency, @leadership_points, @leadership_floor)"
                : "UPDATE mag.characters SET account = @account, name = @name, faction = @faction, head_id = @head, voice_id = @voice, " +
                  "flags = @flags, universe_id = @universe, np_id = @np, experience = @experience, level = @level, skill_points = @skill_points, " +
                  "owned_skills = @owned_skills, unlocked_items = @unlocked_items, last_used_loadout = @last_used, loadout_layout_revision = @layout, " +
                  "leadership_level = @leadership_level, nominations = @nominations, matches_since_last_command = @recency, " +
                  "leadership_points = @leadership_points, leadership_level_floor = @leadership_floor, revision = revision + 1, updated_at = now() " +
                  "WHERE id = @id";
            using (var command = new NpgsqlCommand(sql, connection, transaction))
            {
                var p = command.Parameters;
                p.Add(Typed("id", NpgsqlDbType.Bigint, (long)c.Id));
                p.Add(Text("account", c.Account));
                p.Add(Text("name", c.Name));
                p.Add(Typed("faction", NpgsqlDbType.Smallint, (short)c.Faction));
                p.Add(Typed("head", NpgsqlDbType.Smallint, (short)c.HeadId));
                p.Add(Typed("voice", NpgsqlDbType.Smallint, (short)c.VoiceId));
                p.Add(Typed("flags", NpgsqlDbType.Smallint, (short)c.Flags));
                p.Add(Typed("universe", NpgsqlDbType.Bigint, (long)c.UniverseId));
                p.Add(Typed("np", NpgsqlDbType.Bytea, c.NpId));
                p.Add(Typed("experience", NpgsqlDbType.Integer, c.Experience));
                p.Add(Typed("level", NpgsqlDbType.Smallint, c.Level is byte level ? (short)level : null));
                p.Add(Typed("skill_points", NpgsqlDbType.Smallint, c.SkillPoints is byte points ? (short)points : null));
                p.Add(Typed("owned_skills", NpgsqlDbType.Array | NpgsqlDbType.Bigint, c.OwnedSkills?.Select(v => (long)v).ToArray()));
                p.Add(Typed("unlocked_items", NpgsqlDbType.Array | NpgsqlDbType.Bigint, c.UnlockedItems?.Select(v => (long)v).ToArray()));
                p.Add(Typed("last_used", NpgsqlDbType.Smallint, (short)c.LastUsedLoadout));
                p.Add(Typed("layout", NpgsqlDbType.Numeric, (decimal)c.LoadoutLayoutRevision));
                var lead = c.Leadership;
                p.Add(Typed("leadership_level", NpgsqlDbType.Smallint, lead == null ? null : (short)lead.LeadershipLevel));
                p.Add(Typed("nominations", NpgsqlDbType.Smallint, lead == null ? null : (short)lead.Nominations));
                p.Add(Typed("recency", NpgsqlDbType.Smallint, lead == null ? null : (short)lead.MatchesSinceLastCommand));
                p.Add(Typed("leadership_points", NpgsqlDbType.Bigint, lead?.LeadershipPoints is uint lp ? (long)lp : null));
                p.Add(Typed("leadership_floor", NpgsqlDbType.Smallint, lead?.LeadershipLevelFloor is byte floor ? (short)floor : null));
                if (command.ExecuteNonQuery() != 1) throw new CharacterStoreConflictException("Character row changed concurrently.");
            }
            if (c.Loadouts == null)
            {
                if (insert) return;
                using var clear = new NpgsqlCommand("DELETE FROM mag.character_loadouts WHERE character_id = @id", connection, transaction);
                clear.Parameters.Add(Typed("id", NpgsqlDbType.Bigint, (long)c.Id));
                clear.ExecuteNonQuery();
                return;
            }
            for (int slot = 0; slot < c.Loadouts.Length; slot++)
            {
                using var kit = new NpgsqlCommand("INSERT INTO mag.character_loadouts (character_id, slot, update_id, items) " +
                    "VALUES (@id, @slot, @update, @items) ON CONFLICT (character_id, slot) DO UPDATE SET update_id = EXCLUDED.update_id, " +
                    "items = EXCLUDED.items", connection, transaction);
                kit.Parameters.Add(Typed("id", NpgsqlDbType.Bigint, (long)c.Id));
                kit.Parameters.Add(Typed("slot", NpgsqlDbType.Smallint, (short)slot));
                kit.Parameters.Add(Typed("update", NpgsqlDbType.Smallint, (short)c.Loadouts[slot].UpdateId));
                kit.Parameters.Add(Typed("items", NpgsqlDbType.Array | NpgsqlDbType.Smallint, c.Loadouts[slot].Items));
                kit.ExecuteNonQuery();
            }
        }

        private static void WriteProgress(NpgsqlConnection connection, NpgsqlTransaction transaction, MatchRewardProgress row, string resultKind,
            DateTimeOffset now)
        {
            DateTime rewarded = row.LastRewardAt is long seconds ? DateTimeOffset.FromUnixTimeSeconds(seconds).UtcDateTime : now.UtcDateTime;
            using var command = new NpgsqlCommand("INSERT INTO mag.match_progress (character_id, match_key, experience, fraction, frago_points, " +
                "finalized, result_kind, first_reward_at, last_reward_at) VALUES (@character, @match, @experience, @fraction, @frago, @finalized, " +
                "@result, @rewarded, @rewarded) ON CONFLICT (character_id, match_key) DO UPDATE SET experience = EXCLUDED.experience, " +
                "fraction = EXCLUDED.fraction, frago_points = EXCLUDED.frago_points, finalized = EXCLUDED.finalized, " +
                "result_kind = coalesce(EXCLUDED.result_kind, mag.match_progress.result_kind), last_reward_at = EXCLUDED.last_reward_at",
                connection, transaction);
            AddProgress(command, row, resultKind, rewarded);
            command.ExecuteNonQuery();
        }

        private static void AddProgress(NpgsqlCommand command, MatchRewardProgress row, string resultKind, DateTime rewarded)
        {
            var p = command.Parameters;
            p.Add(Typed("character", NpgsqlDbType.Bigint, (long)row.CharacterId));
            p.Add(Text("match", row.MatchKey));
            p.Add(Typed("experience", NpgsqlDbType.Integer, row.Experience));
            p.Add(Typed("fraction", NpgsqlDbType.Real, row.Fraction));
            p.Add(Typed("frago", NpgsqlDbType.Integer, row.FragoPoints));
            p.Add(Typed("finalized", NpgsqlDbType.Boolean, row.Finalized));
            p.Add(Text("result", resultKind));
            p.Add(Typed("rewarded", NpgsqlDbType.TimestampTz, rewarded));
        }

        /// <summary>Inserts one receipt; false when the reward id was already committed.</summary>
        private static bool InsertReceipt(NpgsqlConnection connection, NpgsqlTransaction transaction, string id, RewardReceipt receipt, DateTime? committedAt)
        {
            using var command = new NpgsqlCommand("INSERT INTO mag.reward_receipts (reward_id, character_id, match_key, kind, units, leadership, " +
                "frago, outcome, committed_at) VALUES (@id, @character, @match, @kind, @units, @leadership, @frago, @outcome, " +
                "coalesce(@committed, now())) ON CONFLICT (reward_id) DO NOTHING", connection, transaction);
            var p = command.Parameters;
            p.Add(Text("id", id));
            p.Add(Typed("character", NpgsqlDbType.Bigint, (long)receipt.CharacterId));
            p.Add(Text("match", receipt.MatchKey));
            p.Add(Text("kind", receipt.Kind));
            p.Add(Typed("units", NpgsqlDbType.Integer, receipt.Units));
            p.Add(Typed("leadership", NpgsqlDbType.Jsonb, receipt.Leadership == null ? null : JsonSerializer.Serialize(receipt.Leadership, JsonOptions)));
            p.Add(Typed("frago", NpgsqlDbType.Boolean, receipt.Frago));
            p.Add(Typed("outcome", NpgsqlDbType.Jsonb, JsonSerializer.Serialize(receipt.Snapshot, JsonOptions)));
            p.Add(Typed("committed", NpgsqlDbType.TimestampTz, committedAt));
            return command.ExecuteNonQuery() == 1;
        }

        private static List<(string Id, RewardReceipt Receipt)> ReadReceipts(NpgsqlCommand command)
        {
            var list = new List<(string, RewardReceipt)>();
            using var r = command.ExecuteReader();
            while (r.Read())
                list.Add((r.GetString(0), new RewardReceipt
                {
                    Account = r.GetString(1), CharacterId = checked((uint)r.GetInt64(2)), MatchKey = r.GetString(3), Kind = r.GetString(4),
                    Units = r.GetInt32(5),
                    Leadership = r.IsDBNull(6) ? null : JsonSerializer.Deserialize<GameplayLeadershipResult>(r.GetString(6), JsonOptions),
                    Frago = r.IsDBNull(7) ? null : r.GetBoolean(7),
                    Snapshot = JsonSerializer.Deserialize<ProgressionRewardOutcome>(r.GetString(8), JsonOptions)
                }));
            return list;
        }

        private static List<MatchRewardProgress> ReadProgress(NpgsqlCommand command, Func<uint, string> accountOf)
        {
            var list = new List<MatchRewardProgress>();
            using var r = command.ExecuteReader();
            while (r.Read())
            {
                uint id = checked((uint)r.GetInt64(0));
                list.Add(new MatchRewardProgress
                {
                    Account = accountOf(id), CharacterId = id, MatchKey = r.GetString(1), Experience = r.GetInt32(2), Fraction = r.GetFloat(3),
                    FragoPoints = r.IsDBNull(4) ? null : r.GetInt32(4), Finalized = r.GetBoolean(5),
                    LastRewardAt = new DateTimeOffset(DateTime.SpecifyKind(r.GetDateTime(6), DateTimeKind.Utc)).ToUnixTimeSeconds()
                });
            }
            return list;
        }

        // ---- Whole-store transfer: import, export, canonical comparison ----

        /// <summary>Every character, receipt and match row in this PostgreSQL store, as a JSON-store document.</summary>
        private Document LoadPostgresDocument(PostgresBackend pg)
        {
            try
            {
                using var connection = pg.Database.OpenConnection();
                using var transaction = connection.BeginTransaction(System.Data.IsolationLevel.RepeatableRead);
                using var characters = new NpgsqlCommand("SELECT " + CharacterColumns + " FROM mag.characters ORDER BY id", connection, transaction);
                var list = ReadCharacters(characters);
                LoadLoadouts(connection, transaction, list);
                foreach (var character in list) PrepareLoaded(character);
                var accounts = list.ToDictionary(c => c.Id, c => c.Account);
                using var rows = new NpgsqlCommand("SELECT character_id, match_key, experience, fraction, frago_points, finalized, last_reward_at " +
                    "FROM mag.match_progress ORDER BY last_reward_at, character_id, match_key", connection, transaction)
                    { CommandTimeout = PostgresCharacterDatabase.MaintenanceCommandTimeoutSeconds };
                var progress = new Dictionary<string, MatchRewardProgress>(StringComparer.Ordinal);
                foreach (var row in ReadProgress(rows, id => accounts[id]))
                    progress[MatchProgressKey(row.Account, row.CharacterId, row.MatchKey)] = row;
                using var receipts = new NpgsqlCommand("SELECT r.reward_id, c.account, r.character_id, r.match_key, r.kind, r.units, " +
                    "r.leadership::text, r.frago, r.outcome::text FROM mag.reward_receipts r JOIN mag.characters c ON c.id = r.character_id " +
                    "ORDER BY r.committed_at, r.reward_id", connection, transaction)
                    { CommandTimeout = PostgresCharacterDatabase.MaintenanceCommandTimeoutSeconds };
                var receiptMap = new Dictionary<string, RewardReceipt>(StringComparer.Ordinal);
                foreach (var (id, receipt) in ReadReceipts(receipts)) receiptMap[id] = receipt;
                transaction.Commit();
                return new Document { SchemaVersion = 3, Account = "player", DisplayName = "Player", Characters = list,
                    RewardReceipts = receiptMap, MatchRewardProgress = progress };
            }
            catch (Exception e) { throw pg.Database.Translate(e); }
        }

        private Document LoadWholeDocument() => _backend is PostgresBackend pg ? LoadPostgresDocument(pg) : ReadDocument();

        private static void RequireNoExtensionData(Document document)
        {
            static bool Has(Dictionary<string, JsonElement> data) => data != null && data.Count > 0;
            string field = Has(document.ExtensionData) ? "document"
                : document.Characters.Any(c => Has(c.ExtensionData)) ? "character"
                : document.Characters.Any(c => c.Loadouts?.Any(l => Has(l.ExtensionData)) == true) ? "loadout"
                : document.RewardReceipts?.Values.Any(r => Has(r.ExtensionData) || Has(r.Snapshot.ExtensionData)) == true ? "reward receipt"
                : document.MatchRewardProgress?.Values.Any(r => Has(r.ExtensionData)) == true ? "match row" : null;
            if (field != null)
                throw new InvalidDataException($"The JSON store has fields this build does not know (in a {field}); import refuses to drop them.");
        }

        /// <summary>
        /// One-time import of a JSON store into this (empty) PostgreSQL store, in one transaction. The JSON
        /// file is read with its own validation and stays locked, so no JSON writer can run meanwhile.
        /// </summary>
        public CharacterStoreSummary ImportFrom(LocalCharacterStore source)
        {
            if (_backend is not PostgresBackend pg) throw new InvalidOperationException("Import needs a PostgreSQL target store.");
            if (source?._backend is not JsonBackend) throw new InvalidOperationException("Import needs a JSON source store.");
            using var session = source._backend.Open(new StoreScope(null, CreateDirectory: false));
            var document = session.Document;
            RequireNoExtensionData(document);
            DateTime now = Clock().UtcDateTime;
            try
            {
                using var connection = pg.Database.OpenConnection();
                using var transaction = connection.BeginTransaction();
                using (var count = new NpgsqlCommand("SELECT count(*) FROM mag.characters", connection, transaction))
                    if ((long)count.ExecuteScalar() != 0) throw new InvalidDataException("The PostgreSQL character store is not empty; import refuses to merge.");
                foreach (var character in document.Characters)
                {
                    if (document.SchemaVersion == 3) { character.Experience ??= 0; character.Level ??= 1; }
                    WriteCharacter(connection, transaction, character, insert: true);
                }
                var receipts = document.RewardReceipts ?? new Dictionary<string, RewardReceipt>();
                foreach (var row in (document.MatchRewardProgress ?? new Dictionary<string, MatchRewardProgress>()).Values)
                {
                    string resultKind = receipts.Values.Where(r => r.CharacterId == row.CharacterId && r.MatchKey == row.MatchKey &&
                        MagProgressionCatalog.IsFinalRewardKind(r.Kind)).Select(r => r.Kind).LastOrDefault();
                    DateTime rewarded = row.LastRewardAt is long seconds ? DateTimeOffset.FromUnixTimeSeconds(seconds).UtcDateTime : now;
                    using var insert = new NpgsqlCommand("INSERT INTO mag.match_progress (character_id, match_key, experience, fraction, frago_points, " +
                        "finalized, result_kind, first_reward_at, last_reward_at) VALUES (@character, @match, @experience, @fraction, @frago, " +
                        "@finalized, @result, @rewarded, @rewarded)", connection, transaction);
                    AddProgress(insert, row, resultKind, rewarded);
                    insert.ExecuteNonQuery();
                }
                foreach (var (id, receipt) in receipts)
                    if (!InsertReceipt(connection, transaction, id, receipt, now))
                        throw new InvalidDataException("Duplicate reward receipt in the JSON store.");
                if (document.Characters.Count > 0)
                    using (var sequence = new NpgsqlCommand("SELECT setval('mag.character_ids', @max)", connection, transaction))
                    {
                        sequence.Parameters.Add(Typed("max", NpgsqlDbType.Bigint, (long)document.Characters.Max(c => c.Id)));
                        sequence.ExecuteScalar();
                    }
                transaction.Commit();
            }
            catch (Exception e) { throw pg.Database.Translate(e); }
            return Summarize();
        }

        /// <summary>
        /// Writes this PostgreSQL store to a JSON store (the rollback path), through the JSON store's own
        /// commit, so settled history is compacted to fit its bound. An existing file is replaced only with
        /// <paramref name="replace"/>, and is first kept beside it with a timestamp.
        /// </summary>
        public CharacterStoreSummary ExportTo(LocalCharacterStore target, bool replace)
        {
            if (_backend is not PostgresBackend pg) throw new InvalidOperationException("Export needs a PostgreSQL source store.");
            if (target?._backend is not JsonBackend) throw new InvalidOperationException("Export needs a JSON target store.");
            var document = LoadPostgresDocument(pg);
            if (document.Characters.Count > 128) throw new InvalidDataException("The JSON store holds at most 128 characters.");
            string path = target.ValidPath();
            Directory.CreateDirectory(Path.GetDirectoryName(path));
            using (var session = target._backend.Open(new StoreScope(null)))
            {
                if (File.Exists(path))
                {
                    if (!replace) throw new IOException("The JSON store already exists; pass --replace to keep a copy and replace it.");
                    File.Copy(path, path + ".before-export-" + target.Clock().UtcDateTime.ToString("yyyyMMddTHHmmssZ"), false);
                }
                target.Commit(path, document.Characters, document);
            }
            return target.Summarize();
        }

        /// <summary>Counts and a digest of the canonical contents (see <see cref="CompareWith"/>).</summary>
        public CharacterStoreSummary Summarize()
        {
            var document = LoadWholeDocument();
            var canonical = Canonical(document);
            using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
            foreach (var (key, value) in canonical) hash.AppendData(Encoding.UTF8.GetBytes(key + "\t" + value.ToJsonString() + "\n"));
            return new CharacterStoreSummary(document.Characters.Count, document.RewardReceipts?.Count ?? 0,
                document.MatchRewardProgress?.Count ?? 0, Convert.ToHexString(hash.GetHashAndReset()).ToLowerInvariant()[..16]);
        }

        /// <summary>
        /// Differences between two stores' canonical contents, by record and field name (never values).
        /// Characters are compared with unset XP and level read as 0 and 1, as every reader does; match rows
        /// without their dates, which only PostgreSQL keeps exactly. Empty means the stores agree.
        /// </summary>
        public IReadOnlyList<string> CompareWith(LocalCharacterStore other)
        {
            var mine = Canonical(LoadWholeDocument());
            var theirs = Canonical(other.LoadWholeDocument());
            var differences = new List<string>();
            foreach (var key in mine.Keys.Union(theirs.Keys).OrderBy(k => k, StringComparer.Ordinal))
            {
                if (!mine.TryGetValue(key, out var a)) { differences.Add($"{key}: only in the {other.BackendName} store"); continue; }
                if (!theirs.TryGetValue(key, out var b)) { differences.Add($"{key}: only in the {BackendName} store"); continue; }
                var fields = a.Select(e => e.Key).Union(b.Select(e => e.Key))
                    .Where(name => !JsonNode.DeepEquals(a[name], b[name])).OrderBy(n => n, StringComparer.Ordinal).ToArray();
                if (fields.Length > 0) differences.Add($"{key}: {string.Join(", ", fields)} differ");
            }
            return differences;
        }

        private static SortedDictionary<string, JsonObject> Canonical(Document document)
        {
            var result = new SortedDictionary<string, JsonObject>(StringComparer.Ordinal);
            foreach (var character in document.Characters)
            {
                var node = JsonSerializer.SerializeToNode(character, JsonOptions).AsObject();
                node["experience"] ??= 0;
                node["level"] ??= 1;
                result["character " + character.Account] = node;
            }
            foreach (var (id, receipt) in document.RewardReceipts ?? new Dictionary<string, RewardReceipt>())
                result["receipt " + id] = JsonSerializer.SerializeToNode(receipt, JsonOptions).AsObject();
            foreach (var (key, row) in document.MatchRewardProgress ?? new Dictionary<string, MatchRewardProgress>())
            {
                var node = JsonSerializer.SerializeToNode(row, JsonOptions).AsObject();
                node.Remove("lastRewardAt");
                result["match " + key] = node;
            }
            return result;
        }
    }
}
