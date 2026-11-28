using System;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using Npgsql;

namespace Server.Medius
{
    /// <summary>A current portal person and the immutable owner key of their existing characters.</summary>
    public sealed record MagExternalAccount(uint AccountId, string Owner);

    public interface IMagExternalIdentityStore
    {
        MagExternalAccount ResolvePortal(uint accountId, string signedName);
        MagExternalAccount ResolvePsn(string subject);
        string IssueProof(string subject, string onlineId, long nowMilliseconds, long ticketExpiryMilliseconds);
    }

    /// <summary>
    /// Reads the portal's current accounts and links. Only the portal redeems/unlinks identities.
    /// Requires portal migration 0009; never migrates or creates an account on game login.
    /// </summary>
    public sealed class PostgresMagExternalIdentityStore : IMagExternalIdentityStore, IDisposable
    {
        public const int MaximumPendingProofs = 10000;
        public const int ProofLifetimeSeconds = 300;
        private readonly NpgsqlDataSource _source;

        public PostgresMagExternalIdentityStore(string connectionString)
        {
            var builder = new NpgsqlConnectionStringBuilder(connectionString)
            { MaxPoolSize = 4, Timeout = 3, CommandTimeout = 3, IncludeErrorDetail = false };
            _source = NpgsqlDataSource.Create(builder.ConnectionString);
        }

        public void CheckSchema()
        {
            using var connection = _source.OpenConnection();
            using var command = new NpgsqlCommand("SELECT a.id, a.kind, a.banned_at, a.player_name, e.subject, p.code_hash " +
                "FROM accounts a LEFT JOIN external_identities e ON e.account_id=a.id LEFT JOIN console_link_proofs p ON false LIMIT 0", connection);
            command.ExecuteNonQuery();
        }

        public MagExternalAccount ResolvePortal(uint accountId, string signedName)
        {
            if (accountId == 0 || accountId > int.MaxValue) return null;
            using var connection = _source.OpenConnection();
            using var command = new NpgsqlCommand("SELECT id, lower(player_name) FROM accounts " +
                "WHERE id=@id AND banned_at IS NULL AND lower(player_name)=@name", connection);
            command.Parameters.AddWithValue("id", (int)accountId);
            command.Parameters.AddWithValue("name", signedName.ToLowerInvariant());
            return ReadAccount(command);
        }

        public MagExternalAccount ResolvePsn(string subject)
        {
            using var connection = _source.OpenConnection();
            using var command = new NpgsqlCommand("SELECT a.id, lower(a.player_name) FROM external_identities e " +
                "JOIN accounts a ON a.id=e.account_id WHERE e.provider='psn' AND e.subject=@subject " +
                "AND a.kind='person' AND a.banned_at IS NULL AND a.player_name IS NOT NULL", connection);
            command.Parameters.AddWithValue("subject", subject);
            return ReadAccount(command);
        }

        private static MagExternalAccount ReadAccount(NpgsqlCommand command)
        {
            using var reader = command.ExecuteReader();
            if (!reader.Read()) return null;
            int id = reader.GetInt32(0);
            string owner = reader.GetString(1);
            return id > 0 && MagDevelopmentAccountPolicy.ValidAccount(owner) ? new MagExternalAccount((uint)id, owner) : null;
        }

        public string IssueProof(string subject, string onlineId, long nowMilliseconds, long ticketExpiryMilliseconds)
        {
            if (ticketExpiryMilliseconds <= nowMilliseconds) return null;
            string code = Convert.ToHexString(RandomNumberGenerator.GetBytes(16)).ToLowerInvariant();
            byte[] hash = SHA256.HashData(Encoding.ASCII.GetBytes(code));
            using var connection = _source.OpenConnection();
            using var transaction = connection.BeginTransaction();
            // A short shared lock keeps the global bound true across MAPS/MAS processes.
            using (var gate = new NpgsqlCommand("SELECT pg_advisory_xact_lock(5570746319637416267)", connection, transaction)) gate.ExecuteNonQuery();
            using (var purge = new NpgsqlCommand("DELETE FROM console_link_proofs WHERE expires_at<=@now OR (provider='psn' AND subject=@subject)", connection, transaction))
            {
                purge.Parameters.AddWithValue("now", DateTimeOffset.FromUnixTimeMilliseconds(nowMilliseconds).UtcDateTime);
                purge.Parameters.AddWithValue("subject", subject);
                purge.ExecuteNonQuery();
            }
            // Check after the potentially blocking proof deletion: concurrent redemption may have
            // committed the mapping while that row was locked. READ COMMITTED refreshes this statement.
            using (var linked = new NpgsqlCommand("SELECT EXISTS(SELECT 1 FROM external_identities WHERE provider='psn' AND subject=@subject)", connection, transaction))
            {
                linked.Parameters.AddWithValue("subject", subject);
                if ((bool)linked.ExecuteScalar()) return null;
            }
            using (var count = new NpgsqlCommand("SELECT count(*) FROM console_link_proofs", connection, transaction))
                if ((long)count.ExecuteScalar() >= MaximumPendingProofs) return null;
            using (var insert = new NpgsqlCommand("INSERT INTO console_link_proofs(code_hash,provider,subject,online_id,expires_at) " +
                "VALUES(@hash,'psn',@subject,@online,@expiry)", connection, transaction))
            {
                insert.Parameters.AddWithValue("hash", hash);
                insert.Parameters.AddWithValue("subject", subject);
                insert.Parameters.AddWithValue("online", onlineId);
                insert.Parameters.AddWithValue("expiry", DateTimeOffset.FromUnixTimeMilliseconds(Math.Min(nowMilliseconds + ProofLifetimeSeconds * 1000L, ticketExpiryMilliseconds)).UtcDateTime);
                insert.ExecuteNonQuery();
            }
            transaction.Commit();
            return code;
        }

        public void Dispose() => _source.Dispose();
    }
}
