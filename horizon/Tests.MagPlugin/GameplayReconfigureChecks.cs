using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// A legacy gameplay worker that restarts on its own comes back in its one-player
// default. Horizon must configure every worker incarnation before its world
// stream opens admission again, and must treat match_not_configured as a restart.
internal static partial class LeadershipProgressionChecks
{
    public static void LegacyWorkerRestartReconfigures() => WithDirectory(directory =>
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
            await using var gameplay = new RestartingGameplay();
            await using var host = new LocalDmeHost(gameplay.Client, store);
            var hello = await gameplay.Client.RequestHelloAsync(new { v = 2, request_id = "synthetic-hello", op = "hello" });
            typeof(LocalDmeHost).GetMethod("InitializeGameplayHello", HostInternals)!.Invoke(host, new object[] { hello });
            Check(!host.UsesFactionMatchmaking, "Synthetic worker unexpectedly enabled faction matchmaking");
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            typeof(LocalDmeHost).GetProperty(nameof(LocalDmeHost.World))!.SetValue(host,
                new Server.Dme.Models.World(manager, 20231, Server.Medius.Program.Settings.MagMaximumPlayers));
            typeof(LocalDmeHost).GetField("_ready", HostInternals)!.SetValue(host, true);

            // Without a startup configuration the stream itself configures first.
            gameplay.RefuseConfigurations(1);
            await (Task)typeof(LocalDmeHost).GetMethod("StartWorldStreamAsync", HostInternals)!.Invoke(host, null)!;
            Check(host.IsReady, "World stream did not open admission after configuring the worker");
            var log = gameplay.Log().Where(entry => entry.Op != "hello").ToList();
            int accepted = log.FindIndex(entry => entry.Op == "configure_match" && entry.Ok);
            int firstSubscribe = log.FindIndex(entry => entry.Op == "subscribe_world");
            Check(log[0].Op == "configure_match" && !log[0].Ok && accepted > 0 && firstSubscribe > accepted,
                "World stream subscribed before the worker accepted its match configuration");
            var configure = gameplay.Requests("configure_match").Last();
            Check(configure["maximum_players"]!.GetValue<int>() == Server.Medius.Program.Settings.MagMaximumPlayers &&
                configure["application_id"]!.GetValue<int>() == 20231, "Reconfiguration did not send the host's startup policy");

            var session = MakeSession(store, 20231, "alpha", 1);
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Configured worker refused the first admission");
            Check(gameplay.Count("configure_match", true) == 1, "A healthy stream reconfigured its worker");

            // The worker process restarts alone: the stream drops and the new
            // incarnation refuses its first two configurations.
            int before = gameplay.Log().Count;
            gameplay.Restart(refuseConfigurations: 2);
            await WaitUntil(() => gameplay.Count("configure_match", false) >= 3);
            Check(!host.IsReady, "Admission reopened before the restarted worker was configured");
            Check(!gameplay.Log().Skip(before).Any(entry => entry.Op == "subscribe_world"),
                "World stream resubscribed to an unconfigured worker");
            await WaitUntil(() => host.IsReady);
            var restarted = gameplay.Log().Skip(before).ToList();
            int reconfigured = restarted.FindIndex(entry => entry.Op == "configure_match" && entry.Ok);
            int resubscribed = restarted.FindIndex(entry => entry.Op == "subscribe_world");
            Check(reconfigured >= 0 && resubscribed > reconfigured,
                "Restarted worker was not configured again before its stream became healthy");
            Check(!restarted.Any(entry => entry.Op == "reserve" && entry.Ok && restarted.IndexOf(entry) < reconfigured),
                "A reservation was accepted before the restarted worker was configured");
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Reconfigured worker refused admission");
            Check(gameplay.Count("configure_match", true) == 2, "Restart did not produce exactly one more accepted configuration");

            // The worker restarts behind a still-open stream. Its refusal closes
            // admission, reconnects the stream and configures the worker again.
            await Release(host, gameplay.Requests("reserve").Last()).WaitAsync(AdmissionTimeout);
            gameplay.ForgetConfiguration();
            before = gameplay.Log().Count;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result != 0, "An unconfigured worker admitted a player");
            Check(gameplay.Log().Skip(before).Any(entry => entry.Op == "reserve" && entry.Error == "match_not_configured"),
                "Synthetic worker did not refuse the unconfigured reservation");
            await WaitUntil(() => gameplay.Count("configure_match", true) == 3 && host.IsReady);
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Worker was not usable after reconfiguration");
            var last = gameplay.Log().Skip(before).ToList();
            int refreshed = last.FindIndex(entry => entry.Op == "configure_match" && entry.Ok);
            Check(refreshed >= 0 && refreshed < last.FindIndex(entry => entry.Op == "reserve" && entry.Ok) &&
                refreshed < last.FindLastIndex(entry => entry.Op == "subscribe_world"), "Admission resumed before reconfiguration");
        }
    });

    private static async Task WaitUntil(Func<bool> condition)
    {
        var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(15);
        while (!condition())
        {
            if (DateTime.UtcNow > deadline) throw new TimeoutException("Synthetic gameplay restart did not settle");
            await Task.Delay(25);
        }
    }

    private sealed record GameplayCall(string Op, bool Ok, string Error);

    /// <summary>Loopback gameplay worker whose process can "restart": it forgets its
    /// configuration, closes world streams and refuses reserve until configured.</summary>
    private sealed class RestartingGameplay : IAsyncDisposable
    {
        private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        private readonly CancellationTokenSource _stop = new();
        private readonly object _gate = new();
        private readonly List<GameplayCall> _log = new();
        private readonly List<JsonObject> _requests = new();
        private readonly Task _loop;
        private CancellationTokenSource _incarnation = new();
        private bool _configured;
        private int _refuseConfigurations;
        public GameplayBridge Client { get; }

        public RestartingGameplay()
        {
            _listener.Start();
            Client = new GameplayBridge("127.0.0.1", ((IPEndPoint)_listener.LocalEndpoint).Port);
            _loop = RunAsync();
        }

        public List<GameplayCall> Log() { lock (_gate) return _log.ToList(); }
        public List<JsonObject> Requests(string op) { lock (_gate) return _requests.Where(r => r["op"]!.GetValue<string>() == op).ToList(); }
        public int Count(string op, bool ok) { lock (_gate) return _log.Count(entry => entry.Op == op && entry.Ok == ok); }
        public void RefuseConfigurations(int count) { lock (_gate) _refuseConfigurations = count; }
        public void ForgetConfiguration() { lock (_gate) _configured = false; }

        public void Restart(int refuseConfigurations)
        {
            CancellationTokenSource previous;
            lock (_gate)
            {
                _configured = false;
                _refuseConfigurations = refuseConfigurations;
                previous = _incarnation;
                _incarnation = new CancellationTokenSource();
            }
            previous.Cancel();
        }

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
                    var response = new JsonObject {
                        ["v"] = 2, ["request_id"] = request["request_id"]!.DeepClone(), ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray()
                    };
                    CancellationToken incarnation;
                    lock (_gate)
                    {
                        _requests.Add(request);
                        incarnation = _incarnation.Token;
                        string error = null;
                        if (op == "configure_match" && _refuseConfigurations > 0) { _refuseConfigurations--; error = "synthetic_configuration_unavailable"; }
                        else if (op == "configure_match") _configured = true;
                        else if (op == "reserve" && !_configured) error = "match_not_configured";
                        if (error != null) { response["ok"] = false; response["error"] = error; }
                        _log.Add(new GameplayCall(op, error == null, error));
                    }
                    if (op == "hello")
                        response["capabilities"] = new JsonArray("committed_loadout_selection_v1", "active_loadout_refresh_v1");
                    await writer.WriteLineAsync(response.ToJsonString().AsMemory(), _stop.Token);
                    if (op != "subscribe_world") return;
                    // Heartbeats until this worker incarnation ends; closing the
                    // socket is what Horizon observes when the process restarts.
                    using var ended = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token, incarnation);
                    while (true)
                    {
                        await Task.Delay(100, ended.Token);
                        response["ok"] = true; response["error"] = null;
                        await writer.WriteLineAsync(response.ToJsonString().AsMemory(), ended.Token);
                    }
                }
                catch (OperationCanceledException) { }
                catch (IOException) { } // Horizon closed this subscription.
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
