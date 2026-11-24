using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Text.Json;

namespace Server.Medius
{
    public sealed record MagConfiguredWorker(string WorkerId, string WorkerHost, int WorkerPort,
        int MinimumPlayers, int MaximumPlayers, IReadOnlyList<int> MinimumPlayersPerTeam, bool Draining);

    /// <summary>Explicit local DME worlds paired with separate, fenced Rust workers.</summary>
    public sealed class MagWorldRegistryConfig
    {
        public int ApplicationId { get; }
        public IPAddress ClientAddress { get; }
        public ushort ClientTcpPort { get; }
        public IReadOnlyList<MagConfiguredWorker> Workers { get; }
        public int? PartyInviteSeconds { get; }
        /// <summary>Restart adoption (see docs/configured-world-registry.md). Defaults to
        /// <see cref="MagWorkerAdoptionPolicy.Default"/> when <c>worker_restart_adoption</c> is absent.</summary>
        public MagWorkerAdoptionPolicy Adoption { get; }
        MagWorldRegistryConfig(int application, IPAddress address, ushort port, MagConfiguredWorker[] workers, int? partyInviteSeconds,
            MagWorkerAdoptionPolicy adoption)
        {
            ApplicationId = application; ClientAddress = address; ClientTcpPort = port; Workers = Array.AsReadOnly(workers);
            PartyInviteSeconds = partyInviteSeconds; Adoption = adoption;
        }

        /// <summary>A <c>client_address</c> of exactly this text takes the address from MAG_ADVERTISE_IPV4,
        /// so a deployment that keeps that variable current needs no second copy in the file.</summary>
        public const string AdvertisedAddressPlaceholder = "$MAG_ADVERTISE_IPV4";

