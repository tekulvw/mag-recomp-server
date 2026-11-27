using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using Npgsql;
using NpgsqlTypes;

namespace Server.Medius
{
    /// <summary>A server-side voice sanction. The relay treats both kinds alike: it drops every voice
    /// packet the account sends, its channel copy and its proximity copy. The kind records the operator's
    /// intent: a mute is a short cool-off, a ban a standing refusal. Either may expire or last until lifted.</summary>
    public enum VoiceSanctionKind { Mute, Ban }

    /// <summary>One account's voice sanction as the store holds it. Times are whole seconds.</summary>
    public sealed record VoiceSanction(string Account, VoiceSanctionKind Kind, string Reason, string Operator,
        DateTimeOffset CreatedAt, DateTimeOffset? ExpiresAt)
    {
        /// <summary>The sanction applies at <paramref name="now"/>: it has no end, or ends later.</summary>
        public bool ActiveAt(DateTimeOffset now) => ExpiresAt == null || ExpiresAt > now;

        public static string Name(VoiceSanctionKind kind) => kind == VoiceSanctionKind.Ban ? "ban" : "mute";

        public static bool TryParseKind(string value, out VoiceSanctionKind kind)
        {
            kind = value == "ban" ? VoiceSanctionKind.Ban : VoiceSanctionKind.Mute;
            return value == "mute" || value == "ban";
        }
    }

    /// <summary>A planned (dry run), applied or refused voice sanction change.</summary>
    public sealed class VoiceSanctionChange
    {
        public string Account { get; init; }
        public VoiceSanctionKind Kind { get; init; }
        /// <summary>True for unmute and unban.</summary>
        public bool Lift { get; init; }
        public bool Applied { get; init; }
        /// <summary>Null when planned or applied; otherwise a refusal word (<c>account-not-found</c>,
        /// <c>not-sanctioned</c>, <c>invalid-reason</c>, <c>invalid-operator</c>, <c>invalid-duration</c>,
        /// <c>backup-directory-required</c>).</summary>
        public string Refusal { get; init; }
        /// <summary>The account's active sanction of this kind before the change, or null.</summary>
        public VoiceSanction Before { get; init; }
        /// <summary>The sanction of this kind after the change: the new one, or null for a lift.</summary>
        public VoiceSanction After { get; init; }
        /// <summary>Where the applied change was journalled: <c>mag.admin_changes</c> or a backup file.</summary>
        public string Journal { get; init; }
        /// <summary>The journal operation: <c>voice-mute</c>, <c>voice-ban</c>, <c>voice-unmute</c> or <c>voice-unban</c>.</summary>
        public string Operation => "voice-" + (Lift ? "un" : "") + VoiceSanction.Name(Kind);
    }

    public sealed partial class LocalCharacterStore
    {
        /// <summary>The longest sanction with an end; a longer one is imposed without <c>--for</c>.</summary>
        public static readonly TimeSpan MaximumVoiceSanctionDuration = TimeSpan.FromDays(3650);
        public static readonly TimeSpan MinimumVoiceSanctionDuration = TimeSpan.FromMinutes(1);

        private sealed class StoredVoiceSanction
        {
            [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
            [JsonRequired] public string Account { get; set; }
            [JsonRequired] public string Kind { get; set; }
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public string Reason { get; set; }
            [JsonRequired] public string Operator { get; set; }
            /// <summary>Unix seconds.</summary>
            [JsonRequired] public long CreatedAt { get; set; }
            /// <summary>Unix seconds; null lasts until lifted.</summary>
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public long? ExpiresAt { get; set; }

            public VoiceSanction ToSanction()
            {
                VoiceSanction.TryParseKind(Kind, out var kind);
                return new VoiceSanction(Account, kind, Reason, Operator, DateTimeOffset.FromUnixTimeSeconds(CreatedAt),
                    ExpiresAt is long end ? DateTimeOffset.FromUnixTimeSeconds(end) : null);
            }
        }

        /// <summary>Printable ASCII, at most 256 characters (the operator kill's reason rule).</summary>
        internal static bool ValidVoiceSanctionReason(string reason) => MagAdminKill.ValidReason(reason);

        /// <summary>1 to 254 printable ASCII characters (the admin API's actor rule).</summary>
        internal static bool ValidVoiceSanctionOperator(string name) =>
            name != null && name.Length >= 1 && name.Length <= MagAdminKill.MaximumOperatorLength && name.All(c => c >= 0x20 && c <= 0x7e);

