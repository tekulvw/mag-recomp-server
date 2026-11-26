using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using DotNetty.Common.Internal.Logging;
using RT.Models;
using RT.Models.MAG;
using DmeClient = Server.Dme.Models.ClientObject;
using DmeProgram = Server.Dme.Program;

namespace Server.Medius
{
    public sealed record MagRegisteredWorld(string WorkerId, string IncarnationId, int? WorldId,
        bool Ready, bool Draining, int ReservedPlayers, int ForwardedQueue);

    /// <summary>One control plane, persistence authority and transport listener; independent live worlds.</summary>
    public sealed partial class MagWorldRegistry : Server.Dme.ILocalGameplaySink, IMagGameplayLobby, IAsyncDisposable
    {
        sealed class Slot
        {
            public MagConfiguredWorker Config;
            public GameplayBridge Bridge;
            public LocalDmeHost Host;
            public volatile bool Faulted;
            // Set once this slot's incarnation was written off; a replacement slot takes its index.
            public volatile bool WrittenOff;
            // Its host never started (for example its world stream was refused), so it admitted
            // no one: the same incarnation may be started again without a write-off.
            public volatile bool StartupFailed;
        }
        static readonly IInternalLogger Logger = InternalLoggerFactory.GetInstance<MagWorldRegistry>();
        readonly MagWorldRegistryConfig _config;
        readonly LocalCharacterStore _store;
        readonly string _configPath;
        readonly Slot[] _slots;
        readonly Server.Dme.MediusManager _manager;
        readonly MagDmePortBudget _portBudget;
        readonly object _gate = new();
        IReadOnlySet<string> _drainingWorkers;
        readonly System.Threading.SemaphoreSlim _progressionGate = new(1,1);
        bool _initialized, _transportStarted, _disposed;
        Server.Dme.TcpServer _ownedTransport;
        DateTime _nextConfigRead;
        string _lastConfigFailure;

        /// <param name="portBudget">DME humans across every world; null is unbounded.</param>
        public MagWorldRegistry(MagWorldRegistryConfig config, LocalCharacterStore store, string configPath = null,
            MagDmePortBudget portBudget = null)
        {
            _config = config ?? throw new ArgumentNullException(nameof(config));
            _store = store ?? throw new ArgumentNullException(nameof(store));
            _configPath = configPath;
            _portBudget = portBudget ?? MagDmePortBudget.Unbounded();
            if (config.PartyInviteSeconds is int seconds) _parties = new MagPartyDirectory(TimeSpan.FromSeconds(seconds));
            _drainingWorkers = config.Workers.Where(w=>w.Draining).Select(w=>w.WorkerId).ToHashSet(StringComparer.Ordinal);
            _manager = new Server.Dme.MediusManager(config.ApplicationId) { LocalOnly = true };
            _slots = config.Workers.Select(row => CreateSlot(row, new GameplayBridge(row.WorkerHost, row.WorkerPort, row.WorkerId))).ToArray();
            _entries = config.Workers.Select(row => new AdoptionEntry(row.WorkerId)).ToArray();
        }
        Slot CreateSlot(MagConfiguredWorker row, GameplayBridge bridge)
        {
            // A row maximum above 64 admits worlds that large; humans stay within 64, and each
            // large profile's own human caps (32, 16 per side) apply beneath that.
            var options = new LocalDmeHostOptions(_config.ApplicationId, 0, row.MinimumPlayers,
                Math.Min(row.MaximumPlayers, MagWorldCapacity.LegacyWorldCharacters),
                row.MinimumPlayersPerTeam.ToArray(), new[] { 0, 1, 2 }, _config.ClientAddress, _config.ClientTcpPort, row.WorkerId,
                worldCapacity: row.MaximumPlayers);
            var host = new LocalDmeHost(bridge, _store, options, progressionGate:_progressionGate, portBudget:_portBudget);
            host.DurableProgressionCommitted += RefreshProgressionOwners;
            host.SetDrainPolicy(()=>System.Threading.Volatile.Read(ref _drainingWorkers).Contains(row.WorkerId));
            return new Slot { Config = row, Bridge = bridge, Host = host };
        }
        void RefreshProgressionOwners(string account,uint character) {
            foreach(var slot in _slots) slot.Host.QueueProgressionRefresh(account,character);
        }
        public Task<MagJoinGameResponse> JoinAsync(MagMlsSession session, MagJoinGameRequest request, Action<MagCharacterRecord> notify=null) =>
            Task.FromResult(JoinResponse(request.TransactionId,2));
        public bool UsesFactionMatchmaking => true;
        /// <summary>DME UDP ports shared by every configured world.</summary>
        public MagDmePortBudget PortBudget => _portBudget;
        public bool SupportsLoadoutLayoutChanges => _slots.All(slot => slot.Host.SupportsLoadoutLayoutChanges);
        public IReadOnlyList<MagRegisteredWorld> Worlds => Array.AsReadOnly(_slots.Select(slot => {
            var state = slot.Host.GetPlacement();
            return new MagRegisteredWorld(slot.Config.WorkerId, slot.Bridge.WorkerIdentity?.IncarnationId,
                slot.WrittenOff || slot.Host.World?.Destroyed != false ? null : slot.Host.World.WorldId, state.Ready && !slot.Faulted, state.Draining || slot.Faulted, state.LiveCount, state.PendingQueueCount);
        }).ToArray());

