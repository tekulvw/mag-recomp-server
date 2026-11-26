using System.Net;
using System.Text;
using System.Text.Json;
using RT.Models.MAG;
using Server.Medius;

// The read-only server status behind the portal's public status page: world states, running
// matches, counts, versions and the signed GET /admin/v1/status endpoint. Accounts, workers and
// profiles are invented; the status must never carry an account name.
internal static class ServerStatusChecks
{
    static void Check(bool value, string message) { if (!value) throw new Exception(message); }

    static readonly MagMatchStartupPolicy Row64 = new(1, 64, new[] { 0, 0 });

    static LocalDmeWorkerPlacement Placement(bool ready, bool draining, MagMatchProfile current, int[] live, int pending,
        params MagMatchProfile[] profiles) =>
        new(ready, draining, ready && !draining, profiles, Row64, new[] { 0, 0 }, current, live, live.Sum(), new[] { 0, 0, 0 }, pending);

    /// <summary>World states and match counts from synthetic placement state.</summary>
    public static void Describe()
    {
        Check(MagServerStatus.WorldState(false, true, false) == "open", "A ready world is open");
        Check(MagServerStatus.WorldState(false, true, true) == "draining", "A drained world is draining");
        Check(MagServerStatus.WorldState(false, false, false) == "unavailable", "A world that is not ready is unavailable");
        Check(MagServerStatus.WorldState(true, true, false) == "faulted" && MagServerStatus.WorldState(true, false, true) == "faulted",
            "A fault outranks every other state");

        // A 256-Character Domination world: humans capped at 32, bots fill the rest.
        var large = new MagMatchProfile("synthetic-dom-1-2", 2, 1, 2, 256, 128, BotFillTotal: 256,
            AdvertisedHumanCapacity: 32, AdvertisedHumanTeamCapacity: 16);
        var other = large with { ProfileId = "synthetic-dom-2-1", AttackingFaction = 2, DefendingFaction = 1 };
        var running = MagServerStatus.Describe("domination", false, Placement(true, false, large, new[] { 4, 3, 2 }, 1, large, other));
        Check(running.State == "open" && running.GameTypes.SequenceEqual(new[] { 2 }) && running.Pending == 1, "Running world state");
        var match = running.Match;
        Check(match != null && match.ProfileId == "synthetic-dom-1-2" && match.GameType == 2 && match.AttackingFaction == 1 &&
            match.DefendingFaction == 2, "Running match identity");
        // Faction 0 is not in this match; its count must not leak into either side.
        Check(match.Humans == 9 && match.AttackingHumans == 3 && match.DefendingHumans == 2, "Humans per side come from the match's factions");
        Check(match.HumanCapacity == 32 && running.HumanCapacity == 32 && match.WorldCapacity == 256 && match.Bots == 247,
            "Large world counts use the human cap and the bot fill");

        // Legacy rows: every seat may be human, within the configured row; no bot fill means no bots.
        var legacy = new MagMatchProfile("synthetic-sab-0-1", 0, 0, 1, 128, 64, AdvertisedHumanCapacity: 32, AdvertisedHumanTeamCapacity: 16);
        var small = new MagMatchProfile("synthetic-acq-1-0", 1, 1, 0, 16, 8);
        var idle = MagServerStatus.Describe("mixed", false, Placement(true, true, null, new[] { 0, 0, 0 }, 0, legacy, small));
        Check(idle.State == "draining" && idle.Match == null && idle.GameTypes.SequenceEqual(new[] { 0, 1 }) && idle.HumanCapacity == 32,
            "An idle world reports the most humans any profile admits");
        var bounded = new LocalDmeWorkerPlacement(true, false, true, new[] { small }, new MagMatchStartupPolicy(1, 6, new[] { 0, 0 }),
            new[] { 0, 0 }, small, new[] { 2, 1, 0 }, 3, new[] { 0, 0, 0 }, 0);
        var row = MagServerStatus.Describe("bounded", false, bounded);
        Check(row.HumanCapacity == 6 && row.Match.HumanCapacity == 6 && row.Match.Bots == 0 && row.Match.AttackingHumans == 1 &&
            row.Match.DefendingHumans == 2, "A configured row bounds humans below the profile");
        var faulted = MagServerStatus.Describe("broken", true, Placement(false, true, null, new[] { 0, 0, 0 }, 0));
        Check(faulted.State == "faulted" && faulted.GameTypes.Count == 0 && faulted.HumanCapacity == 0, "A faulted world without profiles");
        Check(faulted.HumanLimit == 0 && faulted.WorldSize == 0 && faulted.BotFill == null, "A world without profiles holds nobody");
    }

