using System.Collections;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text.Json.Nodes;
using Server.Medius;

// world_stream_resume_v1 and event_replay_v1. A broken world stream to the same
// worker process keeps every player and replays the frames that were in flight;
// a restarted worker or a gap longer than the grace period still releases them.
// An unanswered event is sent again with its original request ID.
internal static partial class LeadershipProgressionChecks
{
    public static void WorldStreamResumeKeepsPlayers() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        var settings = Server.Medius.Program.Settings;
        int previousApp = settings.MagApplicationId;
        settings.MagApplicationId = 20231;
        try { Run().GetAwaiter().GetResult(); }
        finally { settings.MagApplicationId = previousApp; }

        async Task Run()
        {
            await using var gameplay = new ResumableGameplay();
            await using var host = new LocalDmeHost(gameplay.Client, store);
            var hello = await gameplay.Client.RequestHelloAsync(new { v = 2, request_id = "synthetic-hello", op = "hello" });
            typeof(LocalDmeHost).GetMethod("InitializeGameplayHello", HostInternals)!.Invoke(host, new object[] { hello });
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            typeof(LocalDmeHost).GetProperty(nameof(LocalDmeHost.World))!.SetValue(host,
                new Server.Dme.Models.World(manager, 20231, Server.Medius.Program.Settings.MagMaximumPlayers));
            typeof(LocalDmeHost).GetField("_ready", HostInternals)!.SetValue(host, true);
            typeof(LocalDmeHost).GetField("_worldStreamResumeGrace", HostInternals)!.SetValue(host, TimeSpan.FromSeconds(3));
            await (Task)typeof(LocalDmeHost).GetMethod("StartWorldStreamAsync", HostInternals)!.Invoke(host, null)!;
            Check(host.IsReady, "World stream did not open admission");
            var first = gameplay.Requests("subscribe_world").Single();
            Check(first["positions"]?.GetValue<bool>() == true && first["resume"] == null,
                "A resumable worker was not asked for frame positions");

            var session = MakeSession(store, 20231, "alpha", 1);
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Synthetic admission failed");
            Check(LiveReservations(host) == 1, "Admission did not hold a reservation");

            // 1. The stream breaks while two frames are still in flight. The same
            // process accepts the resume and replays exactly what was missed.
            await WaitUntil(() => Position(host) >= 3);
            int subscriptions = gameplay.Requests("subscribe_world").Count;
            gameplay.DropStream(lostFrames: 2);
            await WaitUntil(() => gameplay.Requests("subscribe_world").Count > subscriptions && host.IsReady);
            var resume = gameplay.Requests("subscribe_world").Last();
            Check(resume["resume"]?["epoch"]?.GetValue<string>() == gameplay.Epoch, "Resume named another worker process");
            ulong after = resume["resume"]!["after"]!.GetValue<ulong>();
            Check(after < gameplay.Sequence, "Resume did not ask for the frames lost in flight");
            await WaitUntil(() => Position(host) > after + 2);
            Check(gameplay.Count("release") == 0, "A resumed stream released its players");
            Check(LiveReservations(host) == 1, "A resumed stream lost its reservation");
            Check(gameplay.Replayed >= 2, "The lost frames were not replayed");

            // 2. Retried events keep their request ID and order.
            var reservation = OnlyReservation(host);
            gameplay.AnswerEventsAfterAttempts(2);
            await SendEvent(host, reservation, "0a");
            await WaitUntil(() => gameplay.EventAttempts().Count == 2);
            var attempts = gameplay.EventAttempts();
            Check(attempts.Select(a => a["request_id"]!.GetValue<string>()).Distinct().Count() == 1,
                "A retried event did not keep its request ID");
            Check(attempts.All(a => a["payload_hex"]!.GetValue<string>() == "0a"), "A retried event changed its payload");
            Check(LiveReservations(host) == 1, "A retried event released the player");
            // Without the worker's replay capability, an event is never repeated.
            typeof(LocalDmeHost).GetField("_supportsEventReplay", HostInternals)!.SetValue(host, false);
            gameplay.AnswerEventsAfterAttempts(2);
            await SendEvent(host, reservation, "0b");
            await WaitUntil(() => gameplay.EventAttempts().Count == 3);
            await Task.Delay(800);
            Check(gameplay.EventAttempts().Count == 3, "An event was repeated without event_replay_v1");

            // 3. A gap that outlasts the grace period releases the players.
            gameplay.RefuseSubscriptions(true);
            gameplay.DropStream(lostFrames: 0);
            await WaitUntil(() => gameplay.Count("release") == 1);
            Check(LiveReservations(host) == 0, "An unrecoverable gap kept its reservation");
            gameplay.RefuseSubscriptions(false);
            await WaitUntil(() => host.IsReady);
            Check(gameplay.Requests("subscribe_world").Last()["resume"] == null, "A released stream still asked to resume");

            // 4. A restarted worker cannot replay the stream: its players are released.
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Readmission failed");
            await WaitUntil(() => Position(host) >= 2);
            gameplay.Restart();
            await WaitUntil(() => gameplay.Count("release") == 2);
            Check(gameplay.Refusals >= 1, "The restarted worker was never asked to resume");
            Check(LiveReservations(host) == 0, "A restarted worker kept a reservation it cannot replay");
            await WaitUntil(() => host.IsReady);
        }
    });

    static ulong Position(LocalDmeHost host)
    {
        var position = (GameplayWorldStreamPosition)typeof(LocalDmeHost).GetField("_worldStreamPosition", HostInternals)!.GetValue(host);
        return position?.Sequence ?? 0;
    }

    static IEnumerable<object> Reservations(LocalDmeHost host) =>
        ((IDictionary)typeof(LocalDmeHost).GetField("_reservations", HostInternals)!.GetValue(host)!).Values.Cast<object>();

    static int LiveReservations(LocalDmeHost host) =>
        Reservations(host).Count(r => !(bool)r.GetType().GetField("Invalidated")!.GetValue(r)!);

    static object OnlyReservation(LocalDmeHost host) => Reservations(host).Single();

    /// <summary>Queue one client event for the host's ordered event worker.</summary>
    static Task SendEvent(LocalDmeHost host, object reservation, string payload)
    {
        var events = typeof(LocalDmeHost).GetField("_events", HostInternals)!.GetValue(host)!;
        var writer = events.GetType().GetProperty("Writer")!.GetValue(events)!;
        var message = new { v = 2, request_id = Guid.NewGuid().ToString("N"), op = "event", payload_hex = payload };
        var item = Activator.CreateInstance(typeof(ValueTuple<,>).MakeGenericType(reservation.GetType(), typeof(object)), reservation, (object)message)!;
        var written = (bool)writer.GetType().GetMethod("TryWrite")!.Invoke(writer, new[] { item })!;
        Check(written, "Synthetic event was not queued");
        return Task.CompletedTask;
    }

    /// <summary>Loopback worker with numbered world frames, a replay journal and
    /// event replies keyed by request ID. It can break its stream, refuse
    /// subscriptions, or restart as another process.</summary>
    private sealed class ResumableGameplay : IAsyncDisposable
    {
        private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        private readonly CancellationTokenSource _stop = new();
        private readonly SemaphoreSlim _stream = new(1, 1);
        private readonly object _gate = new();
        private readonly List<JsonObject> _requests = new();
        private readonly List<JsonObject> _events = new();
        private readonly List<(ulong Sequence, string Line)> _journal = new();
        private readonly Task _loop;
        private readonly Task _producer;
        private StreamWriter _subscriber;
        private CancellationTokenSource _subscription;
        private int _epochNumber = 1;
        private int _answerAfter = 1;
        private readonly Dictionary<string, int> _attempts = new();
        private bool _refuse;
        public GameplayBridge Client { get; }
        public string Epoch { get { lock (_gate) return $"synthetic-epoch-{_epochNumber:D4}"; } }
        public ulong Sequence { get; private set; }
        public int Replayed { get; private set; }
        public int Refusals { get; private set; }

        public ResumableGameplay()
        {
            _listener.Start();
            Client = new GameplayBridge("127.0.0.1", ((IPEndPoint)_listener.LocalEndpoint).Port);
            _loop = RunAsync();
            _producer = ProduceAsync();
        }

        public List<JsonObject> Requests(string op) { lock (_gate) return _requests.Where(r => r["op"]!.GetValue<string>() == op).ToList(); }
        public int Count(string op) => Requests(op).Count;
        public List<JsonObject> EventAttempts() { lock (_gate) return _events.ToList(); }
        public void AnswerEventsAfterAttempts(int attempts) { lock (_gate) _answerAfter = attempts; }
        public void RefuseSubscriptions(bool refuse) { lock (_gate) _refuse = refuse; }

        /// <summary>Close the stream. Frames produced since the last delivered one
        /// are journaled but never written, as if lost in flight.</summary>
        public void DropStream(int lostFrames)
        {
            _stream.Wait();
            try
            {
                for (int i = 0; i < lostFrames; i++) Journal();
                _subscription?.Cancel();
                _subscriber = null;
            }
            finally { _stream.Release(); }
        }

        public void Restart()
        {
            _stream.Wait();
            try
            {
                lock (_gate) { _epochNumber++; _attempts.Clear(); }
                Sequence = 0;
                _journal.Clear();
                _subscription?.Cancel();
                _subscriber = null;
            }
            finally { _stream.Release(); }
        }

        string Frame(string requestId, ulong? sequence) => new JsonObject
        {
            ["v"] = 2, ["request_id"] = requestId, ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray(),
            ["world_stream"] = new JsonObject { ["epoch"] = Epoch, ["sequence"] = sequence ?? Sequence }
        }.ToJsonString();

        // Called with _stream held.
        string Journal()
        {
            Sequence++;
            var line = Frame("journal", Sequence);
            _journal.Add((Sequence, line));
            return line;
        }

        private async Task ProduceAsync()
        {
            try
            {
                while (true)
                {
                    await Task.Delay(100, _stop.Token);
                    await _stream.WaitAsync(_stop.Token);
                    try
                    {
                        if (_subscriber == null) continue; // Frames are produced only while subscribed.
                        var line = Journal().Replace("\"journal\"", "\"" + _subscriberId + "\"");
                        try { await _subscriber.WriteLineAsync(line.AsMemory(), _stop.Token); }
                        catch (IOException) { _subscriber = null; }
                    }
                    finally { _stream.Release(); }
                }
            }
            catch (OperationCanceledException) { }
        }
        private string _subscriberId;

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
                    var request = JsonNode.Parse(await reader.ReadLineAsync(_stop.Token))!.AsObject();
                    string op = request["op"]!.GetValue<string>();
                    string id = request["request_id"]!.GetValue<string>();
                    var response = new JsonObject { ["v"] = 2, ["request_id"] = id, ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray() };
                    lock (_gate) _requests.Add(request);
                    if (op == "hello")
                        response["capabilities"] = new JsonArray("committed_loadout_selection_v1", "active_loadout_refresh_v1",
                            "world_stream_resume_v1", "event_replay_v1");
                    if (op == "event")
                    {
                        bool answer;
                        lock (_gate)
                        {
                            _events.Add(request);
                            _attempts[id] = _attempts.GetValueOrDefault(id) + 1;
                            answer = _attempts[id] >= _answerAfter;
                        }
                        if (!answer) return; // Close unanswered, like a lost reply.
                    }
                    if (op == "subscribe_world")
                    {
                        await SubscribeAsync(request, writer);
                        return;
                    }
                    await writer.WriteLineAsync(response.ToJsonString().AsMemory(), _stop.Token);
                }
                catch (OperationCanceledException) { }
                catch (IOException) { }
            }
        }

        private async Task SubscribeAsync(JsonObject request, StreamWriter writer)
        {
            string id = request["request_id"]!.GetValue<string>();
            CancellationTokenSource ended;
            await _stream.WaitAsync(_stop.Token);
            try
            {
                bool refuse; lock (_gate) refuse = _refuse;
                string error = refuse ? "world_subscription_exists" : null;
                var resume = request["resume"];
                List<string> replay = new();
                ulong acknowledged = Sequence;
                if (error == null && resume != null)
                {
                    ulong after = resume["after"]!.GetValue<ulong>();
                    if (resume["epoch"]!.GetValue<string>() != Epoch || after > Sequence ||
                        (after < Sequence && (_journal.Count == 0 || _journal[0].Sequence > after + 1)))
                    { error = "world_resume_unavailable"; Refusals++; }
                    else
                    {
                        acknowledged = after;
                        replay = _journal.Where(f => f.Sequence > after).Select(f => f.Line.Replace("\"journal\"", "\"" + id + "\"")).ToList();
                    }
                }
                if (error != null)
                {
                    var refusal = new JsonObject { ["v"] = 2, ["request_id"] = id, ["ok"] = false, ["error"] = error, ["messages"] = new JsonArray() };
                    await writer.WriteLineAsync(refusal.ToJsonString().AsMemory(), _stop.Token);
                    return;
                }
                await writer.WriteLineAsync(Frame(id, acknowledged).AsMemory(), _stop.Token);
                foreach (var line in replay) await writer.WriteLineAsync(line.AsMemory(), _stop.Token);
                Replayed += replay.Count;
                _subscription = ended = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token);
                _subscriber = writer;
                _subscriberId = id;
            }
            finally { _stream.Release(); }
            // Hold the connection until the stream is dropped or the worker stops.
            try { await Task.Delay(Timeout.Infinite, ended.Token); }
            catch (OperationCanceledException) { }
            await _stream.WaitAsync();
            try { if (_subscriber == writer) _subscriber = null; }
            finally { _stream.Release(); }
        }

        public async ValueTask DisposeAsync()
        {
            _stop.Cancel();
            try { await Task.WhenAll(_loop, _producer).WaitAsync(AdmissionTimeout); }
            finally { _listener.Stop(); _stop.Dispose(); }
        }
    }
}