        private static void ValidateVoiceSanctions(Document document)
        {
            if (document.VoiceSanctions == null) return;
            // A sanction belongs to the account: it outlives a purged character, even the account's last one.
            var seen = new HashSet<string>(StringComparer.Ordinal);
            foreach (var s in document.VoiceSanctions)
                if (s == null || !MagDevelopmentAccountPolicy.ValidAccount(s.Account) || !VoiceSanction.TryParseKind(s.Kind, out _) ||
                    !ValidVoiceSanctionReason(s.Reason) || !ValidVoiceSanctionOperator(s.Operator) || s.CreatedAt < 0 ||
                    (s.ExpiresAt is long end && end <= s.CreatedAt) || !seen.Add(s.Account + "\n" + s.Kind))
                    throw new InvalidDataException("Character-store voice sanctions invalid.");
        }

        /// <summary>Every stored voice sanction, expired ones included, ordered by account and kind.</summary>
        public IReadOnlyList<VoiceSanction> ReadVoiceSanctions()
        {
            IEnumerable<StoredVoiceSanction> rows;
            if (_backend is PostgresBackend pg)
            {
                try
                {
                    using var connection = pg.Database.OpenConnection();
                    using var command = new NpgsqlCommand("SELECT " + VoiceSanctionColumns + " FROM mag.voice_sanctions ORDER BY account, kind", connection);
                    rows = ReadVoiceSanctionRows(command);
                }
                catch (Exception e) { throw pg.Database.Translate(e); }
            }
            else rows = ReadDocument().VoiceSanctions ?? new List<StoredVoiceSanction>();
            return rows.Select(s => s.ToSanction()).OrderBy(s => s.Account, StringComparer.Ordinal).ThenBy(s => s.Kind).ToArray();
        }

        /// <summary>
        /// A value that changes whenever the JSON store file may have changed (its write time and length), so a
        /// poller can skip rereading an unchanged file; null for PostgreSQL, which is always reread.
        /// </summary>
        internal string VoiceSanctionsChangeToken()
        {
            if (_backend is PostgresBackend) return null;
            var file = new FileInfo(ValidPath());
            return file.Exists ? $"{file.LastWriteTimeUtc.Ticks}:{file.Length}" : "absent";
        }

        /// <summary>
        /// Operator voice moderation: impose (<paramref name="lift"/> false) or lift one kind of sanction on an
        /// account that has a character. Imposing replaces any sanction of that kind; <paramref name="duration"/>
        /// null lasts until lifted. Lifting needs an active sanction of that kind. Without
        /// <paramref name="apply"/> nothing is written. An applied change is journalled with its operator and
        /// reason: in mag.admin_changes (PostgreSQL), or by copying the store file into
        /// <paramref name="backupDirectory"/> first (JSON).
        /// </summary>
        public VoiceSanctionChange ChangeVoiceSanction(string account, VoiceSanctionKind kind, bool lift, string reason, string operatorName,
            TimeSpan? duration, bool apply, string backupDirectory)
        {
            VoiceSanctionChange Refused(string why, VoiceSanction before = null) =>
                new VoiceSanctionChange { Account = account, Kind = kind, Lift = lift, Refusal = why, Before = before };
            if (string.IsNullOrWhiteSpace(reason)) reason = null;
            if (!MagDevelopmentAccountPolicy.ValidAccount(account)) return Refused("account-not-found");
            if (!ValidVoiceSanctionReason(reason)) return Refused("invalid-reason");
            if (!ValidVoiceSanctionOperator(operatorName)) return Refused("invalid-operator");
            if (duration != null && (lift || duration < MinimumVoiceSanctionDuration || duration > MaximumVoiceSanctionDuration))
                return Refused("invalid-duration");
            using var session = _backend.Open(new StoreScope(account, CreateDirectory: false, VoiceSanctions: true));
            var document = session.Document;
            // An account is known while it has a character (deleted ones too) or a sanction to lift.
            if (!document.Characters.Any(c => c.Account == account) && document.VoiceSanctions?.Any(s => s.Account == account) != true)
                return Refused("account-not-found");
            string kindName = VoiceSanction.Name(kind);
            var all = document.VoiceSanctions ?? new List<StoredVoiceSanction>();
            var existing = all.SingleOrDefault(s => s.Account == account && s.Kind == kindName);
            DateTimeOffset now = DateTimeOffset.FromUnixTimeSeconds(Clock().ToUnixTimeSeconds());
            var before = existing?.ToSanction() is { } current && current.ActiveAt(now) ? current : null;
            if (lift && before == null) return Refused("not-sanctioned");
            var stored = lift ? null : new StoredVoiceSanction
            {
                Account = account, Kind = kindName, Reason = reason, Operator = operatorName, CreatedAt = now.ToUnixTimeSeconds(),
                ExpiresAt = duration is TimeSpan length ? now.Add(length).ToUnixTimeSeconds() : null
            };
            var change = new VoiceSanctionChange { Account = account, Kind = kind, Lift = lift, Before = before, After = stored?.ToSanction() };
            if (!apply) return change;
            if (_backend is JsonBackend && (string.IsNullOrWhiteSpace(backupDirectory) || !Path.IsPathFullyQualified(backupDirectory)))
                return Refused("backup-directory-required", before);
            if (existing != null) all.Remove(existing);
            if (stored != null) all.Add(stored);
            document.VoiceSanctions = all.Count == 0 ? null : all
                .OrderBy(s => s.Account, StringComparer.Ordinal).ThenBy(s => s.Kind, StringComparer.Ordinal).ToList();
            string beforeJson = JsonSerializer.Serialize(new { sanction = existing }, JsonOptions);
            string afterJson = JsonSerializer.Serialize(new { sanction = stored, @operator = operatorName, reason }, JsonOptions);
            string journal = session.CommitVoiceSanctions(account, change.Operation, beforeJson, afterJson, backupDirectory);
            return new VoiceSanctionChange { Account = account, Kind = kind, Lift = lift, Applied = true, Before = before, After = change.After,
                Journal = journal };
        }