    /// <summary>The effective human limit: the row and profile cap within the server-wide DME port
    /// cap, reported apart from the world size and the bot fill.</summary>
    public static void HumanLimit()
    {
        // Live's shapes: a 64-Character Sabotage row where every seat may be human, and
        // 128/256-Character worlds whose profiles cap humans at 32; 32 DME ports in all.
        var sabotage = new MagMatchProfile("synthetic-sab-1-2", 0, 1, 2, 64, 32, BotFillTotal: 64);
        var acquisition = new MagMatchProfile("synthetic-acq-1-2", 1, 1, 2, 128, 64, BotFillTotal: 128,
            AdvertisedHumanCapacity: 32, AdvertisedHumanTeamCapacity: 16);
        var domination = new MagMatchProfile("synthetic-dom-1-2", 2, 1, 2, 256, 128, BotFillTotal: 256,
            AdvertisedHumanCapacity: 32, AdvertisedHumanTeamCapacity: 16);

        var idle = MagServerStatus.Describe("sabotage", false, Placement(true, false, null, new[] { 0, 0, 0 }, 0, sabotage), 32);
        Check(idle.HumanCapacity == 64 && idle.HumanLimit == 32 && idle.WorldSize == 64 && idle.BotFill == 64,
            "A 64-seat Sabotage row holds only as many humans as there are DME ports");
        var running = MagServerStatus.Describe("sabotage", false, Placement(true, false, sabotage, new[] { 0, 2, 1 }, 0, sabotage), 32);
        Check(running.HumanLimit == 32 && running.Match.HumanCapacity == 64 && running.Match.HumanLimit == 32 &&
            running.Match.BotFill == 64 && running.Match.Bots == 61 && running.WorldSize == 64, "A running Sabotage match");
        foreach (var (profile, size) in new[] { (acquisition, 128), (domination, 256) })
        {
            var world = MagServerStatus.Describe("large", false, Placement(true, false, null, new[] { 0, 0, 0 }, 0, profile), 32);
            Check(world.HumanCapacity == 32 && world.HumanLimit == 32 && world.WorldSize == size && world.BotFill == size,
                "A large world: 32 humans, bots fill to its size");
        }
        // A smaller port cap bounds every world; unbounded ports leave the row cap.
        Check(MagServerStatus.Describe("domination", false, Placement(true, false, null, new[] { 0, 0, 0 }, 0, domination), 16).HumanLimit == 16,
            "The port cap bounds a large world");
        Check(MagServerStatus.Describe("sabotage", false, Placement(true, false, null, new[] { 0, 0, 0 }, 0, sabotage)).HumanLimit == 64,
            "Without a port cap the row cap stands");
        Check(MagServerStatus.Describe("sabotage", false, Placement(true, false, null, new[] { 0, 0, 0 }, 0, sabotage), 0).HumanLimit == 0,
            "A zero port cap holds nobody");
        // No bot fill: the world size is still reported, and bot_fill is absent.
        var plain = new MagMatchProfile("synthetic-acq-0-1", 1, 0, 1, 16, 8);
        var noBots = MagServerStatus.Describe("plain", false, Placement(true, false, null, new[] { 0, 0, 0 }, 0, plain), 32);
        Check(noBots.WorldSize == 16 && noBots.BotFill == null && noBots.HumanLimit == 16, "A world without bot fill");
    }

    /// <summary>Release versions come from the newest heading of each bundled changelog.</summary>
    public static void Versions()
    {
        var both = MagServerNews.FromChangelogs("# Changelog\n## [Unreleased]\n## [0.11.0](https://example.invalid/compare) (2026-11-20)\n* note\n## 0.10.0 (2026-11-01)\n",
            "## 0.15.0 (2026-11-21)\n* note\n");
        Check(both.MediusVersion == "0.11.0" && both.GameplayVersion == "0.15.0", "Newest release of each changelog");
        var medius = MagServerNews.FromChangelogs("## 0.11.0 (2026-11-20)\n", null);
        Check(medius.MediusVersion == "0.11.0" && medius.GameplayVersion == null, "No gameplay notes, no gameplay version");
        var none = MagServerNews.FromChangelogs(null, null);
        Check(none.MediusVersion == null && none.GameplayVersion == null, "No notes, no versions");
    }

    static readonly byte[] Secret = Encoding.UTF8.GetBytes("synthetic-status-api-secret-0123456789abcdef");
    static int _nonce;

