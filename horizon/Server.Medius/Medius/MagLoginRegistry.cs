using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using RT.Models.MAG;

namespace Server.Medius
{
    public sealed class MagLoginTicket
    {
        public int ApplicationId { get; internal set; }
        public uint AccountId { get; internal set; }
        public string AuthenticatedAccount { get; internal set; }
        public uint CharacterId { get; internal set; }
        public uint WorldId { get; internal set; }
        public string AccessToken { get; internal set; }
        public string SessionKey { get; internal set; }
        internal string ChannelId;
        internal DateTime Deadline;
        public MagAccountLoginResponse CreateResponse(int transactionId)
        {
            MagAccountLoginResponse response = ApplicationId == 20231
                ? new MagRetailAccountLoginResponse() : new MagAccountLoginResponse();
            response.TransactionId = transactionId; response.LobbyWorldUid = 1; response.AccountId = AccountId;
            var network = MagNetworkSettings.FromEnvironment();
            response.ConnectionType = 1; response.AddressType = 1; response.ServerAddress = FixedAscii(network.Address.ToString(), 16);
            response.ServerPort = network.LobbyPort; response.WorldId = WorldId;
            response.AccessKey = FixedAscii(AccessToken, 17); response.SessionKey = FixedAscii(SessionKey, 17); response.Result = 1;
            return response;
        }
        private static byte[] FixedAscii(string value, int capacity)
        {
            var result = new byte[capacity]; Encoding.ASCII.GetBytes(value).CopyTo(result, 0); return result;
        }
    }
    /// <summary>Process-local MAG credentials; pending tickets live 60s, bound channels have 120s idle leases.</summary>
    public sealed class MagLoginRegistry
    {
        private readonly object _gate = new object();
        private readonly Dictionary<string, MagLoginTicket> _tickets = new Dictionary<string, MagLoginTicket>(StringComparer.Ordinal);
        private readonly int _capacity;
        private readonly object _markerGate = new object();
        private string _markerPath;
        private DateTime _markerWritten;
        public MagLoginRegistry(int capacity = 128)
        { if (capacity < 1 || capacity > 1024) throw new ArgumentOutOfRangeException(nameof(capacity)); _capacity = capacity; }
        private void Purge(DateTime now)
        { foreach (var key in _tickets.Where(kv => kv.Value.Deadline <= now).Select(kv => kv.Key).ToArray()) _tickets.Remove(key); }
        private static string NewKey() => Convert.ToBase64String(RandomNumberGenerator.GetBytes(12)); // 16 ASCII chars, 96 random bits.
        // Compatibility tickets bind transports; mutation and gameplay require account provenance.
        public MagLoginTicket Issue(uint accountId, uint worldId, DateTime now) => IssueCore(null, accountId, worldId, now);
        public MagLoginTicket IssueForAccount(string account, uint characterId, uint worldId, DateTime now) =>
            MagDevelopmentAccountPolicy.ValidAccount(account) ? IssueCore(account, characterId, worldId, now) : null;
        private MagLoginTicket IssueCore(string account, uint accountId, uint worldId, DateTime now)
        {
            if (accountId == 0 || worldId != 1) return null;
            MagLoginTicket ticket;
            lock (_gate)
            {
                Purge(now); if (_tickets.Count >= _capacity) return null;
                string token; do { token = NewKey(); } while (_tickets.ContainsKey(token));
                ticket = new MagLoginTicket { ApplicationId = Program.Settings.MagApplicationId, AccountId = accountId, AuthenticatedAccount = account, CharacterId = accountId, WorldId = worldId, AccessToken = token,
                    SessionKey = NewKey(), Deadline = now.AddSeconds(60) };
                _tickets.Add(token, ticket);
            }
            if (account != null) PublishMarker(now, true);
            return ticket;
        }
        public bool TryBind(string accessToken, string sessionKey, int appId, uint worldId, string channelId, DateTime now, out MagLoginTicket ticket)
        {
            ticket = null;
            if (!MagPluginRouting.IsKnownMagApplication(appId) || worldId != 1 || accessToken?.Length != 16 || sessionKey?.Length != 16 || string.IsNullOrEmpty(channelId)) return false;
            lock (_gate)
            {
                Purge(now);
                if (!_tickets.TryGetValue(accessToken, out var candidate) || candidate.ChannelId != null || candidate.ApplicationId != appId || candidate.WorldId != worldId ||
                    !string.Equals(candidate.SessionKey, sessionKey, StringComparison.Ordinal) || _tickets.Values.Any(t => t.ChannelId == channelId)) return false;
                candidate.ChannelId = channelId; candidate.Deadline = now.AddSeconds(120); ticket = candidate;
            }
            PublishMarker(now, true);
            return true;
        }
        public bool Touch(string channelId, DateTime now)
        {
            lock (_gate)
            {
                Purge(now); var ticket = _tickets.Values.FirstOrDefault(t => t.ChannelId == channelId);
                if (ticket == null) return false; ticket.Deadline = now.AddSeconds(120);
            }
            PublishMarker(now, false);
            return true;
        }
        public void Release(string channelId)
        {
            lock (_gate) foreach (var key in _tickets.Where(kv => kv.Value.ChannelId == channelId).Select(kv => kv.Key).ToArray()) _tickets.Remove(key);
            PublishMarker(DateTime.UtcNow, true);
        }
        /// <summary>Publish account-bound tickets (pending or bound) with their lease deadlines for operator tools.</summary>
        public void PublishActiveAccountsTo(string path, DateTime now)
        {
            if (string.IsNullOrWhiteSpace(path) || !Path.IsPathFullyQualified(path)) throw new ArgumentException("Absolute marker path required.", nameof(path));
            lock (_markerGate) _markerPath = path;
            PublishMarker(now, true);
        }
        public IReadOnlyDictionary<string, DateTime> ActiveAccounts(DateTime now)
        {
            lock (_gate)
            {
                Purge(now);
                return _tickets.Values.Where(t => t.AuthenticatedAccount != null)
                    .GroupBy(t => t.AuthenticatedAccount, StringComparer.Ordinal)
                    .ToDictionary(g => g.Key, g => g.Max(t => t.Deadline), StringComparer.Ordinal);
            }
        }
        // Keepalives extend leases every few seconds; rewrite at most every 30 s for those, since a
        // reader only needs each lease to stay ahead of the published deadline.
        private void PublishMarker(DateTime now, bool force)
        {
            lock (_markerGate)
            {
                if (_markerPath == null || (!force && now - _markerWritten < TimeSpan.FromSeconds(30))) return;
                try { MagActiveAccounts.Write(_markerPath, ActiveAccounts(now), now); _markerWritten = now; }
                catch (IOException) { } catch (UnauthorizedAccessException) { }
            }
        }
    }
    /// <summary>Process-local session markers shared with the mag-admin CLI inside the same Pod.</summary>
    public static class MagActiveAccounts
    {
        public const string FilePrefix = "mag-active-accounts.";
        public static void Write(string path, IReadOnlyDictionary<string, DateTime> accounts, DateTime now)
        {
            var body = System.Text.Json.JsonSerializer.SerializeToUtf8Bytes(new {
                version = 1, written = now.ToUniversalTime().ToString("O"),
                accounts = accounts.OrderBy(a => a.Key, StringComparer.Ordinal)
                    .Select(a => new { account = a.Key, until = a.Value.ToUniversalTime().ToString("O") }) });
            string temporary = path + ".tmp-" + Environment.ProcessId;
            File.WriteAllBytes(temporary, body);
            File.Move(temporary, path, true);
        }
        /// <summary>Accounts whose lease is still live in any marker in <paramref name="directory"/>; null when no marker exists.</summary>
        public static IReadOnlyDictionary<string, DateTime> Read(string directory, DateTime now)
        {
            if (!Directory.Exists(directory)) return null;
            var files = Directory.GetFiles(directory, FilePrefix + "*.json");
            if (files.Length == 0) return null;
            var active = new Dictionary<string, DateTime>(StringComparer.Ordinal);
            foreach (var file in files)
            {
                using var document = System.Text.Json.JsonDocument.Parse(File.ReadAllBytes(file));
                if (document.RootElement.GetProperty("version").GetInt32() != 1) throw new InvalidDataException("Unsupported session marker: " + file);
                foreach (var entry in document.RootElement.GetProperty("accounts").EnumerateArray())
                {
                    string account = entry.GetProperty("account").GetString();
                    var until = DateTime.Parse(entry.GetProperty("until").GetString(), null, System.Globalization.DateTimeStyles.RoundtripKind).ToUniversalTime();
                    if (until > now && (!active.TryGetValue(account, out var known) || until > known)) active[account] = until;
                }
            }
            return active;
        }
    }
}