        /// <summary>Each configured world for the server status: admission state, running match and
        /// counts. A worker fault (a failed slot, a written-off incarnation or an identity fault) is
        /// reported as faulted rather than draining.</summary>
        public IReadOnlyList<MagWorldStatus> StatusWorlds() => Array.AsReadOnly(_slots.Select(slot =>
            MagServerStatus.Describe(slot.Config.WorkerId, slot.Faulted || slot.WrittenOff || slot.Bridge.HasWorkerIdentityFault,
                slot.Host.GetPlacement(), _portBudget.Capacity)).ToArray());

        /// <summary>Every configured world's current host, for the operator kill (docs/admin-cli.md).</summary>
        public IReadOnlyList<MagAdminGameplayHost> AdminGameplayHosts() =>
            Array.AsReadOnly(_slots.Where(slot => !slot.WrittenOff).Select(slot => new MagAdminGameplayHost(slot.Config.WorkerId, slot.Host)).ToArray());

        /// <summary>Humans waiting in the global queue for a world, by requested game type (0 to 2).</summary>
        public int[] QueuedByGameType()
        {
            var counts = new int[3];
            QueueEntry[] waiting;
            lock (_gate) waiting = _humans.Snapshot().Select(e => e.Value).Where(e => e.Assigned == null && e.Alive).ToArray();
            foreach (var entry in waiting)
                if (entry.Request.Value < (uint)counts.Length) counts[entry.Request.Value] += entry.Party?.Length ?? 1;
            return counts;
        }