    static MagAdminApiRequest Signed(string method, string path, DateTime now, string body = "", bool tamper = false)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(body);
        string ts = new DateTimeOffset(now).ToUnixTimeSeconds().ToString();
        string nonce = "synthetic-status-" + (++_nonce).ToString("D6");
        string signature = MagAdminApiAuthenticator.Sign(Secret, method, path, ts, nonce, bytes);
        if (tamper) signature = new string('0', 64);
        return new MagAdminApiRequest
        {
            Method = method, PathAndQuery = path, Body = bytes,
            Headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
            {
                [MagAdminApiAuthenticator.TimestampHeader] = ts,
                [MagAdminApiAuthenticator.NonceHeader] = nonce,
                [MagAdminApiAuthenticator.ActorHeader] = "portal status",
                [MagAdminApiAuthenticator.SignatureHeader] = signature,
            },
        };
    }

    /// <summary>The signed endpoint: counts, versions and uptime, and never an account name.</summary>
    public static void Endpoint()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-status-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var now = new DateTime(2026, 1, 1, 12, 0, 0, DateTimeKind.Utc);
            // Two live leases and one expired, in two role markers; one account holds a lease in both.
            MagActiveAccounts.Write(Path.Combine(directory, MagActiveAccounts.FilePrefix + "main.json"), new Dictionary<string, DateTime>
            { ["synthetic-alpha"] = now.AddSeconds(60), ["synthetic-bravo"] = now.AddSeconds(30), ["synthetic-gone"] = now.AddSeconds(-5) }, now);
            MagActiveAccounts.Write(Path.Combine(directory, MagActiveAccounts.FilePrefix + "maps.json"), new Dictionary<string, DateTime>
            { ["synthetic-alpha"] = now.AddSeconds(90) }, now);
            Check(MagServerStatus.PlayersOnline(directory, now) == 2, "Players online counts distinct live leases");
            Check(MagServerStatus.PlayersOnline(Path.Combine(directory, "missing"), now) == null, "No markers, no count");
            string broken = Path.Combine(directory, "broken");
            Directory.CreateDirectory(broken);
            File.WriteAllText(Path.Combine(broken, MagActiveAccounts.FilePrefix + "main.json"), "{not json");
            Check(MagServerStatus.PlayersOnline(broken, now) == null, "An unreadable marker gives no count instead of failing");

            var profile = new MagMatchProfile("synthetic-sab-1-0", 0, 1, 0, 64, 32, BotFillTotal: 64);
            var worlds = new[]
            {
                MagServerStatus.Describe("sabotage", false, Placement(true, false, profile, new[] { 1, 2, 0 }, 0, profile), 32),
                MagServerStatus.Describe("acquisition", true, Placement(false, true, null, new[] { 0, 0, 0 }, 0)),
            };
            var ports = new MagDmePortBudget(10505, null);
            var held = ports.TryAcquire(3);
            var started = now.AddHours(-2);
            object Status(DateTime at) => MagServerStatus.Report("registry", worlds, new[] { 2, 1, 0 },
                MagServerStatus.PlayersOnline(directory, at), ports, "0.11.0", "0.15.0", started, at);
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            var api = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), Secret, store, directory, directory, () => now, _ => { }, ports, Status);

            var response = api.Handle(Signed("GET", "/admin/v1/status", now));
            Check(response.Status == 200, "Status refused: " + response.Status);
            string text = Encoding.UTF8.GetString(response.Body);
            Check(!text.Contains("synthetic-alpha") && !text.Contains("synthetic-bravo") && !text.Contains("synthetic-gone"),
                "The status must not name accounts");
            var body = JsonDocument.Parse(response.Body).RootElement;
            Check(body.GetProperty("medius_version").GetString() == "0.11.0" && body.GetProperty("gameplay_version").GetString() == "0.15.0",
                "Versions");
            Check(body.GetProperty("started_at").GetString() == "2026-01-01T10:00:00Z" && body.GetProperty("uptime_seconds").GetInt64() == 7200,
                "Start time and uptime");
            Check(body.GetProperty("mode").GetString() == "registry" && body.GetProperty("players_online").GetInt32() == 2 &&
                body.GetProperty("queued").GetInt32() == 3 && body.GetProperty("queued_by_game_type").GetArrayLength() == 3,
                "Population and queue counts");
            var dme = body.GetProperty("dme_ports");
            Check(dme.GetProperty("capacity").GetInt32() == 32 && dme.GetProperty("in_use").GetInt32() == 3, "DME ports in use against the cap");
            var rows = body.GetProperty("worlds");
            Check(rows.GetArrayLength() == 2, "Every configured world is listed");
            var open = rows[0];
            Check(open.GetProperty("worker_id").GetString() == "sabotage" && open.GetProperty("state").GetString() == "open" &&
                open.GetProperty("human_capacity").GetInt32() == 64 && open.GetProperty("pending").GetInt32() == 0, "Open world");
            var match = open.GetProperty("match");
            Check(match.GetProperty("profile_id").GetString() == "synthetic-sab-1-0" && match.GetProperty("game_type").GetInt32() == 0 &&
                match.GetProperty("attacking_faction").GetInt32() == 1 && match.GetProperty("defending_faction").GetInt32() == 0 &&
                match.GetProperty("humans").GetInt32() == 3 && match.GetProperty("attacking_humans").GetInt32() == 2 &&
                match.GetProperty("defending_humans").GetInt32() == 1 && match.GetProperty("bots").GetInt32() == 61 &&
                match.GetProperty("world_capacity").GetInt32() == 64, "Running match");
            Check(open.GetProperty("human_limit").GetInt32() == 32 && open.GetProperty("world_size").GetInt32() == 64 &&
                open.GetProperty("bot_fill").GetInt32() == 64 && match.GetProperty("human_limit").GetInt32() == 32 &&
                match.GetProperty("bot_fill").GetInt32() == 64 && body.GetProperty("human_limit").GetInt32() == 32,
                "The effective human limit, world size and bot fill, apart from the row cap");
            Check(rows[1].GetProperty("bot_fill").ValueKind == JsonValueKind.Null, "No profiles, no bot fill");
            Check(rows[1].GetProperty("state").GetString() == "faulted" && rows[1].GetProperty("match").ValueKind == JsonValueKind.Null,
                "Faulted world without a match");

            Check(api.Handle(Signed("POST", "/admin/v1/status", now, "{}")).Status == 405, "The status is read-only");
            Check(api.Handle(Signed("GET", "/admin/v1/status", now, tamper: true)).Status == 401, "The status needs a signature");
            var without = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), Secret, store, directory, directory, () => now, _ => { });
            Check(without.Handle(Signed("GET", "/admin/v1/status", now)).Status == 404, "No status source, no endpoint");

            // Without a registry or local world, the process still reports itself.
            var empty = JsonDocument.Parse(JsonSerializer.SerializeToUtf8Bytes(
                MagServerStatus.Capture(null, null, null, directory, now))).RootElement;
            Check(empty.GetProperty("mode").GetString() == "none" && empty.GetProperty("worlds").GetArrayLength() == 0 &&
                empty.GetProperty("players_online").GetInt32() == 2 && empty.GetProperty("dme_ports").GetProperty("capacity").ValueKind == JsonValueKind.Null,
                "Status without worlds");
            foreach (var lease in held) lease.Dispose();
        }
        finally { Directory.Delete(directory, true); }
    }

    /// <summary>Against invented loopback workers: a world that never answered is faulted, a
    /// running match is counted, and the report names no account.</summary>
    public static void Registry()
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try { LeadershipProgressionChecks.RegistryStatus(); }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    }
}