        // ---- PostgreSQL rows (mag.voice_sanctions, migration 0002) ----

        private const string VoiceSanctionColumns = "account, kind, reason, operator, created_at, expires_at";

        private static List<StoredVoiceSanction> ReadVoiceSanctionRows(NpgsqlCommand command)
        {
            static long Seconds(DateTime value) => new DateTimeOffset(DateTime.SpecifyKind(value, DateTimeKind.Utc)).ToUnixTimeSeconds();
            var list = new List<StoredVoiceSanction>();
            using var r = command.ExecuteReader();
            while (r.Read())
                list.Add(new StoredVoiceSanction
                {
                    Account = r.GetString(0), Kind = r.GetString(1), Reason = r.IsDBNull(2) ? null : r.GetString(2), Operator = r.GetString(3),
                    CreatedAt = Seconds(r.GetDateTime(4)), ExpiresAt = r.IsDBNull(5) ? null : Seconds(r.GetDateTime(5))
                });
            return list;
        }

        private static List<StoredVoiceSanction> LoadVoiceSanctionRows(NpgsqlConnection connection, NpgsqlTransaction transaction, string account)
        {
            using var command = new NpgsqlCommand("SELECT " + VoiceSanctionColumns + " FROM mag.voice_sanctions" +
                (account == null ? " ORDER BY account, kind" : " WHERE account = @account ORDER BY kind FOR UPDATE"), connection, transaction)
                { CommandTimeout = PostgresCharacterDatabase.MaintenanceCommandTimeoutSeconds };
            if (account != null) command.Parameters.Add(Text("account", account));
            return ReadVoiceSanctionRows(command);
        }

        private static void InsertVoiceSanction(NpgsqlConnection connection, NpgsqlTransaction transaction, StoredVoiceSanction s)
        {
            using var insert = new NpgsqlCommand("INSERT INTO mag.voice_sanctions (" + VoiceSanctionColumns + ") " +
                "VALUES (@account, @kind, @reason, @operator, @created, @expires)", connection, transaction);
            insert.Parameters.Add(Text("account", s.Account));
            insert.Parameters.Add(Text("kind", s.Kind));
            insert.Parameters.Add(Text("reason", s.Reason));
            insert.Parameters.Add(Text("operator", s.Operator));
            insert.Parameters.Add(Typed("created", NpgsqlDbType.TimestampTz, DateTimeOffset.FromUnixTimeSeconds(s.CreatedAt).UtcDateTime));
            insert.Parameters.Add(Typed("expires", NpgsqlDbType.TimestampTz,
                s.ExpiresAt is long end ? DateTimeOffset.FromUnixTimeSeconds(end).UtcDateTime : null));
            insert.ExecuteNonQuery();
        }

        /// <summary>Replaces <paramref name="account"/>'s rows with <paramref name="rows"/> and journals the change.</summary>
        private static void WriteVoiceSanctions(NpgsqlConnection connection, NpgsqlTransaction transaction, string account,
            IEnumerable<StoredVoiceSanction> rows, string operation, string before, string after)
        {
            using (var clear = new NpgsqlCommand("DELETE FROM mag.voice_sanctions WHERE account = @account", connection, transaction))
            {
                clear.Parameters.Add(Text("account", account));
                clear.ExecuteNonQuery();
            }
            foreach (var row in rows.Where(s => s.Account == account)) InsertVoiceSanction(connection, transaction, row);
            using var audit = new NpgsqlCommand("INSERT INTO mag.admin_changes (operation, account, before, after) " +
                "VALUES (@operation, @account, @before, @after)", connection, transaction);
            audit.Parameters.Add(Text("operation", operation));
            audit.Parameters.Add(Text("account", account));
            audit.Parameters.Add(Typed("before", NpgsqlDbType.Jsonb, before));
            audit.Parameters.Add(Typed("after", NpgsqlDbType.Jsonb, after));
            audit.ExecuteNonQuery();
        }
    }
}