        /// <summary>Initialize worker worlds without opening the shared DME listener.</summary>
        public async Task InitializeAsync()
        {
            lock (_gate) { if (_initialized || _disposed) throw new InvalidOperationException("Registry already initialized or disposed."); _initialized = true; }
            // World IDs in configuration order, before the concurrent worker handshakes.
            foreach (var slot in _slots) slot.Host.ReserveRegistryWorld(_manager);
            await Task.WhenAll(_slots.Select(InitializeSlotAsync));
            for (int i = 0; i < _slots.Length; i++)
                if (!_slots[i].Faulted && _slots[i].Host.WorkerContract is MagWorkerContract contract) _entries[i].Contract = contract;
            if (!_slots.Any(s => s.Host.GetPlacement().Ready))
            { await DisposeAsync(); throw new IOException("No configured gameplay worker is ready."); }
            if (PartyMembershipEnabled && !_slots.Any(s => !s.Faulted && s.Host.GetPlacement() is { Ready: true } placement &&
                placement.Profiles.Any(profile => profile.SquadCounts != null)))
            { await DisposeAsync(); throw new IOException("Party matchmaking requires an opted-in worker with authored squad capacity."); }
        }
        async Task InitializeSlotAsync(Slot slot)
        {
            try { await slot.Host.StartInRegistryAsync(_manager); }
            catch (Exception error) when (error is IOException || error is ArgumentException || error is InvalidOperationException)
            {
                slot.StartupFailed = true; slot.Faulted = true;
                Logger.Warn($"Configured worker unavailable: worker={slot.Config.WorkerId} error={error.GetType().Name} reason=\"{error.Message}\"");
            }
        }
        public async Task StartAsync()
        {
            await InitializeAsync();
            if (DmeProgram.LocalGameplay != null || DmeProgram.TcpServer.IsRunning || DmeProgram.Managers.ContainsKey(_config.ApplicationId))
            { await DisposeAsync(); throw new IOException("Integrated DME ownership is already assigned."); }
            DmeProgram.SERVER_IP = _config.ClientAddress;
            DmeProgram.Settings.TCPPort = 10073; DmeProgram.Settings.UDPPort = MagDmePortBudget.FirstPort;
            DmeProgram.Settings.UDPPortLast = _portBudget.LastPort ?? 0;
            Logger.Info($"DME UDP port capacity for {_slots.Length} worlds: {_portBudget.Describe()}");
            DmeProgram.Plugins = Program.Plugins ?? new Server.Plugins.PluginsManager("plugins/");
            DmeProgram.Managers.Add(_config.ApplicationId, _manager);
            DmeProgram.LocalGameplay = this;
            _ownedTransport=DmeProgram.TcpServer;
            try { await _ownedTransport.StartAsync(); _transportStarted = true; }
            catch { await DisposeAsync(); throw; }
        }
        public async Task TickAsync()
        {
            lock (_gate) if (_disposed || !_initialized) return;
            ReloadDrainPolicy();
            if (_transportStarted) await _ownedTransport.HandleIncomingMessages();
            await Task.WhenAll(_slots.Where(s => !s.Faulted).Select(TickSlotAsync));
            SuperviseWorkers();
            PrunePartySessions();
            SendQueueNotices(); StartPlacement();
            if (_transportStarted) {
                await _ownedTransport.HandleOutgoingMessages();
                foreach(var slot in _slots.Where(s=>!s.Faulted)) slot.Host.FlushWorldOutput();
            }
        }
        async Task TickSlotAsync(Slot slot)
        {
            try { await slot.Host.TickWorldAsync(flushOutput:!_transportStarted); }
            catch (Exception error)
            { slot.Faulted = true; slot.Host.SetDraining(true); Logger.Error($"Configured worker tick failed: worker={slot.Config.WorkerId} error={error.GetType().Name}"); }
        }
        void ReloadDrainPolicy()
        {
            if (_configPath == null || DateTime.UtcNow < _nextConfigRead) return;
            _nextConfigRead = DateTime.UtcNow.AddSeconds(1);
            try { ApplyDrainPolicy(MagWorldRegistryConfig.Load(_configPath)); _lastConfigFailure = null; }
            catch (Exception error) when (error is IOException || error is ArgumentException || error is UnauthorizedAccessException)
            {
                if (_lastConfigFailure != error.GetType().Name) Logger.Warn($"Registry drain reload refused: error={error.GetType().Name}");
                _lastConfigFailure = error.GetType().Name;
            }
        }
        public void ApplyDrainPolicy(MagWorldRegistryConfig next)
        {
            _config.ValidateDrainReload(next);
            IReadOnlySet<string> policy=next.Workers.Where(w=>w.Draining).Select(w=>w.WorkerId).ToHashSet(StringComparer.Ordinal);
            System.Threading.Volatile.Write(ref _drainingWorkers,policy);
        }
        // A disposed host's world ID is freed and may be issued again to an adopted world.
        Slot Owner(DmeClient client) => client?.DmeWorld == null ? null :
            _slots.SingleOrDefault(s => !s.WrittenOff && s.Host.World is { Destroyed: false } world && world.WorldId == client.DmeWorld.WorldId);
        public bool TryBind(DmeClient client, RT_MSG_CLIENT_CONNECT_TCP_AUX_UDP connect) => Owner(client)?.Host.TryBind(client, connect) == true;
        public bool CanUse(DmeClient client) => Owner(client)?.Host.CanUse(client) == true;
        public void Connected(DmeClient client) => Owner(client)?.Host.Connected(client);
        public void Event(DmeClient client, string transport, byte rtType, byte? pluginId, byte[] payload) =>
            Owner(client)?.Host.Event(client, transport, rtType, pluginId, payload);
        public void ArmoryLoadoutCommitted(string account, uint character)
        { foreach (var slot in _slots) slot.Host.ArmoryLoadoutCommitted(account, character); }

