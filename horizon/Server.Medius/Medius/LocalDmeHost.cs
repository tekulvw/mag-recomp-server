using System;
using System.Diagnostics;
using DotNetty.Common.Internal.Logging;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Text;
using System.Threading;
using System.Threading.Channels;
using System.Threading.Tasks;
using RT.Common;
using RT.Models;
using RT.Models.MAG;
using DmeClient = Server.Dme.Models.ClientObject;
using DmeProgram = Server.Dme.Program;
namespace Server.Medius
{
    /// <summary>One shared local Horizon world; each member has a Rust-acknowledged reservation.</summary>
    public sealed class LocalDmeHost : Server.Dme.ILocalGameplaySink, IAsyncDisposable
    {
        sealed class Reservation { public string Id; public uint AccountId; public string Account; public uint CharacterId; public Action<MagCharacterRecord> Notify; public DmeClient Client; public DateTime Deadline; public byte Faction; public bool Accepted; public bool Claimed; public bool Invalidated; public string ReleaseReason; public string TerminalMatchId; }
        sealed class PendingRelease { public Reservation Reservation; public string RequestId; public DateTime NextAttempt; }
        static readonly IInternalLogger Logger = InternalLoggerFactory.GetInstance<LocalDmeHost>();
        readonly int _applicationId;
        readonly int _minimumPlayers;
        readonly MagMatchStartupPolicy _startupPolicy;
        readonly int _gameType;
        readonly int _maximumPlayers;
        readonly HashSet<byte> _allowedFactions;
        readonly GameplayBridge _bridge;
        readonly LocalCharacterStore _store;
        readonly object _gate = new object();
        readonly SemaphoreSlim _admission = new SemaphoreSlim(1, 1);
        readonly Dictionary<string, Reservation> _reservations = new Dictionary<string, Reservation>(StringComparer.Ordinal);
        readonly Dictionary<string, PendingRelease> _pendingReleases = new Dictionary<string, PendingRelease>(StringComparer.Ordinal);
        readonly SemaphoreSlim _releaseSettlement = new SemaphoreSlim(1, 1);
        // Keep admission identities after transport release until pending rewards settle.
        // Bound them rather than recycling a reservation identity into a new actor.
        readonly Dictionary<string, Reservation> _rewardOwners = new Dictionary<string, Reservation>(StringComparer.Ordinal);
        readonly HashSet<string> _notifiedRewards = new HashSet<string>(StringComparer.Ordinal);
        readonly SemaphoreSlim _rewardSettlement = new SemaphoreSlim(1, 1);
        bool _rewardRetry;
        DateTime _nextRewardPoll;
        readonly Channel<(Reservation reservation, object message)> _events = Channel.CreateBounded<(Reservation, object)>(new BoundedChannelOptions(128) { SingleReader = true, FullMode = BoundedChannelFullMode.Wait });
        readonly Task _worker;
        readonly CancellationTokenSource _worldStreamCancellation = new CancellationTokenSource();
        readonly TaskCompletionSource<bool> _firstWorldStream = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        Task _worldStreamWorker = Task.CompletedTask;
        bool _worldStreamHealthy;
        bool _ready; bool _started; bool _disposed;
        public Server.Dme.Models.World World { get; private set; }
        public bool IsReady { get { lock (_gate) return _ready && _worldStreamHealthy && !_disposed; } }
        public LocalDmeHost(GameplayBridge bridge, LocalCharacterStore store)
        {
            _applicationId = Program.Settings.MagApplicationId;
            _startupPolicy = new MagMatchStartupPolicy(Program.Settings.MagMinimumPlayers, Program.Settings.MagMaximumPlayers, Program.Settings.MagMinimumPlayersPerTeam);
            _minimumPlayers = _startupPolicy.MinimumPlayers;
            _gameType = Program.Settings.MagGameType;
            if (_gameType != 2 && _gameType != 3 && !(_applicationId == 20231 && _gameType == 0)) throw new ArgumentException("Local gameplay serves types2/3, plus original retail Sabotage type0.");
            _maximumPlayers = Program.Settings.MagMaximumPlayers;
            int supportedMaximum = _applicationId == 20231 && (_gameType == 0 || _gameType == 3) ? 64 : 8;
            if (_minimumPlayers < 1 || _maximumPlayers > supportedMaximum || _minimumPlayers > _maximumPlayers) throw new ArgumentException($"Local match limits must be 1..{supportedMaximum}; 64-player capacity requires retail Sabotage or Suppression.");
            var configuredFactions = Program.Settings.MagAllowedFactions ?? new[] { 0, 1, 2 };
            if (configuredFactions.Length == 0 || configuredFactions.Any(faction => faction < 0 || faction > 2) || configuredFactions.Distinct().Count() != configuredFactions.Length)
                throw new ArgumentException("MagAllowedFactions requires distinct faction values0..2, or null for legacy availability.");
            _allowedFactions = new HashSet<byte>(configuredFactions.Select(faction => (byte)faction));
            _bridge = bridge; _store = store; _worker = RunEvents();
        }
        static string Id() => Guid.NewGuid().ToString("N");
        public async Task StartAsync()
        {
            if (_started) throw new InvalidOperationException("DME already started."); _started = true;
            if (!await _bridge.RequestAsync(new { v = 2, request_id = Id(), op = "hello" })) throw new IOException("Gameplay service is not ready.");
            if (!await _bridge.RequestAsync(_startupPolicy.ConfigureRequest(Id(), _applicationId))) throw new IOException("Gameplay match configuration was not acknowledged.");
            DmeProgram.SERVER_IP = MagNetworkSettings.FromEnvironment().Address; DmeProgram.Settings.TCPPort = 10073; DmeProgram.Settings.UDPPort = 10474;
            DmeProgram.Plugins = Program.Plugins ?? new Server.Plugins.PluginsManager("plugins/");
            var manager = new Server.Dme.MediusManager(_applicationId) { LocalOnly = true }; DmeProgram.Managers.Add(_applicationId, manager);
            World = new Server.Dme.Models.World(manager, _applicationId, _maximumPlayers); DmeProgram.LocalGameplay = this;
            try {
                _worldStreamWorker = RunWorldStreamAsync();
                await _firstWorldStream.Task.WaitAsync(TimeSpan.FromSeconds(5));
                await DmeProgram.TcpServer.StartAsync(); lock (_gate) _ready = true;
            }
            catch { await DisposeAsync(); throw; }
        }
        public MagServerStatusResponse Status(int transactionId)
        {
            lock (_gate)
            {
                var status = new MagServerStatusResponse
                {
                    TransactionId = transactionId, ContractOwners = Enumerable.Repeat((byte)255, 9).ToArray(),
                    MinimumPlayers = Enumerable.Repeat((uint)_minimumPlayers, 4).ToArray(), QueueEnabled = new bool[12]
                };
                if (_ready && _worldStreamHealthy && !_disposed)
                    foreach (var faction in _allowedFactions) status.QueueEnabled[_gameType * 3 + faction] = true;
                var now = DateTime.UtcNow;
                foreach (var reservation in _reservations.Values)
                {
                    var client = reservation.Client;
                    if (!reservation.Accepted || reservation.Invalidated || reservation.TerminalMatchId != null || reservation.Deadline <= now || reservation.Faction > 2 ||
                        client == null || client.IsDestroyed || client.Disconnected || World == null ||
                        !World.Clients.TryGetValue(client.DmeId, out var member) || member != client ||
                        (client.Tcp != null && !client.Tcp.Active)) continue;
                    // Native status matrices are game type first, then faction.
                    // These are authenticated world members, not physical spawns.
                    int index = _gameType * 3 + reservation.Faction;
                    if (reservation.Claimed && client.IsAuthenticated && client.Tcp?.Active == true) status.PlayersInGame[index]++;
                    else status.QueueSizes[index]++;
                }
                return status;
            }
        }
        public async Task<MagJoinGameResponse> JoinAsync(MagMlsSession session, MagJoinGameRequest request, Action<MagCharacterRecord> notify = null)
        {
            var failure = JoinResponse(request.TransactionId, 2);
            if ((request is MagRetailJoinGameRequest) != (_applicationId == 20231)) return failure;
            // Party membership/admission is not implemented. Do not silently turn a party request into a solo join.
            if (request is MagRetailJoinGameRequest retail && retail.PartyId != 0) return failure;
            if (session.ApplicationId != _applicationId || session.State != MagMlsSession.SessionState.Ready || session.IsExpired(DateTime.UtcNow) || session.AccountId == 0 || session.CharacterId == 0 || !MagDevelopmentAccountPolicy.ValidAccount(session.AuthenticatedAccount) || request.Preference != 1 || request.Value != (uint)_gameType || request.ClientType != 0 || !IsReady) return failure;
            await _admission.WaitAsync();
            try
            {
                Reservation completed;
                lock (_gate) completed = _reservations.Values.FirstOrDefault(r => r.Account == session.AuthenticatedAccount && r.CharacterId == session.CharacterId && r.TerminalMatchId != null);
                if (completed != null) { completed.ReleaseReason = "results_requeue"; await ReleaseAsync(completed); }
                PendingRelease pending;
                lock (_gate) pending = _pendingReleases.Values.FirstOrDefault(r => r.Reservation.Account == session.AuthenticatedAccount);
                if (pending != null) await RetryReleaseAsync(pending);
                // The local DME hosts one shared round. Keep results available to the
                // remaining cohort and wait for its releases before starting another.
                lock (_gate) if (_reservations.Values.Any(r => r.TerminalMatchId != null) || _pendingReleases.Values.Any(r => r.Reservation.TerminalMatchId != null || r.Reservation.Account == session.AuthenticatedAccount)) return JoinResponse(request.TransactionId, 6);
                lock (_gate) if (_rewardOwners.Count >= 4096 || _reservations.Count >= _maximumPlayers || _reservations.Values.Any(r => r.AccountId == session.AccountId)) return JoinResponse(request.TransactionId, 6);
                StoredCharacter character;
                try { character = _store.ReadForAccount(session.AuthenticatedAccount).SingleOrDefault(c => c.Id == session.CharacterId); }
                catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException) { return failure; }
                if (character == null) return failure;
                if (!_allowedFactions.Contains(character.Faction)) return JoinResponse(request.TransactionId, 4);
                var record = character.ToRecord();
                var allocation = await World.OnJoinGameRequest(new MediusServerJoinGameRequest { ConnectInfo = new NetConnectionInfo { SessionKey = session.SessionKey } });
                if (allocation.Confirmation != MGCL_ERROR_CODE.MGCL_SUCCESS || !World.Clients.TryGetValue(allocation.DmeClientIndex, out var client)) return failure;
                client.ApplicationId = _applicationId;
                var reservation = new Reservation { Id = Id(), AccountId = character.Id, Account = session.AuthenticatedAccount, CharacterId = character.Id, Faction = character.Faction, Notify = notify, Client = client, Deadline = DateTime.UtcNow.AddSeconds(30) };
                lock (_gate) { _reservations.Add(reservation.Id, reservation); _rewardOwners.Add(reservation.Id, reservation); }
                bool admitted = await _bridge.RequestAsync(new { v = 2, request_id = Id(), op = "reserve", reservation_id = reservation.Id,
                    world_id = (uint)World.WorldId, player_id = (ushort)client.DmeId, account_id = character.Id, session_key = client.SessionKey, access_key = client.Token,
                    character = new { leadership = character.Leadership == null ? null : new GameplayLeadershipRecord(character.Leadership), account_id = _store.ProgressionCatalog == null ? null : reservation.Account, progression = _store.ProgressionCatalog == null ? null : new { experience = record.Experience, level = record.Level, skill_points = record.SkillPoints }, skills = record.Skills, character_id = record.AccountId, name = record.Name, faction = record.Faction, head = record.HeadId, voice = record.VoiceId, universe_id = record.UniverseId,
                        loadouts = character.Loadouts == null ? null : new { selected_index = record.LastUsedLoadout,
                            update_ids = record.Loadouts.Select(l => (int)l.UpdateId).ToArray(),
                            inventories = record.Loadouts.Select(l => l.Items.Select(item => item < 0 ? (ushort?)null : (ushort)item).ToArray()).ToArray() } } });
                if (!admitted || !IsReady) { await ReleaseAsync(reservation); return failure; }
                lock (_gate) { if (!Current(reservation) || reservation.Invalidated || !_worldStreamHealthy) { reservation.Invalidated = true; reservation.ReleaseReason = "world_stream_gap"; return failure; } reservation.Accepted = true; }
                var response = JoinResponse(request.TransactionId, 0);
                response.MediusWorldUid = (uint)World.WorldId; response.ConnectionType = 3; response.AddressType = 1;
                var network = MagNetworkSettings.FromEnvironment();
                response.ServerAddress = FixedAscii(network.Address.ToString(), 16); response.ServerPort = network.GameTcpPort;
                response.WorldId = (uint)World.WorldId; response.AccessKey = FixedAscii(client.Token, 17);
                return response;
            }
            finally { _admission.Release(); }
        }
        MagJoinGameResponse JoinResponse(int transactionId, byte result)
        {
            MagJoinGameResponse response = _applicationId == 20231 ? new MagRetailJoinGameResponse() : new MagJoinGameResponse();
            response.TransactionId = transactionId; response.Result = result; return response;
        }
        static byte[] FixedAscii(string value, int length) { var b = new byte[length]; Encoding.ASCII.GetBytes(value).CopyTo(b, 0); return b; }
        Reservation Find(DmeClient client) => _reservations.Values.FirstOrDefault(r => r.Client == client);
        bool Current(Reservation r) => r != null && _reservations.TryGetValue(r.Id, out var current) && current == r;
        public bool TryBind(DmeClient client, RT_MSG_CLIENT_CONNECT_TCP_AUX_UDP connect)
        {
            lock (_gate)
            {
                var r = Find(client);
                if (!_ready || !_worldStreamHealthy || r == null || r.Invalidated || r.Claimed || r.Deadline <= DateTime.UtcNow || client == null || client.IsDestroyed ||
                    connect.AppId != _applicationId || connect.ARG1 != (uint)World.WorldId || connect.SessionKey != client.SessionKey || connect.AccessToken != client.Token) return false;
                r.Claimed = true; return true;
            }
        }
        public bool CanUse(DmeClient client) { lock (_gate) { var r = Find(client); return _ready && _worldStreamHealthy && r != null && !r.Invalidated && r.Claimed && r.Deadline > DateTime.UtcNow; } }
        public void Connected(DmeClient client) { lock (_gate) if (CanUse(client)) Find(client).Deadline = DateTime.UtcNow.AddSeconds(120); }
        public void Event(DmeClient client, string transport, byte rtType, byte? pluginId, byte[] payload)
        {
            lock (_gate)
            {
                if (!CanUse(client) || !client.IsAuthenticated) return;
                var r = Find(client);
                if (payload == null || payload.Length > 16384 || (transport != "tcp" && transport != "udp")) { r.Invalidated = true; r.ReleaseReason = "invalid_event"; return; }
                var message = new { v = 2, request_id = Id(), op = "event", reservation_id = r.Id, world_id = (uint)World.WorldId, player_id = (ushort)client.DmeId,
                    transport, direction = "client_to_server", rt_type = rtType, plugin_id = pluginId, payload_hex = Convert.ToHexString(payload).ToLowerInvariant() };
                if (!_events.Writer.TryWrite((r, message))) { r.Invalidated = true; r.ReleaseReason = "event_queue_full"; }
            }
        }
        // Called with _gate held; validate the entire batch before queueing anything.
        bool Route(GameplayBridgeResponse reply)
        {
            if (reply?.Ok != true) return false;
            if (reply.MatchTerminal != null && reply.MatchTerminal.WorldId != (uint)World.WorldId) return false;
            var deliveries = new List<(DmeClient client, GameplayBridgeMessage message)>();
            foreach (var message in reply.Messages)
            {
                // A released recipient may disappear while Rust's reply is in flight.
                // Reservation IDs prevent delivery to the next occupant of that slot.
                if (!_reservations.TryGetValue(message.ReservationId, out var recipient)) continue;
                var client = recipient.Client;
                if (message.WorldId != (uint)World.WorldId || message.PlayerId != (ushort)client.DmeId) return false;
                if (!CanUse(client) || !client.IsAuthenticated || client.IsDestroyed || client.Disconnected || client.Tcp?.Active != true ||
                    !World.Clients.TryGetValue(client.DmeId, out var member) || member != client) continue;
                deliveries.Add((client, message));
            }
            foreach (var delivery in deliveries) delivery.client.EnqueueTcp(delivery.message.ToScertMessage());
            if (reply.MatchTerminal != null)
                foreach (var id in reply.MatchTerminal.ReservationIds)
                    if (_reservations.TryGetValue(id, out var completed)) completed.TerminalMatchId = reply.MatchTerminal.MatchId;
            return true;
        }
        async Task RunWorldStreamAsync()
        {
            var cancellation = _worldStreamCancellation.Token;
            while (!cancellation.IsCancellationRequested)
            {
                try
                {
                    await _bridge.SubscribeWorldAsync((uint)World.WorldId, Id(), async reply =>
                    {
                        lock (_gate)
                        {
                            if (_disposed || cancellation.IsCancellationRequested) return;
                            if (!Route(reply)) throw new InvalidDataException("World stream recipient mismatch.");
                            _worldStreamHealthy = true;
                            _firstWorldStream.TrySetResult(true);
                        }
                        await SettleRewardsAsync(reply);
                    }, cancellation);
                }
                catch (OperationCanceledException) when (cancellation.IsCancellationRequested) { break; }
                catch (Exception error) when (error is IOException || error is InvalidDataException || error is System.Net.Sockets.SocketException || error is OperationCanceledException)
                {
                    Reservation[] affected;
                    lock (_gate)
                    {
                        _worldStreamHealthy = false;
                        affected = _reservations.Values.ToArray();
                        foreach (var reservation in affected) { reservation.Invalidated = true; reservation.ReleaseReason = "world_stream_gap"; }
                    }
                    Logger.Warn($"Gameplay world stream interrupted: failure={error.GetType().Name}; releasing {affected.Length} affected reservations");
                    // A successful socket write does not prove client receipt. We
                    // deliberately end affected sessions instead of resuming after
                    // an unknown replication gap. Fresh joins use fresh identities.
                    foreach (var reservation in affected) await ReleaseAsync(reservation);
                }
                try { await Task.Delay(TimeSpan.FromSeconds(1), cancellation); }
                catch (OperationCanceledException) when (cancellation.IsCancellationRequested) { break; }
            }
        }
        async Task RunEvents()
        {
            await foreach (var item in _events.Reader.ReadAllAsync())
            {
                lock (_gate) if (!Current(item.reservation) || item.reservation.Invalidated) continue;
                var elapsed = Stopwatch.StartNew();
                var reply = await _bridge.RequestEventAsync(item.message);
                lock (_gate)
                {
                    var reservation = item.reservation; var client = reservation.Client;
                    bool current = Current(reservation) && CanUse(client) && client.IsAuthenticated && !client.IsDestroyed && !client.Disconnected &&
                        client.Tcp?.Active == true && World.Clients.TryGetValue(client.DmeId, out var member) && member == client;
                    // A missing response cannot tell us whether Rust committed the event.
                    // Keep the authenticated transport; never retry an uncertain shot.
                    if (reply == null || reply.IsTransientFailure)
                    {
                        _rewardRetry = true;
                        Logger.Warn($"Gameplay bridge event unanswered: player={client.DmeId} failure={reply?.Failure.ToString() ?? "Unavailable"} elapsed_ms={elapsed.ElapsedMilliseconds}; retaining authenticated reservation without retry");
                        continue;
                    }
                    // Peer damage/life was committed by Rust even if the sender left
                    // while the request awaited. Route each still-current recipient.
                    if (!Route(reply))
                    {
                        if (current) { reservation.Invalidated = true; reservation.ReleaseReason = reply.Failure == GameplayBridgeFailure.None && !reply.Ok ? "rust_rejection" : "invalid_bridge_reply"; }
                        Logger.Warn($"Gameplay bridge event refused: player={client.DmeId} failure={reply.Failure} elapsed_ms={elapsed.ElapsedMilliseconds}; invalidating current reservation");
                        continue;
                    }
                    if (current) reservation.Deadline = DateTime.UtcNow.AddSeconds(120);
                }
                await SettleRewardsAsync(reply);
            }
        }
        async Task SettleRewardsAsync(GameplayBridgeResponse reply)
        {
            if (reply?.Ok != true || reply.Rewards.Count == 0) return;
            await _rewardSettlement.WaitAsync();
            try
            {
                var owners = new List<(GameplayReward reward, Reservation owner)>();
                lock (_gate)
                {
                    // Validate the whole reward batch against authenticated admission,
                    // never an account name supplied by a gameplay/client packet alone.
                    foreach (var reward in reply.Rewards)
                    {
                        if (!_rewardOwners.TryGetValue(reward.ReservationId, out var owner) ||
                            reward.WorldId != (uint)World.WorldId || reward.PlayerId != (ushort)owner.Client.DmeId ||
                            reward.AccountId != owner.Account || reward.CharacterId != owner.CharacterId)
                        { _rewardRetry = true; Logger.Warn("Gameplay reward rejected: admission identity mismatch"); return; }
                        owners.Add((reward, owner));
                    }
                }
                bool retry = false; bool pending = false;
                foreach (var pair in owners)
                {
                    var reward = pair.reward; var owner = pair.owner;
                    try
                    {
                        // Replacement reservations share the surviving round's native XP carry and result basis.
                        var result = _store.ApplyRewardForAccount(owner.Account, owner.CharacterId, reward.RewardId,
                            $"world-{reward.WorldId}-{reward.MatchId}", reward.Kind, checked((int)reward.Units));
                        if (result.Result != MagNetResult.Success)
                        { retry = true; Logger.Warn($"Gameplay reward not committed: player={owner.Client.DmeId} reason={result.Reason}"); break; }
                        // Publish the latest account record, not an older receipt snapshot.
                        // A missed notification is also repaired by the next0238 query.
                        bool notify; lock (_gate) notify = result.ExperienceAwarded > 0 && _notifiedRewards.Add(reward.RewardId);
                        if (notify && owner.Notify != null)
                        {
                            var current = _store.ReadForAccount(owner.Account).Single(c => c.Id == owner.CharacterId).ToRecord();
                            owner.Notify(current);
                        }
                        var ack = await _bridge.RequestProgressionAsync(new { v = 2, request_id = Id(), op = "progression_committed",
                            reservation_id = owner.Id, world_id = reward.WorldId, player_id = reward.PlayerId, reward_id = reward.RewardId,
                            progression = new { experience = (uint)result.Experience, level = result.Level, skill_points = result.SkillPoints },
                            match_experience = (uint)result.MatchExperience });
                        lock (_gate) { if (!Route(ack)) retry = true; else pending = ack.Rewards.Count > 0; }
                        Logger.Info($"Gameplay reward committed: player={owner.Client.DmeId} kind={reward.Kind} units={reward.Units} xp={result.Experience} level={result.Level} skill_points={result.SkillPoints} match_xp={result.MatchExperience} duplicate={result.Duplicate}");
                    }
                    catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                    { retry = true; Logger.Warn($"Gameplay reward persistence deferred: player={owner.Client.DmeId} error={e.GetType().Name}"); break; }
                }
                lock (_gate) _rewardRetry = retry || pending;
            }
            finally { _rewardSettlement.Release(); }
        }
        public async Task TickAsync()
        {
            if (_disposed || !_ready) return;
            bool poll; lock (_gate) { poll = _rewardRetry && DateTime.UtcNow >= _nextRewardPoll; if (poll) _nextRewardPoll = DateTime.UtcNow.AddSeconds(1); }
            if (poll)
            {
                var reply = await _bridge.RequestProgressionAsync(new { v = 2, request_id = Id(), op = "hello" });
                lock (_gate) Route(reply);
                await SettleRewardsAsync(reply);
            }
            PendingRelease[] pending; lock (_gate) pending = _pendingReleases.Values.Where(r => r.NextAttempt <= DateTime.UtcNow).ToArray();
            foreach (var item in pending) await RetryReleaseAsync(item);
            Reservation[] release;
            lock (_gate) release = _reservations.Values.Where(r => r.Invalidated || r.Deadline <= DateTime.UtcNow || r.Client.IsDestroyed || r.Client.Disconnected ||
                (r.Claimed && r.Client.Tcp != null && !r.Client.Tcp.Active)).ToArray();
            foreach (var r in release) await ReleaseAsync(r);
            await DmeProgram.TcpServer.HandleIncomingMessages(); await World.HandleIncomingMessages(); await DmeProgram.TcpServer.HandleOutgoingMessages();
            foreach (var client in World.Clients.Values) client.HandleOutgoingMessages();
        }
        async Task ReleaseAsync(Reservation r)
        {
            PendingRelease pending;
            lock (_gate) { if (!Current(r)) return; r.Invalidated = true; _reservations.Remove(r.Id); pending = new PendingRelease { Reservation = r, RequestId = Id() }; _pendingReleases.Add(r.Id, pending); }
            Logger.Warn($"Gameplay reservation released: player={r.Client.DmeId} reason={r.ReleaseReason ?? (r.Deadline <= DateTime.UtcNow ? "application_inactivity" : "transport_closed")}");
            await r.Client.Stop(); World.Manager.RemoveClient(r.Client); World.Clients.TryRemove(r.Client.DmeId, out _); World.UnregisterClientIndex(r.Client.DmeId);
            await RetryReleaseAsync(pending);
        }
        async Task RetryReleaseAsync(PendingRelease pending)
        {
            await _releaseSettlement.WaitAsync();
            try
            {
                lock (_gate) if (!_pendingReleases.TryGetValue(pending.Reservation.Id, out var current) || current != pending || pending.NextAttempt > DateTime.UtcNow) return;
                var reply = await _bridge.RequestReleaseAsync(new { v = 2, request_id = pending.RequestId, op = "release", reservation_id = pending.Reservation.Id });
                lock (_gate)
                {
                    if (Route(reply)) _pendingReleases.Remove(pending.Reservation.Id);
                    else
                    {
                        bool retry = reply == null || reply.IsTransientFailure;
                        pending.NextAttempt = retry ? DateTime.UtcNow.AddSeconds(1) : DateTime.MaxValue;
                        _rewardRetry = true;
                        Logger.Warn($"Gameplay release acknowledgement pending: player={pending.Reservation.Client.DmeId} retry={retry} failure={reply?.Failure.ToString() ?? "Unavailable"}");
                    }
                }
                await SettleRewardsAsync(reply);
            }
            finally { _releaseSettlement.Release(); }
        }
        public async ValueTask DisposeAsync()
        {
            lock (_gate) { if (_disposed) return; _disposed = true; _ready = false; foreach (var r in _reservations.Values) r.Invalidated = true; }
            _worldStreamCancellation.Cancel(); await _worldStreamWorker;
            _events.Writer.TryComplete(); await _worker;
            Reservation[] release; lock (_gate) release = _reservations.Values.ToArray(); foreach (var r in release) await ReleaseAsync(r);
            if (World != null) { await World.Stop(); DmeProgram.Managers.Remove(_applicationId); }
            if (_started) await DmeProgram.TcpServer.Stop(); if (DmeProgram.LocalGameplay == this) DmeProgram.LocalGameplay = null;
        }
    }
}
