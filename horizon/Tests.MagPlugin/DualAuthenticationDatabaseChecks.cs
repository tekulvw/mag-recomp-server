using System.Security.Cryptography;
using System.Text;
using Npgsql;
using Server.Medius;

internal static class DualAuthenticationDatabaseChecks
{
    static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    public static void Run()
    {
        string admin = Environment.GetEnvironmentVariable("MAG_TEST_POSTGRES");
        if (string.IsNullOrWhiteSpace(admin))
        {
            if (Environment.GetEnvironmentVariable("MAG_REQUIRE_POSTGRES_CHECKS") == "1") throw new Exception("MAG_TEST_POSTGRES required for dual authentication database checks");
            Console.WriteLine("NOT RUN dual authentication database checks: MAG_TEST_POSTGRES unset"); return;
        }
        void Sql(string cs, string sql)
        { using var c = new NpgsqlConnection(cs); c.Open(); using var cmd = new NpgsqlCommand(sql, c); cmd.ExecuteNonQuery(); }
        string database = "mag_dual_" + Guid.NewGuid().ToString("N");
        Sql(admin, "CREATE DATABASE " + database);
        string connection = new NpgsqlConnectionStringBuilder(admin) { Database = database }.ConnectionString;
        try
        {
            Sql(connection, "CREATE TABLE accounts(id integer PRIMARY KEY,kind text NOT NULL,provider text NOT NULL,subject text NOT NULL,player_name text,banned_at timestamptz);" +
                File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "console-link-schema.sql")));
            Sql(connection, "INSERT INTO accounts VALUES(7,'person','google','synthetic-google','PortalOwner',NULL)");
            using var store = new PostgresMagExternalIdentityStore(connection);
            store.CheckSchema();
            Check(store.ResolvePortal(7, "PortalOwner")?.Owner == "portalowner", "Portal id resolves canonical name");
            Check(store.ResolvePortal(8, "PortalOwner") == null && store.ResolvePortal(7, "SomeoneElse") == null, "No signed id/name confusion");
            Check(store.ResolvePsn("256:42") == null, "Unlinked Sony subject must not resolve by name");
            long now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
            string code = store.IssueProof("256:42", "ConsoleUser", now, now + 1000000);
            using (var c = new NpgsqlConnection(connection))
            {
                c.Open(); using var cmd = new NpgsqlCommand("SELECT code_hash,online_id,expires_at FROM console_link_proofs WHERE subject='256:42'", c);
                using var r = cmd.ExecuteReader(); Check(r.Read(), "Proof not stored");
                Check(r.GetFieldValue<byte[]>(0).SequenceEqual(SHA256.HashData(Encoding.ASCII.GetBytes(code))), "C#/portal lowercase-code SHA256 contract");
                Check(r.GetString(1) == "ConsoleUser" && r.GetDateTime(2) == DateTimeOffset.FromUnixTimeMilliseconds(now+300000).UtcDateTime, "Proof identity/expiry contract");
            }
            string rotated = store.IssueProof("256:42", "ConsoleUser", now, now + 40000);
            Check(rotated != code, "Proof rotation must invalidate old code");
            using (var redemption = new NpgsqlConnection(connection))
            {
                redemption.Open(); using var tx = redemption.BeginTransaction();
                using (var cmd = new NpgsqlCommand("DELETE FROM console_link_proofs WHERE subject='256:42'; INSERT INTO external_identities(provider,subject,account_id,online_id) VALUES('psn','256:42',7,'ConsoleUser')", redemption, tx)) cmd.ExecuteNonQuery();
                var issuing = Task.Run(() => store.IssueProof("256:42", "ConsoleUser", now, now+1000000));
                bool blocked = false;
                var deadline = DateTime.UtcNow.AddSeconds(2);
                using var monitor = new NpgsqlConnection(connection); monitor.Open();
                while (DateTime.UtcNow < deadline && !blocked)
                {
                    using var inspect = new NpgsqlCommand("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'DELETE FROM console_link_proofs%')", monitor);
                    blocked = (bool)inspect.ExecuteScalar();
                    if (!blocked) Thread.Sleep(10);
                }
                Check(blocked, "Concurrent proof issuer did not reach the existing proof row lock");
                tx.Commit();
                Check(issuing.GetAwaiter().GetResult() == null, "Proof issuance must recheck mapping after concurrent redemption commits");
            }
            Check(store.ResolvePsn("256:42") == new MagExternalAccount(7, "portalowner"), "Explicit differing-name mapping");
            Check(store.IssueProof("256:42", "ConsoleUser", now, now + 1000000) == null, "Linked/banned identities must not be offered fresh linking proofs");
            Sql(connection, "UPDATE accounts SET banned_at=now() WHERE id=7");
            Check(store.ResolvePortal(7, "PortalOwner") == null && store.ResolvePsn("256:42") == null, "Current ban applies to both providers");
            Sql(connection, "UPDATE accounts SET banned_at=NULL,kind='service' WHERE id=7");
            Check(store.ResolvePsn("256:42") == null, "Service account console access");
            Check(store.ResolvePortal(7, "PortalOwner")?.Owner == "portalowner", "Existing signed portal service account must remain admitted in dual mode");
            Sql(connection, "DELETE FROM accounts WHERE id=7");
            Check(store.ResolvePsn("256:42") == null, "Deleted linked account access");
        }
        finally { NpgsqlConnection.ClearAllPools(); Sql(admin, "DROP DATABASE IF EXISTS " + database + " WITH (FORCE)"); }
    }
}
