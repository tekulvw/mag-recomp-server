using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    /// <summary>A live incident, replayed with an owned Rust worker: the worker keeps running while
    /// the Horizon (Medius) process that admitted its players dies without a goodbye, and a new
    /// Horizon process starts. Each Horizon process reaches the worker through its own relay, so
    /// severing the relay is that process dying: its connections drop and nothing it sends later
    /// arrives. The new process must open the world at once, with no stream refusal, loop or
    /// fault-close, and form a new match there, whether its world ID differs from the old one
    /// (the old process had reached higher IDs) or is the same (a fresh process counts from the
    /// same start).</summary>
    public static void RealRegistryMediusRestart(string binary, string catalogs) => WithDirectory(directory =>
    {
        var settings = Server.Medius.Program.Settings;
        int previous = settings.MagApplicationId;
        settings.MagApplicationId = 20231;
        try { VerifyAsync().GetAwaiter().GetResult(); }
        finally { settings.MagApplicationId = previous; }

        async Task VerifyAsync()
        {
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1), ("charlie", (byte)0), ("delta", (byte)1),
                ("echo", (byte)0), ("foxtrot", (byte)1), ("golf", (byte)0), ("hotel", (byte)1) })
                Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] },
                    account).Result == MagNetResult.Success, "Synthetic character creation failed");

            await using var worker = await RealRegistryWorkerChecks.Worker.StartAsync("survivor", directory, binary, catalogs);
            int transaction = 0;
            async Task<MagJoinGameResponse[]> Admit(MagWorldRegistry registry, params string[] accounts)
            {
                var queued = accounts.Select(account => QueueRegistryJoin(registry, store, account, ++transaction, mode: 0)).ToArray();
                await TickUntil(registry, () => queued.All(t => t.IsCompleted),
                    $"A real worker did not admit {string.Join(",", accounts)} ({Holding(registry)})");
                return await Task.WhenAll(queued);
            }
            string Holding(MagWorldRegistry registry)
            {
                var world = World(registry, worker.Id);
                return $"world={world.WorldId} ready={world.Ready} draining={world.Draining} reserved={world.ReservedPlayers} " +
                    $"journal_held={HeldReservations(worker).Count} rebinds={Rebinds(worker).Count}";
            }
            async Task<int?> AdmitThenDie(Relay link, params string[] accounts)
            {
                var registry = AdoptionRegistry(store, null, AdoptionWorkerRow(worker.Id, link.Port));
                try
                {
                    await registry.InitializeAsync();
                    var replies = await Admit(registry, accounts);
                    int? world = World(registry, worker.Id).WorldId;
                    Check(replies.All(r => r.Result == 0 && r.WorldId == world), "The first Horizon process did not form its match");
                    await TickUntil(registry, () => HeldReservations(worker).Count == accounts.Length,
                        "The real worker did not record the first Horizon process's match");
                    // The process dies: its connections drop and it never says goodbye.
                    link.Sever();
                    return world;
                }
                finally { await registry.DisposeAsync(); }
            }
            async Task ReopenAfterDeath(int? oldWorld, bool sameWorldId, string label, params string[] accounts)
            {
                int rebindsBefore = Rebinds(worker).Count;
                await using var registry = AdoptionRegistry(store, null, AdoptionWorkerRow(worker.Id, worker.Port));
                await registry.InitializeAsync();
                var world = World(registry, worker.Id);
                Check(world.Ready && !world.Draining && (world.WorldId == oldWorld) == sameWorldId,
                    $"{label}: the new Horizon process did not open the surviving worker's world ({Holding(registry)}, old world {oldWorld})");
                var rebinds = Rebinds(worker);
                Check(rebinds.Count == rebindsBefore + 1 && rebinds[^1]["reservation_ids"]!.AsArray().Count == 2 &&
                    rebinds[^1]["previous_world_ids"]!.AsArray().Select(id => id!.GetValue<int>()).SequenceEqual(new[] { oldWorld!.Value }) &&
                    rebinds[^1]["world_id"]!.GetValue<int>() == world.WorldId && HeldReservations(worker).Count == 0,
                    $"{label}: the worker did not release the dead process's admissions when the new one subscribed");
                // The stream holds: no refusal loop, fault-close, write-off or adoption.
                await TickFor(registry, TimeSpan.FromSeconds(3));
                var counters = Counters(registry, worker.Id);
                Check(World(registry, worker.Id) is { Ready: true, Draining: false } && counters.Adoptions == 0 && counters.WriteOffs == 0 &&
                    Rebinds(worker).Count == rebindsBefore + 1,
                    $"{label}: the surviving worker's world did not stay open ({Holding(registry)})");
                var replies = await Admit(registry, accounts);
                Check(replies.All(r => r.Result == 0 && r.WorldId == world.WorldId) &&
                    HeldReservations(worker) is { Count: 2 } held && held.All(row => row["world_id"]!.GetValue<int>() == world.WorldId),
                    $"{label}: the new Horizon process could not form a match on the surviving worker");
            }

            // 1. Another world ID: the dead process's world still holds its ID in this test
            //    process, as a live Medius that had adopted workers held higher IDs.
            await using (var link = await Relay.StartAsync(worker.Port))
            {
                int? oldWorld = await AdmitThenDie(link, "alpha", "bravo");
                await ReopenAfterDeath(oldWorld, false, "another world ID", "charlie", "delta");
            }

            // 2. The same world ID: a fresh process counts world IDs from where the dead one began.
            await using (var link = await Relay.StartAsync(worker.Port))
            {
                int next = NextWorldId();
                int? oldWorld = await AdmitThenDie(link, "echo", "foxtrot");
                Check(oldWorld == next, "The first Horizon process's world did not take the next world ID");
                SetNextWorldId(next);
                await ReopenAfterDeath(oldWorld, true, "the same world ID", "golf", "hotel");
            }
        }
    });

    static readonly FieldInfo WorldIdCounter = typeof(Server.Dme.Models.World).GetField("_idCounter", BindingFlags.Static | BindingFlags.NonPublic)!;
    static int NextWorldId() => (int)WorldIdCounter.GetValue(null)!;
    static void SetNextWorldId(int value) => WorldIdCounter.SetValue(null, value);

    static IReadOnlyList<JsonObject> Rebinds(RealRegistryWorkerChecks.Worker worker) =>
        worker.ReadJournal().Where(row => row["op"]?.GetValue<string>() == "control_plane_rebound").ToArray();

    /// <summary>Journal reservations the worker still holds: not released, expired or rebound away.</summary>
    static IReadOnlyList<JsonObject> HeldReservations(RealRegistryWorkerChecks.Worker worker)
    {
        var rows = worker.ReadJournal();
        var gone = rows.Where(row => row["op"]?.GetValue<string>() is string op && (op.StartsWith("released", StringComparison.Ordinal) || op == "expired"))
            .Select(row => row["reservation_id"]?.GetValue<string>()).Where(id => id != null).ToHashSet(StringComparer.Ordinal);
        return rows.Where(row => row["op"]?.GetValue<string>() == "reserved" && !gone.Contains(row["reservation_id"]!.GetValue<string>())).ToArray();
    }

    /// <summary>A loopback TCP relay standing in for one Horizon process's network path.</summary>
    sealed class Relay : IAsyncDisposable
    {
        readonly TcpListener _listener = new(IPAddress.Loopback, 0);
        readonly CancellationTokenSource _stop = new();
        readonly List<TcpClient> _open = new();
        readonly int _target;
        Task _loop = Task.CompletedTask;
        public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;

        Relay(int target) => _target = target;

        public static Task<Relay> StartAsync(int target)
        {
            var relay = new Relay(target);
            relay._listener.Start();
            relay._loop = relay.AcceptAsync();
            return Task.FromResult(relay);
        }

        async Task AcceptAsync()
        {
            var pumps = new List<Task>();
            try
            {
                while (true)
                {
                    var client = await _listener.AcceptTcpClientAsync(_stop.Token);
                    var upstream = new TcpClient();
                    try { await upstream.ConnectAsync(IPAddress.Loopback, _target, _stop.Token); }
                    catch (Exception error) when (error is SocketException || error is OperationCanceledException)
                    { client.Dispose(); upstream.Dispose(); continue; }
                    lock (_open) { _open.Add(client); _open.Add(upstream); }
                    pumps.Add(PumpAsync(client, upstream));
                }
            }
            catch (Exception error) when (error is OperationCanceledException || error is ObjectDisposedException || error is SocketException) { }
            await Task.WhenAll(pumps);
        }

        async Task PumpAsync(TcpClient client, TcpClient upstream)
        {
            try
            {
                var inbound = client.GetStream(); var outbound = upstream.GetStream();
                await Task.WhenAny(inbound.CopyToAsync(outbound, _stop.Token), outbound.CopyToAsync(inbound, _stop.Token));
            }
            catch (Exception error) when (error is IOException || error is ObjectDisposedException || error is OperationCanceledException || error is InvalidOperationException) { }
            finally { client.Dispose(); upstream.Dispose(); }
        }

        /// <summary>The Horizon process behind this relay is gone: every connection drops at once
        /// and no new connection reaches the worker.</summary>
        public void Sever()
        {
            if (_stop.IsCancellationRequested) return;
            _stop.Cancel();
            _listener.Stop();
            lock (_open) foreach (var socket in _open) socket.Dispose();
        }

        public async ValueTask DisposeAsync()
        {
            Sever();
            await _loop;
            _stop.Dispose();
        }
    }
}