        public MagServerStatusResponse Status(int transaction)
        {
            var result = new MagServerStatusResponse { TransactionId = transaction, ContractOwners = Enumerable.Repeat((byte)255, 9).ToArray(),
                QueueEnabled = new bool[12], QueueSizes = new uint[12], PlayersInGame = new uint[12], MinimumPlayers = new uint[4] };
            foreach (var slot in _slots)
            {
                var placement = slot.Host.GetPlacement(); var status = slot.Host.Status(transaction);
                for (int i = 0; i < 12; i++)
                {
                    result.QueueSizes[i] += status.QueueSizes[i]; result.PlayersInGame[i] += status.PlayersInGame[i];
                    if (!slot.Faulted && placement.Ready && !placement.Draining && placement.Profiles.Any(p => p.GameType == i / 3 && (p.AttackingFaction == i % 3 || p.DefendingFaction == i % 3)))
                    {
                        result.QueueEnabled[i] = true;
                        uint minimum = (uint)placement.StartupPolicy.MinimumPlayers;
                        result.MinimumPlayers[i / 3] = result.MinimumPlayers[i / 3] == 0 ? minimum : Math.Min(result.MinimumPlayers[i / 3], minimum);
                    }
                }
            }
            QueueEntry[] waiting; lock (_gate) waiting = _humans.Snapshot().Select(e => e.Value).Where(e => e.Assigned == null && e.Alive).ToArray();
            foreach (var entry in waiting) result.QueueSizes[(int)entry.Request.Value * 3 + entry.Faction] += (uint)(entry.Party?.Length ?? 1);
            return result;
        }
        public async ValueTask DisposeAsync()
        {
            Task[] pending;
            lock (_gate)
            {
                if (_disposed) return; _disposed = true;
                var tickets=_humans.Snapshot();
                pending=tickets.Select(e=>e.Value.Preparation).Where(t=>t!=null).Append(_placementWorker)
                    .Concat(_entries.Select(e=>e.Supervision)).ToArray();
                foreach(var entry in tickets) {System.Threading.Volatile.Write(ref entry.Value.Cancelled,1);_humans.Remove(entry);}
                foreach(var entry in _acknowledged.Values)System.Threading.Volatile.Write(ref entry.Cancelled,1);
                _acknowledged.Clear();
            }
            try {
                try {await Task.WhenAll(pending).WaitAsync(TimeSpan.FromSeconds(5));}
                catch(TimeoutException) {Logger.Warn("Registry placement shutdown exceeded its bounded deadline.");}
            } finally {
                try {await Task.WhenAll(_slots.Select(async slot=>await slot.Host.DisposeAsync()));}
                finally {
                    try {if(_ownedTransport!=null)await _ownedTransport.Stop();}
                    finally {
                        if(ReferenceEquals(DmeProgram.LocalGameplay,this))DmeProgram.LocalGameplay=null;
                        if(DmeProgram.Managers.TryGetValue(_config.ApplicationId,out var manager) && ReferenceEquals(manager,_manager))DmeProgram.Managers.Remove(_config.ApplicationId);
                    }
                }
            }
        }
    }
}
