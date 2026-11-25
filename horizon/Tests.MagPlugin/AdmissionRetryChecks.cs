using System.Collections.Concurrent;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// A gameplay worker that did not answer a join's reserve once left the player on
// the queue screen: Medius released the reservation and answered the ordinary
// failure. These synthetic workers drop or delay replies to prove that an
// unanswered admission is retried, then requeued or refused, never left hanging.
internal static partial class LeadershipProgressionChecks
{
    private static readonly GameplayAdmissionPolicy FastAdmission = new(3, TimeSpan.FromMilliseconds(500));

    public static void AdmissionPolicyConfiguration()
    {
        var defaults = GameplayAdmissionPolicy.Parse(null, null);
        Check(defaults.Attempts == GameplayAdmissionPolicy.DefaultAttempts && defaults.Deadline == GameplayBridge.RequestDeadline,
            "Unset admission settings changed the defaults");
        var configured = GameplayAdmissionPolicy.Parse("2", "3500");
        Check(configured.Attempts == 2 && configured.Deadline == TimeSpan.FromMilliseconds(3500), "Configured admission policy was not applied");
        foreach (var (attempts, deadline) in new (string, string)[] { ("0", null), ("6", null), ("two", null), ("-1", null), (null, "499"), (null, "10001"), (null, "2s") })
        {
            try
            {
                GameplayAdmissionPolicy.Parse(attempts, deadline);
                throw new Exception($"Invalid admission setting accepted: attempts={attempts} deadline={deadline}");
            }
            catch (ArgumentException) { }
        }
    }