internal static partial class LeadershipProgressionChecks
{
    public static void RegistryStatus() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "status", ("alpha", 0), ("bravo", 1));
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            await using var ready = new RegistryWorker("first") { Fence = true };
            await using var down = new RegistryWorker("second") { Fence = true, Offline = true };
            await using var registry = AdoptionRegistry(store, null, AdoptionWorkerRow("first", ready.Port), AdoptionWorkerRow("second", down.Port));
            await registry.InitializeAsync();
            MagWorldStatus Status(string id) => registry.StatusWorlds().Single(w => w.WorkerId == id);
            Check(Status("first") is { State: "open", Match: null, Pending: 0 } first && first.GameTypes.SequenceEqual(new[] { 2 }) &&
                first.HumanCapacity == 2, "A ready idle world is open without a match");
            Check(Status("second") is { State: "faulted", Match: null }, "A worker unreachable at startup is faulted");
            Check(registry.QueuedByGameType().Sum() == 0, "Nobody is queued yet");

            var replies = new[] { QueueRegistryJoin(registry, store, "alpha", 1), QueueRegistryJoin(registry, store, "bravo", 2) };
            await TickUntil(registry, () => replies.All(t => t.IsCompleted), "Synthetic registry admission did not finish");
            Check((await Task.WhenAll(replies)).All(reply => reply.Result == 0), "Synthetic admission failed");
            var match = Status("first").Match;
            Check(match != null && match.GameType == 2 && match.Humans == 2 && match.AttackingHumans == 1 && match.DefendingHumans == 1 &&
                match.ProfileId.StartsWith("profile-") && match.Bots == 0, "The running match is counted");
            Check(registry.QueuedByGameType().Sum() == 0, "Admitted players are not queued");

            var report = MagServerStatus.Report("registry", registry.StatusWorlds(), registry.QueuedByGameType(), 2, registry.PortBudget,
                null, null, DateTime.UtcNow, DateTime.UtcNow);
            string text = JsonSerializer.Serialize(report);
            Check(!text.Contains("alpha") && !text.Contains("bravo"), "The registry status must not name accounts");
        }
    });
}
