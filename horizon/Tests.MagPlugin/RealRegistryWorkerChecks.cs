using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static class RealRegistryWorkerChecks
{
    public const string BinaryVariable = "MAG_REGISTRY_TEST_BINARY";
    private const string CatalogsVariable = "MAG_REGISTRY_TEST_CATALOGS";
    private static readonly TimeSpan Deadline = TimeSpan.FromSeconds(10);

    public static void Run()
    {
        string binary = Path.GetFullPath(Environment.GetEnvironmentVariable(BinaryVariable));
        string catalogPath = Environment.GetEnvironmentVariable(CatalogsVariable);
        if (string.IsNullOrWhiteSpace(catalogPath))
            throw new InvalidOperationException($"{CatalogsVariable} must point to generated synthetic catalogs.");
        string catalogs = Path.GetFullPath(catalogPath);
        LeadershipProgressionChecks.RealRegistryWorkers(binary, catalogs);
    }

    public static void RunParties()
    {
        string binary = Path.GetFullPath(Environment.GetEnvironmentVariable(BinaryVariable));
        string catalogs = Path.GetFullPath(Environment.GetEnvironmentVariable(CatalogsVariable));
        LeadershipProgressionChecks.RealRegistryParties(binary, catalogs);
    }

    public static void RunLargeWorld()
    {
        string binary = Path.GetFullPath(Environment.GetEnvironmentVariable(BinaryVariable));
        string catalogs = Path.GetFullPath(Environment.GetEnvironmentVariable(CatalogsVariable));
        LeadershipProgressionChecks.RealRegistryLargeWorld(binary, catalogs);
    }

    public static void RunMediusRestart()
    {
        string binary = Path.GetFullPath(Environment.GetEnvironmentVariable(BinaryVariable));
        string catalogs = Path.GetFullPath(Environment.GetEnvironmentVariable(CatalogsVariable));
        LeadershipProgressionChecks.RealRegistryMediusRestart(binary, catalogs);
    }

    public static void RunRestart()
    {
        string binary = Path.GetFullPath(Environment.GetEnvironmentVariable(BinaryVariable));
        string catalogs = Path.GetFullPath(Environment.GetEnvironmentVariable(CatalogsVariable));
        LeadershipProgressionChecks.RealRegistryRestart(binary, catalogs);
    }

    internal sealed class Worker : IAsyncDisposable
    {
        private readonly Process _process;
        private readonly Task<string> _errors;
        private readonly string _journal;
        private int _disposed;
        public string Id { get; }
        public int Port { get; private set; }

        private Worker(string id, Process process, string journal)
        {
            Id = id;
            _process = process;
            _journal = journal;
            _errors = process.StandardError.ReadToEndAsync();
        }

        /// <summary>Start an owned worker. <paramref name="port"/> rebinds a known endpoint, as a restarted
        /// worker does; <paramref name="instance"/> keeps each process's journal separate.</summary>
        public static async Task<Worker> StartAsync(string id, string directory, string binary, string catalogs, bool parties = false,
            int port = 0, string instance = null, string profiles = null)
        {
            string workingDirectory = Directory.CreateDirectory(Path.Combine(directory, instance ?? id)).FullName;
            string journal = Path.Combine(workingDirectory, "worker.jsonl");
            var start = new ProcessStartInfo(binary)
            {
                WorkingDirectory = workingDirectory,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            foreach (string arg in new[] { "--worker-id", id, "--bind", $"127.0.0.1:{port}", "--log", journal, "--application-id", "20231" })
                start.ArgumentList.Add(arg);
            foreach (var (option, file) in new[]
            {
                ("--item-catalog", "items.json"), ("--spawn-catalog", "spawns.json"), ("--wire-profiles", "wire.json"),
                ("--character-blast-rules", "blast.json"), ("--bootstrap-loadouts", "bootstrap.json"),
                ("--retail-combat-metadata", "combat.json"), ("--retail-armory-catalog", "armory.json"),
                ("--skill-catalog", "skills.json"), ("--fall-damage-rules", "fall.json"), ("--match-profiles", profiles ?? (parties ? "party-profiles.json" : "profiles.json"))
            })
            {
                start.ArgumentList.Add(option);
                start.ArgumentList.Add(Path.Combine(catalogs, file));
            }

            if (parties) start.ArgumentList.Add("--party-reservations");
            var process = Process.Start(start) ?? throw new InvalidOperationException("Could not start the owned synthetic worker.");
            var worker = new Worker(id, process, journal);
            try
            {
                using var stop = new CancellationTokenSource(Deadline);
                while (await process.StandardOutput.ReadLineAsync(stop.Token) is { } line)
                {
                    var message = JsonNode.Parse(line);
                    if (message?["event"]?.GetValue<string>() == "listening")
                    {
                        var endpoint = IPEndPoint.Parse(message["address"].GetValue<string>());
                        if (!endpoint.Address.Equals(IPAddress.Loopback) || endpoint.Port == 0 || (port != 0 && endpoint.Port != port))
                            throw new IOException("The owned synthetic worker did not bind an ephemeral loopback endpoint.");
                        worker.Port = endpoint.Port;
                        return worker;
                    }
                }
                throw new IOException($"Synthetic worker {id} exited during startup. {await worker._errors.WaitAsync(stop.Token)}");
            }
            catch
            {
                await worker.DisposeAsync();
                throw;
            }
        }

        public async Task<JsonObject> RequestAsync(string operation, JsonObject identity = null, string reservation = null)
        {
            using var deadline = new CancellationTokenSource(Deadline);
            using var socket = new TcpClient();
            await socket.ConnectAsync(IPAddress.Loopback, Port, deadline.Token);
            using var stream = socket.GetStream();
            using var reader = new StreamReader(stream);
            string requestId = Guid.NewGuid().ToString("N");
            var request = new JsonObject { ["v"] = 2, ["request_id"] = requestId, ["op"] = operation };
            if (identity != null) request["worker_identity"] = identity.DeepClone();
            if (reservation != null) request["reservation_id"] = reservation;
            await stream.WriteAsync(Encoding.UTF8.GetBytes(request.ToJsonString() + "\n"), deadline.Token);
            string line = await reader.ReadLineAsync(deadline.Token);
            var reply = JsonNode.Parse(line).AsObject();
            if (reply["request_id"]?.GetValue<string>() != requestId)
                throw new InvalidDataException("The worker replied to a different request.");
            return reply;
        }

        public IReadOnlyList<JsonObject> ReadJournal()
        {
            using var stream = new FileStream(_journal, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
            using var reader = new StreamReader(stream);
            var lines = reader.ReadToEnd().Split('\n');
            // A concurrent append can leave the final line incomplete.
            return lines.SkipLast(1).Select(line => JsonNode.Parse(line).AsObject()).ToArray();
        }

        /// <summary>Kill the process abruptly, as a crash would. Idempotent.</summary>
        public Task KillAsync() => DisposeAsync().AsTask();

        public async ValueTask DisposeAsync()
        {
            if (Interlocked.Exchange(ref _disposed, 1) != 0) return;
            using (_process)
            {
                if (!_process.HasExited)
                {
                    try { _process.Kill(entireProcessTree: true); }
                    catch (InvalidOperationException) when (_process.HasExited) { }
                }
                using var stop = new CancellationTokenSource(Deadline);
                await _process.WaitForExitAsync(stop.Token);
                await _errors.WaitAsync(stop.Token);
            }
        }
    }
}

internal static partial class LeadershipProgressionChecks
{
    public static void RealRegistryWorkers(string binary, string catalogs) => WithDirectory(directory =>
    {
        var settings = Server.Medius.Program.Settings;
        int previous = settings.MagApplicationId;
        settings.MagApplicationId = 20231;
        try { VerifyAsync().GetAwaiter().GetResult(); }
        finally { settings.MagApplicationId = previous; }

        async Task VerifyAsync()
        {
            const string address = "192.0.2.9";
            const int tcpPort = 12073;
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            var characters = new Dictionary<string, uint>(StringComparer.Ordinal);
            foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1), ("charlie", (byte)0), ("delta", (byte)1) })
            {
                var request = new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] };
                Check(store.CreateForAccount(request, account).Result == MagNetResult.Success, "Synthetic character creation failed");
                characters.Add(account, store.ReadForAccount(account).Single().Id);
            }

            await using var first = await RealRegistryWorkerChecks.Worker.StartAsync("first", directory, binary, catalogs);
            await using var second = await RealRegistryWorkerChecks.Worker.StartAsync("second", directory, binary, catalogs);
            JsonObject WorkerConfig(RealRegistryWorkerChecks.Worker worker) => new()
            {
                ["worker_id"] = worker.Id, ["worker_host"] = "127.0.0.1", ["worker_port"] = worker.Port, ["draining"] = false,
                ["minimum_players"] = 2, ["maximum_players"] = 2, ["minimum_players_per_team"] = new JsonArray(1, 1)
            };
            var config = new JsonObject
            {
                ["format"] = "mag_world_registry_v1", ["application_id"] = 20231, ["client_address"] = address, ["client_tcp_port"] = tcpPort,
                ["workers"] = new JsonArray(WorkerConfig(first), WorkerConfig(second))
            };
            IReadOnlyList<JsonObject> aReservations;
            IReadOnlyList<JsonObject> bReservations;
            await using (var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store))
            {
                await registry.InitializeAsync();
                var aWorld = registry.Worlds.Single(world => world.WorkerId == first.Id);
                var bWorld = registry.Worlds.Single(world => world.WorkerId == second.Id);
                Check(aWorld.Ready && bWorld.Ready && aWorld.WorldId != bWorld.WorldId &&
                    aWorld.IncarnationId != null && bWorld.IncarnationId != null && aWorld.IncarnationId != bWorld.IncarnationId,
                    "Real workers did not produce distinct ready worlds and incarnations");

                int transaction = 0;
                Task<MagJoinGameResponse> Queue(string account)
                {
                    var session = MakeSession(store, 20231, account, characters[account]);
                    var result = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                    registry.QueueJoin(session, new MagRetailJoinGameRequest { TransactionId = ++transaction, Preference = 1, Value = 0 },
                        message => { if (message is MagJoinGameResponse response) result.TrySetResult(response); }, () => true);
                    return result.Task;
                }
                async Task<MagJoinGameResponse[]> AdmitAsync(params Task<MagJoinGameResponse>[] queued)
                {
                    using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(10));
                    while (queued.Any(task => !task.IsCompleted))
                    {
                        await registry.TickAsync().WaitAsync(deadline.Token);
                        await Task.Delay(10, deadline.Token);
                    }
                    return await Task.WhenAll(queued);
                }

                var lone = Queue("alpha");
                await registry.TickAsync();
                Check(!lone.IsCompleted && registry.Status(1).QueueSizes[0] == 1 &&
                    Reservations(first).Count == 0 && Reservations(second).Count == 0,
                    "A lone faction consumed a worker without a feasible quorum");
                var aAdmitted = await AdmitAsync(lone, Queue("bravo"));
                var bAdmitted = await AdmitAsync(Queue("charlie"), Queue("delta"));
                Check(aAdmitted.All(reply => Accepts(reply, aWorld)) && bAdmitted.All(reply => Accepts(reply, bWorld)) &&
                    aAdmitted.Concat(bAdmitted).Select(reply => Convert.ToHexString(reply.AccessKey)).Distinct().Count() == 4,
                    "Real worker acknowledgments lost the owning world, endpoint or unique access key");
                Check(registry.Worlds.All(world => world.Ready && world.ReservedPlayers == 2),
                    "Both real workers did not retain independent reserved quorums");
                aReservations = Reservations(first);
                bReservations = Reservations(second);
                CheckReserves(aReservations, aWorld, "alpha", "bravo");
                CheckReserves(bReservations, bWorld, "charlie", "delta");
                Check(!Ids(aReservations).Overlaps(Ids(bReservations)), "Reservation IDs overlapped across real workers");

                var aIdentity = new JsonObject { ["worker_id"] = aWorld.WorkerId, ["incarnation_id"] = aWorld.IncarnationId };
                var bIdentity = new JsonObject { ["worker_id"] = bWorld.WorkerId, ["incarnation_id"] = bWorld.IncarnationId };
                var stale = new JsonObject { ["worker_id"] = aWorld.WorkerId, ["incarnation_id"] = "synthetic-stale-incarnation" };
                string aReservation = aReservations[0]["reservation_id"].GetValue<string>();
                await RejectRelease(first, bIdentity, aIdentity, aReservation);
                await RejectRelease(first, stale, aIdentity, aReservation);
                await RejectRelease(second, aIdentity, bIdentity, aReservation);
                Check(Releases(first).Count == 0 && Releases(second).Count == 0,
                    "A fenced release reached a worker recorder before registry disposal");

                bool Accepts(MagJoinGameResponse response, MagRegisteredWorld world) =>
                    response.Result == 0 && response.WorldId == world.WorldId && response.ServerPort == tcpPort &&
                    response.ServerAddress.TakeWhile(value => value != 0).SequenceEqual(Encoding.ASCII.GetBytes(address)) &&
                    response.AccessKey.Any(value => value != 0);
            }

            CheckDisposal(first, aReservations);
            CheckDisposal(second, bReservations);

            void CheckReserves(IReadOnlyList<JsonObject> receipts, MagRegisteredWorld world, params string[] accounts)
            {
                Check(receipts.Count == 2 && receipts.All(row => row["world_id"].GetValue<int>() == world.WorldId) &&
                    receipts.Select(row => row["account_id"].GetValue<uint>()).ToHashSet().SetEquals(accounts.Select(account => characters[account])) &&
                    receipts.Select(row => row["team"].GetValue<int>()).ToHashSet().SetEquals(new[] { 0, 1 }),
                    "A real worker recorded another world's membership or failed to assign opposite factions to separate teams");
            }
        }
    });

    /// <summary>A real worker with 128-Character profiles advertises world_capacity_v1 and human caps
    /// 32/16; Horizon offers them only on a registry row that large and configures 32 humans.</summary>
    public static void RealRegistryLargeWorld(string binary, string catalogs) => WithDirectory(directory =>
    {
        var settings = Server.Medius.Program.Settings;
        int previous = settings.MagApplicationId;
        settings.MagApplicationId = 20231;
        try { VerifyAsync().GetAwaiter().GetResult(); }
        finally { settings.MagApplicationId = previous; }

        async Task VerifyAsync()
        {
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            var characters = new Dictionary<string, uint>(StringComparer.Ordinal);
            foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1) })
            {
                var request = new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] };
                Check(store.CreateForAccount(request, account).Result == MagNetResult.Success, "Synthetic character creation failed");
                characters.Add(account, store.ReadForAccount(account).Single().Id);
            }
            await using var worker = await RealRegistryWorkerChecks.Worker.StartAsync("large", directory, binary, catalogs,
                profiles: "large-profiles.json");

            // The hello Horizon parses carries world size and the human caps.
            var hello = await worker.RequestAsync("hello");
            Check(GameplayBridgeResponseCodec.TryParse(System.Text.Encoding.UTF8.GetBytes(hello.ToJsonString()),
                    hello["request_id"]!.GetValue<string>(), false, out var parsed) && parsed.Ok &&
                parsed.Capabilities.Contains("world_capacity_v1") && parsed.MatchProfiles.Count == 6 &&
                parsed.MatchProfiles.All(p => p.MaximumPlayers == 128 && p.TeamCapacity == 64 && p.LargeWorld &&
                    p.HumanCapacity == 32 && p.HumanTeamCapacity == 16),
                "A real worker did not advertise 128-Character rows with human caps 32/16 under world_capacity_v1");

            JsonObject Config(int maximum) => new()
            {
                ["format"] = "mag_world_registry_v1", ["application_id"] = 20231, ["client_address"] = "192.0.2.9", ["client_tcp_port"] = 12073,
                ["workers"] = new JsonArray(new JsonObject
                {
                    ["worker_id"] = worker.Id, ["worker_host"] = "127.0.0.1", ["worker_port"] = worker.Port, ["draining"] = false,
                    ["minimum_players"] = 2, ["maximum_players"] = maximum, ["minimum_players_per_team"] = new JsonArray(1, 1)
                })
            };
            // A row limited to 64 Characters never offers the larger profiles; with nothing left the world stays down.
            bool refused = false;
            try
            {
                await using var narrow = new MagWorldRegistry(MagWorldRegistryConfig.Parse(Config(64).ToJsonString()), store);
                await narrow.InitializeAsync();
            }
            catch (IOException) { refused = true; }
            Check(refused, "A registry row of 64 offered a real worker's 128-Character profiles");

            await using (var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(Config(128).ToJsonString()), store))
            {
                await registry.InitializeAsync();
                var world = registry.Worlds.Single();
                Check(world.Ready, "A registry row of 128 did not ready the real 128-Character worker");
                int transaction = 0;
                Task<MagJoinGameResponse> Queue(string account)
                {
                    var session = MakeSession(store, 20231, account, characters[account]);
                    var result = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                    registry.QueueJoin(session, new MagRetailJoinGameRequest { TransactionId = ++transaction, Preference = 1, Value = 0 },
                        message => { if (message is MagJoinGameResponse response) result.TrySetResult(response); }, () => true);
                    return result.Task;
                }
                var queued = new[] { Queue("alpha"), Queue("bravo") };
                using (var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(10)))
                    while (queued.Any(task => !task.IsCompleted))
                    {
                        await registry.TickAsync().WaitAsync(deadline.Token);
                        await Task.Delay(10, deadline.Token);
                    }
                var replies = await Task.WhenAll(queued);
                var configured = worker.ReadJournal().Where(row => row["op"]?.GetValue<string>() == "match_configured").ToArray();
                Check(replies.All(reply => reply.Result == 0 && reply.WorldId == world.WorldId) && Reservations(worker).Count == 2 &&
                    configured.Length == 1 && configured[0]["maximum_players"]!.GetValue<int>() == 32,
                    "A real 128-Character world did not admit humans under a 32-human configuration");
            }
        }
    });

    private static IReadOnlyList<JsonObject> Reservations(RealRegistryWorkerChecks.Worker worker) =>
        worker.ReadJournal().Where(row => row["op"]?.GetValue<string>() == "reserved").ToArray();

    private static IReadOnlyList<JsonObject> Releases(RealRegistryWorkerChecks.Worker worker) =>
        worker.ReadJournal().Where(row => row["op"]?.GetValue<string>().StartsWith("released", StringComparison.Ordinal) == true).ToArray();

    private static HashSet<string> Ids(IEnumerable<JsonObject> rows) =>
        rows.Select(row => row["reservation_id"].GetValue<string>()).ToHashSet(StringComparer.Ordinal);

    private static async Task RejectRelease(RealRegistryWorkerChecks.Worker worker, JsonObject sent, JsonObject actual, string id)
    {
        var reply = await worker.RequestAsync("release", sent, id);
        Check(reply["ok"].GetValue<bool>() == false && reply["error"]?.GetValue<string>() == "worker_identity_mismatch" &&
            JsonNode.DeepEquals(reply["worker_identity"], actual),
            "A real worker accepted a wrong or stale release fence or echoed the wrong incarnation");
    }

    private static void CheckDisposal(RealRegistryWorkerChecks.Worker worker, IReadOnlyList<JsonObject> reservations)
    {
        var rows = Releases(worker);
        Check(rows.All(row => row["op"].GetValue<string>() == "released") && rows.Count == 2 && Ids(rows).SetEquals(Ids(reservations)),
            "Registry shutdown failed to release only the exact reservations recorded on their owning worker");
    }
}