        public static MagWorldRegistryConfig Load(string path) => Load(path, Environment.GetEnvironmentVariable);
        public static MagWorldRegistryConfig Load(string path, Func<string, string> environment)
        {
            if (!Path.IsPathFullyQualified(path)) throw Invalid();
            var info = new FileInfo(path);
            if (!info.Exists || info.Length > 65536) throw Invalid();
            return Parse(File.ReadAllText(path), environment);
        }
        public static MagWorldRegistryConfig Parse(string json) => Parse(json, Environment.GetEnvironmentVariable);
        public static MagWorldRegistryConfig Parse(string json, Func<string, string> environment)
        {
            if (environment == null) throw new ArgumentNullException(nameof(environment));
            if (json == null || System.Text.Encoding.UTF8.GetByteCount(json) > 65536) throw Invalid();
            try
            {
                using var doc = JsonDocument.Parse(json);
                var root = doc.RootElement;
                bool parties = root.ValueKind == JsonValueKind.Object && root.TryGetProperty("party_invite_seconds", out _);
                bool adoptionSet = root.ValueKind == JsonValueKind.Object && root.TryGetProperty("worker_restart_adoption", out _);
                var fields = new List<string> { "format", "application_id", "client_address", "client_tcp_port", "workers" };
                if (parties) fields.Add("party_invite_seconds");
                if (adoptionSet) fields.Add("worker_restart_adoption");
                Exact(root, fields.ToArray());
                int? inviteSeconds = parties ? Integer(root, "party_invite_seconds", 1, 300) : null;
                var adoption = adoptionSet ? ParseAdoption(root.GetProperty("worker_restart_adoption")) : MagWorkerAdoptionPolicy.Default;
                if (root.GetProperty("format").GetString() != "mag_world_registry_v1" ||
                    Integer(root, "application_id", 20231, 20231) != 20231) throw Invalid();
                string address = Text(root, "client_address");
                if (address == AdvertisedAddressPlaceholder) address = environment("MAG_ADVERTISE_IPV4") ?? throw Invalid();
                int port = Integer(root, "client_tcp_port", 1, 65535);
                var network = MagNetworkSettings.Load(key => key switch {
                    "MAG_ADVERTISE_IPV4" => address, "MAG_GAME_TCP_PORT" => port.ToString(System.Globalization.CultureInfo.InvariantCulture), _ => null
                });
                var array = root.GetProperty("workers");
                if (array.ValueKind != JsonValueKind.Array || array.GetArrayLength() < 1 || array.GetArrayLength() > 16) throw Invalid();
                var ids = new HashSet<string>(StringComparer.Ordinal);
                var endpoints = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                var workers = new List<MagConfiguredWorker>();
                foreach (var row in array.EnumerateArray())
                {
                    Exact(row, "worker_id", "worker_host", "worker_port", "minimum_players", "maximum_players", "minimum_players_per_team", "draining");
                    string id = Text(row, "worker_id"), host = Text(row, "worker_host");
                    if (!GameplayWorkerIdentity.IsValidIdentifier(id) || !ids.Add(id) || host.Length > 253 ||
                        host != host.Trim() || Uri.CheckHostName(host) == UriHostNameType.Unknown) throw Invalid();
                    host = host.TrimEnd('.').ToLowerInvariant();
                    int workerPort = Integer(row, "worker_port", 1, 65535);
                    if (!endpoints.Add(host + ":" + workerPort)) throw Invalid();
                    // One human may start a worker whose profiles advertise bot fill; workers
                    // without it still form only with a human on each side (see MagFactionMatchmaking).
                    int minimum = Integer(row, "minimum_players", 1, 64), maximum = Integer(row, "maximum_players", minimum, 64);
                    var teamRows = row.GetProperty("minimum_players_per_team");
                    if (teamRows.ValueKind != JsonValueKind.Array || teamRows.GetArrayLength() != 2) throw Invalid();
                    var teams = teamRows.EnumerateArray().Select(x => x.TryGetInt32(out int n) && n >= 0 && n <= 32 ? n : throw Invalid()).ToArray();
                    var policy = new MagMatchStartupPolicy(minimum, maximum, teams);
                    var drain = row.GetProperty("draining");
                    if (drain.ValueKind != JsonValueKind.True && drain.ValueKind != JsonValueKind.False) throw Invalid();
                    workers.Add(new MagConfiguredWorker(id, host, workerPort, policy.MinimumPlayers, maximum, Array.AsReadOnly(teams), drain.GetBoolean()));
                }
                return new MagWorldRegistryConfig(20231, network.Address, (ushort)port, workers.ToArray(), inviteSeconds, adoption);
            }
            catch (Exception error) when (error is JsonException || error is InvalidOperationException || error is FormatException || error is OverflowException)
            { throw Invalid(); }
        }
        public void ValidateDrainReload(MagWorldRegistryConfig next)
        {
            if (next == null || ApplicationId != next.ApplicationId || !ClientAddress.Equals(next.ClientAddress) || ClientTcpPort != next.ClientTcpPort || PartyInviteSeconds != next.PartyInviteSeconds || Adoption != next.Adoption || Workers.Count != next.Workers.Count) throw Invalid();
            foreach (var worker in Workers)
            {
                var other = next.Workers.SingleOrDefault(w => w.WorkerId == worker.WorkerId);
                if (other == null || worker.WorkerHost != other.WorkerHost || worker.WorkerPort != other.WorkerPort ||
                    worker.MinimumPlayers != other.MinimumPlayers || worker.MaximumPlayers != other.MaximumPlayers ||
                    !worker.MinimumPlayersPerTeam.SequenceEqual(other.MinimumPlayersPerTeam)) throw Invalid();
            }
        }
        // Every member is optional; unknown and duplicate members are rejected.
        static MagWorkerAdoptionPolicy ParseAdoption(JsonElement value)
        {
            if (value.ValueKind != JsonValueKind.Object) throw Invalid();
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var field in value.EnumerateObject())
                if (!names.Add(field.Name) || field.Name is not ("enabled" or "max_adoptions" or "window_seconds")) throw Invalid();
            var policy = MagWorkerAdoptionPolicy.Default;
            if (names.Contains("enabled"))
            {
                var enabled = value.GetProperty("enabled");
                if (enabled.ValueKind != JsonValueKind.True && enabled.ValueKind != JsonValueKind.False) throw Invalid();
                policy = policy with { Enabled = enabled.GetBoolean() };
            }
            if (names.Contains("max_adoptions")) policy = policy with { MaximumAdoptions = Integer(value, "max_adoptions", 1, 100) };
            if (names.Contains("window_seconds")) policy = policy with { Window = TimeSpan.FromSeconds(Integer(value, "window_seconds", 1, 86400)) };
            return policy;
        }
        static string Text(JsonElement value, string field) => value.GetProperty(field).ValueKind == JsonValueKind.String ? value.GetProperty(field).GetString() : throw Invalid();
        static int Integer(JsonElement value, string field, int low, int high) => value.GetProperty(field).TryGetInt32(out int n) && n >= low && n <= high ? n : throw Invalid();
        static void Exact(JsonElement value, params string[] expected)
        {
            if (value.ValueKind != JsonValueKind.Object) throw Invalid();
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var field in value.EnumerateObject()) if (!names.Add(field.Name) || !expected.Contains(field.Name)) throw Invalid();
            if (names.Count != expected.Length) throw Invalid();
        }
        static ArgumentException Invalid() => new("Invalid configured world registry or unsupported structural reload.");
    }
}