    /// <summary>The first reserve reaches the worker and is never answered. Medius releases
    /// that reservation and admits the player under a new one.</summary>
    public static void UnansweredAdmissionIsRetried() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        WithSilentWorker(store, silentReserves: 1, async (host, worker, session) =>
        {
            var reply = await BeginJoin(host, session).WaitAsync(AdmissionTimeout);
            Check(reply.Result == 0, $"An admission retried after an unanswered reserve was refused: result={reply.Result}");
            var calls = worker.Calls();
            var reserves = calls.Where(call => call.Op == "reserve").ToList();
            Check(reserves.Count == 2 && reserves[0].ReservationId != reserves[1].ReservationId,
                "The retry did not reserve again under a new reservation");
            int released = calls.FindIndex(call => call.Op == "release" && call.ReservationId == reserves[0].ReservationId);
            Check(released >= 0 && released < calls.IndexOf(reserves[1]),
                "The unanswered reservation was not released before the retry reserved again");
        }).GetAwaiter().GetResult();
    });

    /// <summary>A reserve that never left Medius (the worker's name did not resolve) is sent
    /// again as it is: the worker cannot hold it, so nothing is released.</summary>
    public static void UndeliveredAdmissionIsResent() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        int lookups = 0;
        Func<string, CancellationToken, Task<IPAddress[]>> resolve = (host, token) =>
            Interlocked.Increment(ref lookups) == 2
                ? Task.FromException<IPAddress[]>(new SocketException((int)SocketError.TryAgain))
                : Task.FromResult(new[] { IPAddress.Loopback });
        WithSilentWorker(store, silentReserves: 0, async (host, worker, session) =>
        {
            // Hello resolved the name; the join's first lookup is lost.
            typeof(GameplayBridge).GetField("_addresses", HostInternals)!.SetValue(worker.Client, null);
            var reply = await BeginJoin(host, session).WaitAsync(AdmissionTimeout);
            Check(reply.Result == 0, $"An undelivered reserve was not sent again: result={reply.Result}");
            var calls = worker.Calls();
            Check(calls.Count(call => call.Op == "reserve") == 1 && !calls.Any(call => call.Op == "release"),
                "An undelivered reserve reached the worker or was released");
            Check(lookups == 3, $"Expected the lost lookup and one fresh lookup, saw {lookups - 1}");
        }, "synthetic-gameplay.invalid", resolve).GetAwaiter().GetResult();
    });

    /// <summary>A worker that never answers: each attempt is released, and the join ends with
    /// a reply within its bound. A queued join answers 6, which returns its ticket to the
    /// queue; the direct join keeps the ordinary failure.</summary>
    public static void UnansweredAdmissionRequeuesOrFails() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        WithSilentWorker(store, silentReserves: int.MaxValue, async (host, worker, session) =>
        {
            var queuedJoin = typeof(LocalDmeHost).GetMethod("JoinAsync", HostInternals, new[] {
                typeof(MagMlsSession), typeof(MagJoinGameRequest), typeof(Action<MagCharacterRecord>), typeof(Func<bool>) })!;
            var request = new MagRetailJoinGameRequest { Preference = 1, Value = (uint)Server.Medius.Program.Settings.MagGameType };
            var elapsed = Stopwatch.StartNew();
            var queued = await ((Task<MagJoinGameResponse>)queuedJoin.Invoke(host, new object[] { session, request, null, (Func<bool>)(() => true) })!)
                .WaitAsync(TimeSpan.FromSeconds(15));
            Check(queued.Result == 6, $"A queued join the worker never answered was not requeued: result={queued.Result}");
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(10), $"Queued admission retries were not bounded: {elapsed.ElapsedMilliseconds} ms");
            var calls = worker.Calls();
            var reserves = calls.Where(call => call.Op == "reserve").Select(call => call.ReservationId).ToList();
            Check(reserves.Count == FastAdmission.Attempts && reserves.Distinct().Count() == reserves.Count,
                $"Expected {FastAdmission.Attempts} distinct reserve attempts, saw {reserves.Count}");
            Check(reserves.All(id => calls.Any(call => call.Op == "release" && call.ReservationId == id)),
                "An unanswered reservation was left on the worker");
            Check(host.Status(1).QueueSizes.Sum(size => (int)size) == 0 && host.Status(1).PlayersInGame.Sum(size => (int)size) == 0,
                "A requeued join kept a seat in the world");

            int before = calls.Count;
            elapsed.Restart();
            var direct = await BeginJoin(host, session).WaitAsync(TimeSpan.FromSeconds(15));
            Check(direct.Result == 2, $"A direct join the worker never answered did not fail: result={direct.Result}");
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(10), $"Direct admission retries were not bounded: {elapsed.ElapsedMilliseconds} ms");
            Check(worker.Calls().Skip(before).Count(call => call.Op == "reserve") == FastAdmission.Attempts,
                "The direct join did not use the configured attempts");
        }).GetAwaiter().GetResult();
    });

    private static async Task WithSilentWorker(LocalCharacterStore store, int silentReserves,
        Func<LocalDmeHost, SilentWorker, MagMlsSession, Task> test, string host = null,
        Func<string, CancellationToken, Task<IPAddress[]>> resolve = null)
    {
        var settings = Server.Medius.Program.Settings;
        int previousApp = settings.MagApplicationId;
        settings.MagApplicationId = 20231;
        try
        {
            await using var worker = new SilentWorker(silentReserves, host, resolve);
            await using var dme = new LocalDmeHost(worker.Client, store) { AdmissionPolicy = FastAdmission };
            var hello = await worker.Client.RequestHelloAsync(new { v = 2, request_id = "synthetic-hello", op = "hello" });
            typeof(LocalDmeHost).GetMethod("InitializeGameplayHello", HostInternals)!.Invoke(dme, new object[] { hello });
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            typeof(LocalDmeHost).GetProperty(nameof(LocalDmeHost.World))!.SetValue(dme,
                new Server.Dme.Models.World(manager, 20231, settings.MagMaximumPlayers));
            typeof(LocalDmeHost).GetField("_ready", HostInternals)!.SetValue(dme, true);
            typeof(LocalDmeHost).GetField("_worldStreamHealthy", HostInternals)!.SetValue(dme, true);
            await test(dme, worker, MakeSession(store, 20231, "alpha", 1));
        }
        finally { settings.MagApplicationId = previousApp; }
    }

    private sealed record WorkerCall(string Op, string ReservationId);

    /// <summary>Loopback worker that receives the first <c>silentReserves</c> reserves and never
    /// answers them; every other request is acknowledged.</summary>
    private sealed class SilentWorker : IAsyncDisposable
    {
        private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        private readonly CancellationTokenSource _stop = new();
        private readonly ConcurrentQueue<WorkerCall> _calls = new();
        private readonly Task _loop;
        private int _silentReserves;
        public GameplayBridge Client { get; }

        public SilentWorker(int silentReserves, string host, Func<string, CancellationToken, Task<IPAddress[]>> resolve)
        {
            _silentReserves = silentReserves;
            _listener.Start();
            Client = new GameplayBridge(host ?? "127.0.0.1", ((IPEndPoint)_listener.LocalEndpoint).Port);
            if (resolve != null) typeof(GameplayBridge).GetField("_resolve", HostInternals)!.SetValue(Client, resolve);
            _loop = RunAsync();
        }

        public List<WorkerCall> Calls() => _calls.ToList();

        private async Task RunAsync()
        {
            var connections = new List<Task>();
            try
            {
                while (true)
                {
                    var socket = await _listener.AcceptTcpClientAsync(_stop.Token);
                    connections.RemoveAll(task => task.IsCompleted);
                    connections.Add(AnswerAsync(socket));
                }
            }
            catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            finally { await Task.WhenAll(connections); }
        }

        private async Task AnswerAsync(TcpClient socket)
        {
            using (socket)
            using (var stream = socket.GetStream())
            using (var reader = new StreamReader(stream))
            using (var writer = new StreamWriter(stream) { AutoFlush = true })
            {
                try
                {
                    var request = JsonNode.Parse((await reader.ReadLineAsync(_stop.Token))!)!.AsObject();
                    string op = request["op"]!.GetValue<string>();
                    _calls.Enqueue(new WorkerCall(op, request["reservation_id"]?.GetValue<string>()));
                    if (op == "reserve" && Interlocked.Decrement(ref _silentReserves) >= 0)
                    {
                        // Hold the reserve unanswered until Medius gives up and closes.
                        await reader.ReadLineAsync(_stop.Token);
                        return;
                    }
                    var response = new JsonObject {
                        ["v"] = 2, ["request_id"] = request["request_id"]!.DeepClone(), ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray()
                    };
                    if (op == "hello") response["capabilities"] = new JsonArray("committed_loadout_selection_v1", "active_loadout_refresh_v1");
                    await writer.WriteLineAsync(response.ToJsonString().AsMemory(), _stop.Token);
                }
                catch (OperationCanceledException) { }
                catch (IOException) { }
            }
        }

        public async ValueTask DisposeAsync()
        {
            _stop.Cancel();
            try { await _loop.WaitAsync(AdmissionTimeout); }
            finally { _listener.Stop(); _stop.Dispose(); }
        }
    }
}
