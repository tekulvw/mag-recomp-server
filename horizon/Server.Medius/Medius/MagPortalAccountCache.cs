using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Npgsql;

namespace Server.Medius
{
    /// <summary>Optional stable numeric portal identity lookup. Independent of Sony authentication.</summary>
    public sealed class MagPortalAccountLookupSettings
    {
        public string ConnectionFile { get; private set; }
        public static MagPortalAccountLookupSettings Load(Func<string, string> read, MagPortalTicketMode tickets)
        {
            string mode = read("MAG_PORTAL_ACCOUNT_LOOKUP");
            if (string.IsNullOrEmpty(mode) || mode == "off") return null;
            if (mode != "required") throw new ArgumentException("MAG_PORTAL_ACCOUNT_LOOKUP must be off or required.");
            if (tickets != MagPortalTicketMode.Required) throw new ArgumentException("MAG_PORTAL_ACCOUNT_LOOKUP=required requires MAG_PORTAL_TICKETS=required.");
            string file = read("MAG_PORTAL_ACCOUNT_DB_CONNECTION_FILE");
            if (string.IsNullOrWhiteSpace(file) || !Path.IsPathFullyQualified(file))
                throw new ArgumentException("MAG_PORTAL_ACCOUNT_DB_CONNECTION_FILE requires an absolute secret-file path.");
            return new MagPortalAccountLookupSettings { ConnectionFile = file };
        }
    }

    /// <summary>
    /// Bounded off-tick snapshot of numeric account IDs to immutable lowercase owners. Readers do
    /// no I/O or waiting. Complete snapshots are atomically published; stale or failed refreshes
    /// deny admission. Bans, deletion and unnamed accounts disappear on the next refresh.
    /// </summary>
    public sealed class MagPortalAccountCache : IDisposable, IAsyncDisposable
    {
        public const int MaximumRows = 100000;
        public static readonly TimeSpan MaximumAge = TimeSpan.FromSeconds(5);
        private sealed record Snapshot(IReadOnlyDictionary<uint, string> Owners, DateTime At);
        private readonly Func<CancellationToken, Task<IReadOnlyDictionary<uint, string>>> _load;
        private readonly Func<DateTime> _clock;
        private readonly CancellationTokenSource _stop = new CancellationTokenSource();
        private readonly SemaphoreSlim _refresh = new SemaphoreSlim(1, 1);
        private Snapshot _snapshot;
        private Task _worker;
        private bool _disposed;
        public MagPortalAccountCache(Func<CancellationToken, Task<IReadOnlyDictionary<uint, string>>> load, Func<DateTime> clock = null)
        { _load = load ?? throw new ArgumentNullException(nameof(load)); _clock = clock ?? (() => DateTime.UtcNow); }

        private static bool ValidOwner(string owner) => MagDevelopmentAccountPolicy.ValidAccount(owner) &&
            owner.Length <= 15 && owner[0] >= 'a' && owner[0] <= 'z';
        public bool TryResolve(uint accountId, out string owner)
        {
            owner = null;
            if (_disposed) return false;
            var snapshot = Volatile.Read(ref _snapshot);
            var age = snapshot == null ? MaximumAge : _clock() - snapshot.At;
            return snapshot != null && age >= TimeSpan.Zero && age < MaximumAge && snapshot.Owners.TryGetValue(accountId, out owner);
        }
        public async Task RefreshAsync(CancellationToken cancellation = default)
        {
            await _refresh.WaitAsync(cancellation);
            try
            {
                using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellation, _stop.Token);
                deadline.CancelAfter(TimeSpan.FromSeconds(3));
                var loaded = await _load(deadline.Token).WaitAsync(deadline.Token);
                if (loaded == null || loaded.Count > MaximumRows) throw new InvalidDataException();
                var owners = new Dictionary<uint, string>();
                var unique = new HashSet<string>(StringComparer.Ordinal);
                foreach (var pair in loaded)
                {
                    if (pair.Key == 0 || !ValidOwner(pair.Value) || !unique.Add(pair.Value)) throw new InvalidDataException();
                    owners.Add(pair.Key, pair.Value);
                }
                deadline.Token.ThrowIfCancellationRequested();
                Volatile.Write(ref _snapshot, new Snapshot(owners, _clock()));
            }
            catch
            {
                Volatile.Write(ref _snapshot, null);
                // Never attach an inner exception: connection strings, SQL or paths may be secret.
                throw new InvalidOperationException("Portal account snapshot refresh failed; admission denied.");
            }
            finally { _refresh.Release(); }
        }
        public async Task StartAsync()
        {
            if (_disposed || _worker != null) throw new InvalidOperationException("Portal account cache already started or disposed.");
            await RefreshAsync(_stop.Token); // Startup refuses a missing migration/owner column.
            _worker = Task.Run(async () =>
            {
                using var timer = new PeriodicTimer(TimeSpan.FromSeconds(1));
                try
                {
                    while (await timer.WaitForNextTickAsync(_stop.Token))
                        try { await RefreshAsync(_stop.Token); } catch (InvalidOperationException) { }
                }
                catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            });
        }
        public static MagPortalAccountCache FromConnectionFile(string path)
        {
            string connectionString;
            try
            {
                var info = new FileInfo(path);
                if (info.Length > 16384) throw new InvalidDataException();
                string secret = File.ReadAllText(path).Trim();
                var builder = new NpgsqlConnectionStringBuilder(secret) {
                    MaxPoolSize = 2, MinPoolSize = 0, Timeout = 3, CommandTimeout = 3,
                    IncludeErrorDetail = false, LogParameters = false, ApplicationName = "mag-portal-owner-cache"
                };
                if (string.IsNullOrWhiteSpace(builder.Host) || string.IsNullOrWhiteSpace(builder.Database)) throw new InvalidDataException();
                connectionString = builder.ConnectionString;
            }
            catch { throw new InvalidOperationException("MAG_PORTAL_ACCOUNT_DB_CONNECTION_FILE could not be loaded."); }
            return new MagPortalAccountCache(async cancellation =>
            {
                using var connection = new NpgsqlConnection(connectionString);
                await connection.OpenAsync(cancellation);
                using var command = new NpgsqlCommand("SELECT id, game_account_name FROM accounts WHERE player_name IS NOT NULL AND banned_at IS NULL LIMIT 100001", connection);
                using var reader = await command.ExecuteReaderAsync(cancellation);
                var owners = new Dictionary<uint, string>();
                while (await reader.ReadAsync(cancellation))
                {
                    long id = reader.GetInt64(0);
                    if (id <= 0 || id > uint.MaxValue || reader.IsDBNull(1) || owners.Count >= MaximumRows) throw new InvalidDataException();
                    owners.Add((uint)id, reader.GetString(1));
                }
                return owners;
            });
        }
        public async ValueTask DisposeAsync()
        {
            if (_disposed) return;
            _disposed = true; _stop.Cancel();
            if (_worker != null) await _worker;
            Volatile.Write(ref _snapshot, null);
            _stop.Dispose();
        }
        public void Dispose() => DisposeAsync().AsTask().GetAwaiter().GetResult();
    }
}
