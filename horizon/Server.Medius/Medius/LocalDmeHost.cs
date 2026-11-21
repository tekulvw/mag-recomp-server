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
using DmeWorld = Server.Dme.Models.World;
namespace Server.Medius
{
    /// <summary>One shared local Horizon world; each member has a Rust-acknowledged reservation.</summary>
    public sealed partial class LocalDmeHost : Server.Dme.ILocalGameplaySink, IAsyncDisposable
    {
        sealed class Reservation { public string Id; public uint AccountId; public string Account; public uint CharacterId; public Action<MagCharacterRecord> Notify; public DmeClient Client; public DateTime Deadline; public byte Faction; public bool Accepted; public bool Claimed; public bool Invalidated; public string ReleaseReason; public string TerminalMatchId; public bool FragoActionBonusEnabled; public bool FragoObjectiveBonusEnabled; public bool ExtendedObjectiveRewardsEnabled; public bool RepairRewardsEnabled; }
        sealed class PendingRelease { public Reservation Reservation; public string RequestId; public DateTime NextAttempt; public DateTime? FirstRejected; public bool InFlight; }
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
        // Keep admission identities after transport release until pending rewards settle.
        // Bound them rather than recycling a reservation identity into a new actor.
        readonly Dictionary<string, Reservation> _rewardOwners = new Dictionary<string, Reservation>(StringComparer.Ordinal);
        readonly HashSet<string> _notifiedRewards = new HashSet<string>(StringComparer.Ordinal);
        readonly SemaphoreSlim _rewardSettlement = new SemaphoreSlim(1, 1);
        readonly SemaphoreSlim _rewardAcknowledgements = new SemaphoreSlim(1, 1);
        readonly SemaphoreSlim _loadoutSelectionSettlement = new SemaphoreSlim(1, 1);
        readonly GameplayLoadoutSelectionSettlement _loadoutSelections;
        readonly GameplayLoadoutRefreshQueue _loadoutRefreshes;
        readonly CancellationTokenSource _loadoutRefreshCancellation = new CancellationTokenSource();
        Task _loadoutRefreshWorker = Task.CompletedTask;
        bool _selectionRetry;
        DateTime _nextSelectionPoll;
        bool _rewardRetry;
        DateTime _nextRewardPoll;
        static Channel<(Reservation reservation, object message)> CreateEventQueue() =>
            Channel.CreateBounded<(Reservation, object)>(new BoundedChannelOptions(128) { SingleReader = true, FullMode = BoundedChannelFullMode.Wait });
        readonly Channel<(Reservation reservation, object message)> _events = CreateEventQueue();
        readonly Channel<(Reservation reservation, object message)> _resultsEvents = CreateEventQueue();
        readonly Task _worker;
        readonly Task _resultsEventWorker;
        readonly CancellationTokenSource _worldStreamCancellation = new CancellationTokenSource();
        readonly TaskCompletionSource<bool> _firstWorldStream = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        Task _worldStreamWorker = Task.CompletedTask;
        bool _worldStreamHealthy;
        bool _supportsFragoKillBonus;
        bool _supportsFragoActionBonus;
        bool _supportsFragoObjectiveBonus;
        bool _supportsExtendedObjectiveRewards;
        bool _supportsRepairRewards;
        bool _ready; bool _started; bool _disposed;
        public DmeWorld World { get; private set; }
        public bool IsReady { get { lock (_gate) return _ready && _worldStreamHealthy && !_disposed; } }
        // Rust evicts a reservation that has sent no event for its 120 s idle timeout.
        // Horizon stops forwarding events once it releases, so after this grace a
        // definitively rejected release cannot still name a live Rust reservation.
        public static readonly TimeSpan RejectedReleaseRetryInterval = TimeSpan.FromSeconds(15);
        public static readonly TimeSpan RejectedReleaseAbandonAfter = TimeSpan.FromSeconds(150);
        /// <summary>Next attempt for a definitively rejected release, or null once it must be abandoned.</summary>
        public static DateTime? NextRejectedReleaseAttempt(DateTime firstRejectedUtc, DateTime nowUtc) =>
            nowUtc - firstRejectedUtc >= RejectedReleaseAbandonAfter ? null : nowUtc + RejectedReleaseRetryInterval;
        /// <summary>Release reason for a join whose gameplay reserve was not accepted, or accepted while Horizon was not ready.</summary>
        public static string AdmissionFailureReason(GameplayBridgeResponse reply, bool ready)
        {
            if (reply == null) return "admission_unanswered";
            if (reply.Ok != true) return reply.IsTransientFailure ? "admission_unavailable" : "admission_rejected";
            return ready ? "admission_accepted" : "admission_not_ready";
        }
        /// <summary>First reason a join request's own fields are unsupported here, or null.</summary>
        public static string JoinRequestRejection(MagJoinGameRequest request, bool retailApplication, Func<uint, bool> modeIsCurrent)
        {
            if ((request is MagRetailJoinGameRequest) != retailApplication) return "request_application_mismatch";
            if (request is MagRetailJoinGameRequest retail && retail.PartyId != 0) return "party_join_unsupported";
            if (request.Preference != 1) return "preference_unsupported";
            if (!modeIsCurrent(request.Value)) return "mode_not_current";
            if (request.ClientType != 0) return "client_type_unsupported";
            return null;
        }
        string JoinSessionRejection(MagMlsSession session)
        {
            if (session == null || session.ApplicationId != _applicationId) return "session_application_mismatch";
            if (session.State != MagMlsSession.SessionState.Ready) return "session_not_ready";
            if (session.IsExpired(DateTime.UtcNow)) return "session_expired";
            if (session.AccountId == 0 || session.CharacterId == 0) return "session_without_character";
            if (!MagDevelopmentAccountPolicy.ValidAccount(session.AuthenticatedAccount)) return "account_not_allowed";
            if (!IsReady) return "host_not_ready";
            return null;
        }
        static void LogJoinRejection(string reason, MagJoinGameRequest request) =>
            Logger.Warn($"Gameplay join rejected: reason={reason} preference={request.Preference} value={request.Value} client_type={request.ClientType} party={(request as MagRetailJoinGameRequest)?.PartyId.ToString() ?? "none"}");
        public static int SupportedMaximumPlayers(int applicationId, int gameType, bool hasDominationProfiles = false)
        {
            bool retail = applicationId == 20231;
            if (gameType != 2 && gameType != 3 && !(retail && (gameType == 0 || gameType == 1)))
                throw new ArgumentException("Local gameplay serves types2/3, plus retail Sabotage0 and bounded Acquisition1.");
            return retail && (gameType == 0 || gameType == 1 || gameType == 3 || (gameType == 2 && hasDominationProfiles)) ? 64 : 8;
        }
        public LocalDmeHost(GameplayBridge bridge, LocalCharacterStore store)
        {
            _applicationId = Program.Settings.MagApplicationId;
            _startupPolicy = new MagMatchStartupPolicy(Program.Settings.MagMinimumPlayers, Program.Settings.MagMaximumPlayers, Program.Settings.MagMinimumPlayersPerTeam);
            _minimumPlayers = _startupPolicy.MinimumPlayers;
            _gameType = Program.Settings.MagGameType;
            _maximumPlayers = Program.Settings.MagMaximumPlayers;
            // Hello verifies profile-backed Domination before the host opens reservations.
            int supportedMaximum = SupportedMaximumPlayers(_applicationId, _gameType, hasDominationProfiles: true);
            if (_minimumPlayers < 1 || _maximumPlayers > supportedMaximum || _minimumPlayers > _maximumPlayers) throw new ArgumentException($"Local match limits must be 1..{supportedMaximum}.");
            var configuredFactions = Program.Settings.MagAllowedFactions ?? new[] { 0, 1, 2 };
            if (configuredFactions.Length == 0 || configuredFactions.Any(faction => faction < 0 || faction > 2) || configuredFactions.Distinct().Count() != configuredFactions.Length)
                throw new ArgumentException("MagAllowedFactions requires distinct faction values0..2, or null for legacy availability.");
            _allowedFactions = new HashSet<byte>(configuredFactions.Select(faction => (byte)faction));
            _bridge = bridge; _store = store; _loadoutSelections = new GameplayLoadoutSelectionSettlement(_gate);
            _loadoutRefreshes = new GameplayLoadoutRefreshQueue(_gate, (target, revision, reason) =>
                Logger.Warn($"Active Armory refresh dropped: player={target.PlayerId} revision={revision} reason={reason}"));
            _worker = RunEvents(_events.Reader, false);
            _resultsEventWorker = RunEvents(_resultsEvents.Reader, true);
        }
        static string Id() => Guid.NewGuid().ToString("N");
        public async Task StartAsync()
        {
            if (_started) throw new InvalidOperationException("DME already started."); _started = true;
            var hello = await _bridge.RequestHelloAsync(new { v = 2, request_id = Id(), op = "hello" });
            InitializeGameplayHello(hello);
            bool hasDominationProfiles = UsesFactionMatchmaking && _matchProfiles.Any(profile => profile.GameType == 2);
            if (_maximumPlayers > SupportedMaximumPlayers(_applicationId, _gameType, hasDominationProfiles))
                throw new IOException("Expanded retail Domination admission requires all ordered faction profiles.");
            if (!UsesFactionMatchmaking && !await _bridge.RequestAsync(_startupPolicy.ConfigureRequest(Id(), _applicationId))) throw new IOException("Gameplay match configuration was not acknowledged.");
            DmeProgram.SERVER_IP = MagNetworkSettings.FromEnvironment().Address; DmeProgram.Settings.TCPPort = 10073; DmeProgram.Settings.UDPPort = 10474;
            DmeProgram.Plugins = Program.Plugins ?? new Server.Plugins.PluginsManager("plugins/");
            var manager = new Server.Dme.MediusManager(_applicationId) { LocalOnly = true }; DmeProgram.Managers.Add(_applicationId, manager);
            World = new DmeWorld(manager, _applicationId, _maximumPlayers); DmeProgram.LocalGameplay = this;
            try {
                _worldStreamWorker = RunWorldStreamAsync();
                await _firstWorldStream.Task.WaitAsync(TimeSpan.FromSeconds(5));
                await DmeProgram.TcpServer.StartAsync(); lock (_gate) _ready = true;
            }
            catch { await DisposeAsync(); throw; }
        }
        void InitializeGameplayHello(GameplayBridgeResponse hello)
        {
            RequireLoadoutSelectionCapability(hello);
            InitializeFactionMatchmaking(hello);
            _independentMatchCohorts = hello.Capabilities.Contains("independent_match_cohorts_v1", StringComparer.Ordinal);
            _supportsFragoKillBonus = hello.Capabilities.Contains("frago_kill_bonus_v1", StringComparer.Ordinal);
            _supportsFragoActionBonus = hello.Capabilities.Contains("frago_action_bonus_v1", StringComparer.Ordinal);
            _supportsFragoObjectiveBonus = hello.Capabilities.Contains("frago_objective_bonus_v1", StringComparer.Ordinal);
            _supportsExtendedObjectiveRewards = hello.Capabilities.Contains("extended_objective_rewards_v1", StringComparer.Ordinal);
            _supportsRepairRewards = hello.Capabilities.Contains("repair_rewards_v1", StringComparer.Ordinal);
        }
        public static void RequireLoadoutSelectionCapability(GameplayBridgeResponse hello)
        {
            if (hello?.Ok != true || !hello.SupportsCommittedLoadoutSelections || !hello.SupportsActiveLoadoutRefresh)
                throw new IOException("Gameplay service lacks committed_loadout_selection_v1 or active_loadout_refresh_v1; admission remains disabled.");
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
                if (!UsesFactionMatchmaking && _ready && _worldStreamHealthy && !_disposed)
                    foreach (var faction in _allowedFactions) status.QueueEnabled[_gameType * 3 + faction] = true;
                var now = DateTime.UtcNow;
                foreach (var reservation in _reservations.Values)
                {
                    var client = reservation.Client;
                    if (!IsLiveCohort(reservation) || !reservation.Accepted || reservation.Invalidated || reservation.TerminalMatchId != null || reservation.Deadline <= now || reservation.Faction > 2 ||
                        client == null || client.IsDestroyed || client.Disconnected || World == null ||
                        !client.DmeWorld.Clients.TryGetValue(client.DmeId, out var member) || member != client ||
                        (client.Tcp != null && !client.Tcp.Active)) continue;
                    // Native status matrices are game type first, then faction.
                    // These are authenticated world members, not physical spawns.
                    int index = (UsesFactionMatchmaking?_formedProfile.GameType:_gameType) * 3 + reservation.Faction;
                    if (reservation.Claimed && client.IsAuthenticated && client.Tcp?.Active == true) status.PlayersInGame[index]++;
                    else status.QueueSizes[index]++;
                }
                AddQueuedStatus(status);
                return status;
            }
        }
        public async Task<MagJoinGameResponse> JoinAsync(MagMlsSession session, MagJoinGameRequest request, Action<MagCharacterRecord> notify = null)
        {
            var failure = JoinResponse(request.TransactionId, 2);
            // Party membership/admission is not implemented. Do not silently turn a party request into a solo join.
            var rejection = JoinRequestRejection(request, _applicationId == 20231, RequestedModeIsCurrent) ?? JoinSessionRejection(session);
            if (rejection != null) { LogJoinRejection(rejection, request); return failure; }
            await _admission.WaitAsync();
            try
            {
                await ReleasePriorResultsAsync(session.AuthenticatedAccount, session.CharacterId);
                PendingRelease pending;
                lock (_gate) pending = _pendingReleases.Values.FirstOrDefault(r => !IsResultsCohort(r.Reservation) && r.Reservation.Account == session.AuthenticatedAccount);
                if (pending != null) await RetryReleaseAsync(pending);
                Reservation reservation;
                DmeClient client;
                GameplayBridgeResponse admittedReply;
                bool admittedRouted;
                // A reward must either precede the snapshot read or reach the reserved actor by ACK.
                // Release and reply settlement below can themselves settle rewards and require this gate.
                await _rewardSettlement.WaitAsync();
                try
                {
                    DmeWorld admissionWorld;
                    lock (_gate)
                    {
                        if (_independentMatchCohorts)
                        {
                            if (_admissionTerminal && (UsesFactionMatchmaking || !AdvanceTerminalCohort())) return JoinResponse(request.TransactionId, 6);
                        }
                        else if (_reservations.Values.Any(r => r.TerminalMatchId != null)) return JoinResponse(request.TransactionId, 6);
                        if (_pendingReleases.Values.Any(p => !IsResultsCohort(p.Reservation) &&
                            (p.Reservation.TerminalMatchId != null || p.Reservation.Account == session.AuthenticatedAccount))) return JoinResponse(request.TransactionId, 6);
                        if (_rewardOwners.Count >= 4096 || _reservations.Values.Count(IsLiveCohort) >= _maximumPlayers ||
                            _reservations.Values.Any(r => IsLiveCohort(r) && r.AccountId == session.AccountId)) return JoinResponse(request.TransactionId, 6);
                        admissionWorld = AdmissionWorld;
                    }
                    StoredCharacter character;
                    try { character = _store.ReadForAccount(session.AuthenticatedAccount).SingleOrDefault(c => c.Id == session.CharacterId); }
                    catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException) { return failure; }
                    if (character == null) return failure;
                    if (UsesFactionMatchmaking ? character.Faction!=_formedProfile.AttackingFaction&&character.Faction!=_formedProfile.DefendingFaction : !_allowedFactions.Contains(character.Faction)) return JoinResponse(request.TransactionId, 4);
                    var record = character.ToRecord();
                    GameplayAdmissionLoadouts admissionLoadouts;
                    try { admissionLoadouts = GameplayLoadoutAdmission.Project(character, record, _store.LoadoutCatalog); }
                    catch (InvalidDataException) { return failure; }
                    var allocation = await admissionWorld.OnJoinGameRequest(new MediusServerJoinGameRequest { ConnectInfo = new NetConnectionInfo { SessionKey = session.SessionKey } });
                    if (allocation.Confirmation != MGCL_ERROR_CODE.MGCL_SUCCESS || !admissionWorld.Clients.TryGetValue(allocation.DmeClientIndex, out client)) return failure;
                    client.ApplicationId = _applicationId;
                    var progressionCatalog = _store.ProgressionCatalog;
                    reservation = new Reservation {
                        Id = Id(), AccountId = character.Id, Account = session.AuthenticatedAccount, CharacterId = character.Id,
                        Faction = character.Faction, Notify = notify, Client = client, Deadline = DateTime.UtcNow.AddSeconds(30),
                        FragoActionBonusEnabled = _supportsFragoActionBonus && _applicationId == 20231 && progressionCatalog?.FragoActionBonusEnabled == true,
                        FragoObjectiveBonusEnabled = _supportsFragoObjectiveBonus && _applicationId == 20231 && progressionCatalog?.FragoBonusEnabled == true,
                        ExtendedObjectiveRewardsEnabled = _supportsExtendedObjectiveRewards && _applicationId == 20231 &&
                            progressionCatalog?.ExtendedObjectiveRewardsEnabled == true,
                        RepairRewardsEnabled = _supportsRepairRewards && _applicationId == 20231 && progressionCatalog?.RepairRewardsEnabled == true
                    };
                    lock (_gate) { _reservations.Add(reservation.Id, reservation); _rewardOwners.Add(reservation.Id, reservation); }
                    var admissionCharacter = new Dictionary<string, object> {
                        ["loadout_account_id"] = reservation.Account,
                        ["leadership"] = character.Leadership == null ? null : new GameplayLeadershipRecord(character.Leadership),
                        ["leadership_progression_enabled"] = progressionCatalog?.InitialLeadershipPoints.HasValue == true,
                        ["account_id"] = progressionCatalog == null ? null : reservation.Account,
                        ["progression"] = progressionCatalog == null ? null : new { experience = record.Experience, level = record.Level, skill_points = record.SkillPoints },
                        ["skills"] = record.Skills, ["character_id"] = record.AccountId, ["name"] = record.Name,
                        ["faction"] = record.Faction, ["head"] = record.HeadId, ["voice"] = record.VoiceId,
                        ["universe_id"] = record.UniverseId, ["loadouts"] = admissionLoadouts
                    };
                    // Older workers reject unknown character fields even when their value is false.
                    if (_supportsFragoKillBonus)
                        admissionCharacter["frago_bonus_enabled"] = _applicationId == 20231 && progressionCatalog?.FragoBonusEnabled == true;
                    if (_supportsFragoActionBonus)
                        admissionCharacter["frago_action_bonus_enabled"] = reservation.FragoActionBonusEnabled;
                    if (_supportsFragoObjectiveBonus)
                        admissionCharacter["frago_objective_bonus_enabled"] = reservation.FragoObjectiveBonusEnabled;
                    if (_supportsExtendedObjectiveRewards)
                        admissionCharacter["extended_objective_rewards_enabled"] = reservation.ExtendedObjectiveRewardsEnabled;
                    if (_supportsRepairRewards)
                        admissionCharacter["repair_rewards_enabled"] = reservation.RepairRewardsEnabled;
                    admittedReply = await _bridge.RequestControlAsync(new { v = 2, request_id = Id(), op = "reserve", reservation_id = reservation.Id,
                        world_id = (uint)World.WorldId, player_id = (ushort)client.DmeId, account_id = character.Id, session_key = client.SessionKey, access_key = client.Token,
                        character = admissionCharacter });
                    lock (_gate) admittedRouted = Route(admittedReply);
                }
                finally { _rewardSettlement.Release(); }
                await ObserveReceiptsAsync(admittedReply, admittedRouted);
                bool admitted = admittedReply?.Ok == true;
                bool ready; bool readyFlag; bool streamHealthy; lock (_gate) { readyFlag = _ready; streamHealthy = _worldStreamHealthy; ready = readyFlag && streamHealthy && !_disposed; }
                if (!admitted || !ready)
                {
                    // Gameplay may record the reservation before a later step of its reserve
                    // fails, so name the cause here instead of the default transport_closed.
                    reservation.ReleaseReason = AdmissionFailureReason(admittedReply, ready);
                    Logger.Warn($"Gameplay admission refused: player={client.DmeId} reason={reservation.ReleaseReason} error={admittedReply?.Error ?? "none"} failure={admittedReply?.Failure.ToString() ?? "Unavailable"} elapsed_ms={admittedReply?.ElapsedMilliseconds ?? -1} ready={readyFlag} world_stream={streamHealthy}");
                    bool terminalRefusal = _independentMatchCohorts && admittedReply?.Failure == GameplayBridgeFailure.None &&
                        admittedReply.Error is "results_cohort_limit" or "match_finished";
                    // These rejections precede any Rust reservation. A release
                    // retry would hold the next live admission against an unused ID.
                    await ReleaseReservationAsync(reservation, !terminalRefusal);
                    return terminalRefusal ? JoinResponse(request.TransactionId, 6) : failure;
                }
                lock (_gate)
                {
                    if (!Current(reservation) || reservation.Invalidated || !_worldStreamHealthy)
                    { reservation.Invalidated = true; reservation.ReleaseReason = "world_stream_gap"; return failure; }
                    if (!IsResultsCohort(reservation)) reservation.Accepted = true;
                }
                if (!reservation.Accepted)
                {
                    reservation.ReleaseReason = "admission_after_terminal";
                    await ReleaseAsync(reservation);
                    return JoinResponse(request.TransactionId, 6);
                }
                // Reconcile once after admission, closing saves committed between
                // the initial store read and registration/reserve acknowledgement.
                ArmoryLoadoutCommitted(reservation.Account, reservation.CharacterId);
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
                if (!_ready || !_worldStreamHealthy || r == null || IsResultsCohort(r) || r.Invalidated || r.Claimed || r.Deadline <= DateTime.UtcNow || client == null || client.IsDestroyed ||
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
                bool archived = IsArchivedTransport(r);
                var queue = archived ? _resultsEvents : _events;
                if (!queue.Writer.TryWrite((r, message)) && !archived) { r.Invalidated = true; r.ReleaseReason = "event_queue_full"; }
            }
        }
        // Called with _gate held; validate the entire batch before queueing anything.
        bool Route(GameplayBridgeResponse reply)
        {
            if (reply?.Ok != true) return false;
            if (!TryFindTerminalMembers(reply.MatchTerminal, out var completed)) return false;
            var deliveries = new List<(DmeClient client, GameplayBridgeMessage message)>();
            foreach (var message in reply.Messages)
            {
                // A released recipient may disappear while Rust's reply is in flight.
                // Reservation IDs prevent delivery to the next occupant of that slot.
                if (!_reservations.TryGetValue(message.ReservationId, out var recipient)) continue;
                var client = recipient.Client;
                if (message.WorldId != (uint)World.WorldId || message.PlayerId != (ushort)client.DmeId) return false;
                if (!CanUse(client) || !client.IsAuthenticated || client.IsDestroyed || client.Disconnected || client.Tcp?.Active != true ||
                    !client.DmeWorld.Clients.TryGetValue(client.DmeId, out var member) || member != client) continue;
                deliveries.Add((client, message));
            }
            foreach (var delivery in deliveries) delivery.client.EnqueueTcp(delivery.message.ToScertMessage());
            foreach (var r in completed) r.TerminalMatchId = reply.MatchTerminal.MatchId;
            if (_independentMatchCohorts && completed.Any(r => r.Client.DmeWorld == AdmissionWorld)) _admissionTerminal = true;
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
                        await ObserveReceiptsAsync(reply, true);
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
                    await ReleaseReservationsAsync(affected);
                }
                try { await Task.Delay(TimeSpan.FromSeconds(1), cancellation); }
                catch (OperationCanceledException) when (cancellation.IsCancellationRequested) { break; }
            }
        }
        async Task RunEvents(ChannelReader<(Reservation reservation, object message)> reader, bool resultsOnly)
        {
            await foreach (var item in reader.ReadAllAsync())
            {
                lock (_gate)
                {
                    if (!Current(item.reservation) || item.reservation.Invalidated) continue;
                    if (!resultsOnly && IsArchivedTransport(item.reservation))
                    {
                        _resultsEvents.Writer.TryWrite(item);
                        continue;
                    }
                }
                var elapsed = Stopwatch.StartNew();
                var reply = await _bridge.RequestEventAsync(item.message);
                lock (_gate)
                {
                    var reservation = item.reservation; var client = reservation.Client;
                    bool current = Current(reservation) && CanUse(client) && client.IsAuthenticated && !client.IsDestroyed && !client.Disconnected &&
                        client.Tcp?.Active == true && client.DmeWorld.Clients.TryGetValue(client.DmeId, out var member) && member == client;
                    // A missing response cannot tell us whether Rust committed the event.
                    // Keep the authenticated transport; never retry an uncertain shot.
                    if (reply == null || reply.IsTransientFailure)
                    {
                        _rewardRetry = true; _selectionRetry = true;
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
                await ObserveReceiptsAsync(reply, true);
            }
        }
        bool IsCurrentRefreshOwner(GameplayLoadoutRefreshTarget target)
        {
            return !_disposed && _ready && _worldStreamHealthy && World != null &&
                _reservations.TryGetValue(target.ReservationId, out var owner) && IsLiveCohort(owner) && owner.Accepted && !owner.Invalidated &&
                owner.Deadline > DateTime.UtcNow && owner.Client != null && !owner.Client.IsDestroyed && !owner.Client.Disconnected &&
                target.MatchesOwner(owner.Id, (uint)World.WorldId, (ushort)owner.Client.DmeId, owner.Account, owner.CharacterId);
        }
        GameplayAdmissionLoadouts ProjectCurrentLoadouts(GameplayLoadoutRefreshTarget target)
        {
            var character = _store.ReadForAccount(target.AccountId).SingleOrDefault(row => row.Id == target.CharacterId);
            if (character == null || _store.LoadoutCatalog == null) throw new InvalidDataException("Current Armory catalog or character unavailable.");
            return GameplayLoadoutAdmission.Project(character, character.ToRecord(), _store.LoadoutCatalog);
        }
        public void ArmoryLoadoutCommitted(string account, uint characterId)
        {
            lock (_gate)
            {
                if (_disposed || World == null) return;
                var owner = _reservations.Values.FirstOrDefault(row => IsLiveCohort(row) &&
                    string.Equals(row.Account, account, StringComparison.Ordinal) && row.CharacterId == characterId);
                if (owner == null) return; // The next admission reads the latest durable record.
                var target = new GameplayLoadoutRefreshTarget(owner.Id, (uint)World.WorldId, (ushort)owner.Client.DmeId, owner.Account, owner.CharacterId);
                _loadoutRefreshes.Schedule(target, DateTime.UtcNow, IsCurrentRefreshOwner, ProjectCurrentLoadouts);
                StartLoadoutRefreshPump();
            }
        }
        void StartLoadoutRefreshPump()
        {
            lock (_gate)
            {
                if (_disposed || !_loadoutRefreshWorker.IsCompleted || !_loadoutRefreshes.HasDue(DateTime.UtcNow)) return;
                _loadoutRefreshWorker = Task.Run(PumpLoadoutRefreshesAsync);
            }
        }
        async Task PumpLoadoutRefreshesAsync()
        {
            // Keep refresh RPCs off the game tick. Each RPC has the bridge's two
            // second deadline; cancellation stops before another request starts.
            for (int batch = 0; batch < 8 && !_loadoutRefreshCancellation.IsCancellationRequested; batch++)
            {
                GameplayLoadoutRefreshRequest request = null;
                try
                {
                    request = _loadoutRefreshes.TakeDue(DateTime.UtcNow, IsCurrentRefreshOwner, ProjectCurrentLoadouts);
                    if (request == null) return;
                    // Release may win after this check. Rust also checks the exact
                    // reservation and revision, so an already-sent old request cannot
                    // modify the actor that later occupies its player slot.
                    if (!_loadoutRefreshes.StillCurrent(request, IsCurrentRefreshOwner))
                    { _loadoutRefreshes.Complete(request, DateTime.UtcNow, false, false, IsCurrentRefreshOwner); continue; }
                    var reply = await _bridge.RequestControlAsync(request);
                    lock (_gate)
                    {
                        if (_disposed || _loadoutRefreshCancellation.IsCancellationRequested) return;
                        bool routed = Route(reply);
                        _loadoutRefreshes.Complete(request, DateTime.UtcNow, routed, reply?.IsTransientFailure == true, IsCurrentRefreshOwner);
                        if (routed)
                        {
                            // Use the normal Hello/outbox settlement paths rather
                            // than holding this bounded pump on other receipt RPCs.
                            if (reply.LoadoutSelections.Count > 0) _selectionRetry = true;
                            if (reply.Rewards.Count > 0) _rewardRetry = true;
                        }
                    }
                }
                catch (Exception error)
                {
                    Logger.Error($"Active Armory refresh failed: player={request?.PlayerId} revision={request?.Revision} error={error.GetType().Name}");
                    if (request != null) _loadoutRefreshes.Complete(request, DateTime.UtcNow, false, false, IsCurrentRefreshOwner);
                }
            }
        }
        bool IsCurrentLoadoutOwner(GameplayLoadoutSelection action)
        {
            return !_disposed && World != null && _reservations.TryGetValue(action.ReservationId, out var owner) && IsLiveCohort(owner) &&
                !owner.Invalidated && owner.Accepted && owner.Client != null &&
                action.MatchesOwner(owner.Id, (uint)World.WorldId, (ushort)owner.Client.DmeId, owner.Account, owner.CharacterId);
        }
        async Task SettleLoadoutSelectionsAsync(GameplayBridgeResponse reply)
        {
            if (reply?.Ok != true) return;
            await _loadoutSelectionSettlement.WaitAsync();
            try
            {
                var actions = new Queue<GameplayLoadoutSelection>(reply.LoadoutSelections);
                var seen = new HashSet<(string, ulong)>();
                bool retry = false;
                int remaining = 64; // Bounded acknowledgements even if the peer repeats an uncleared receipt.
                while (actions.Count > 0 && remaining-- > 0)
                {
                    lock (_gate) if (_disposed && _independentMatchCohorts) return;
                    var action = actions.Dequeue();
                    if (!seen.Add((action.SelectionId, action.Revision))) { retry = true; continue; }
                    string status = _loadoutSelections.Settle(action, DateTime.UtcNow, IsCurrentLoadoutOwner,
                        selection => _store.SaveLastUsedLoadoutForAccount(selection.AccountId, selection.CharacterId, selection.InventoryIndex));
                    if (status == null) { retry = true; continue; }
                    var ack = await _bridge.RequestLoadoutSelectionSettlementAsync(new { v = 2, request_id = Id(), op = "loadout_selection_settled",
                        reservation_id = action.ReservationId, world_id = action.WorldId, player_id = action.PlayerId,
                        selection_id = action.SelectionId, revision = action.Revision, status });
                    lock (_gate)
                    {
                        if (!Route(ack)) retry = true;
                        else
                        {
                            foreach (var next in ack.LoadoutSelections) actions.Enqueue(next);
                            if (ack.Rewards.Count > 0) _rewardRetry = true;
                        }
                    }
                }
                lock (_gate) _selectionRetry = (_independentMatchCohorts && _selectionRetry) || retry || actions.Count > 0;
            }
            finally { _loadoutSelectionSettlement.Release(); }
        }
        async Task SettleRewardsAsync(GameplayBridgeResponse reply)
        {
            if (reply?.Ok != true || reply.Rewards.Count == 0) return;
            await _rewardAcknowledgements.WaitAsync();
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
                        if (reward.Kind == MagProgressionCatalog.RepairRewardKind && !owner.RepairRewardsEnabled)
                        { _rewardRetry = true; Logger.Warn("Gameplay reward rejected: repair rewards were disabled at admission"); return; }
                        if (MagProgressionCatalog.IsExtendedObjectiveRewardKind(reward.Kind) && !owner.ExtendedObjectiveRewardsEnabled)
                        { _rewardRetry = true; Logger.Warn("Gameplay reward rejected: extended objectives were disabled at admission"); return; }
                        if (reward.Frago == true && MagProgressionCatalog.IsFragoSupportKind(reward.Kind) && !owner.FragoActionBonusEnabled)
                        { _rewardRetry = true; Logger.Warn("Gameplay reward rejected: FRAGO support was disabled at admission"); return; }
                        if (reward.Frago == true && MagProgressionCatalog.IsFragoObjectiveKind(reward.Kind) && !owner.FragoObjectiveBonusEnabled)
                        { _rewardRetry = true; Logger.Warn("Gameplay reward rejected: FRAGO objectives were disabled at admission"); return; }
                        owners.Add((reward, owner));
                    }
                }
                bool retry = false; bool pending = false;
                foreach (var pair in owners)
                {
                    lock (_gate) if (_disposed && _independentMatchCohorts) return;
                    var reward = pair.reward; var owner = pair.owner;
                    try
                    {
                        // Replacement reservations share the surviving round's native XP carry and result basis.
                        // Serialize the durable apply against admission, then release the
                        // gate before acknowledging it to Rust over the network.
                        ProgressionRewardOutcome result;
                        await _rewardSettlement.WaitAsync();
                        try
                        {
                            result = _store.ApplyRewardForAccount(owner.Account, owner.CharacterId, reward.RewardId,
                                $"world-{reward.WorldId}-{reward.MatchId}", reward.Kind, checked((int)reward.Units), reward.Leadership, reward.Frago);
                        }
                        finally { _rewardSettlement.Release(); }
                        if (result.Result != MagNetResult.Success)
                        { retry = true; Logger.Warn($"Gameplay reward not committed: player={owner.Client.DmeId} reason={result.Reason}"); break; }
                        // Publish the latest account record, not an older receipt snapshot.
                        // A missed notification is also repaired by the next0238 query.
                        Action<MagCharacterRecord> notify = null;
                        if (result.ShouldNotifyCharacterRecord)
                        {
                            lock (_gate)
                            {
                                var currentOwner = _reservations.Values.FirstOrDefault(r => IsLiveCohort(r) && !r.Invalidated &&
                                    r.Account == owner.Account && r.CharacterId == owner.CharacterId);
                                var callback = currentOwner?.Notify ?? owner.Notify;
                                if (callback != null && _notifiedRewards.Add(reward.RewardId)) notify = callback;
                            }
                        }
                        if (notify != null)
                        {
                            var current = _store.ReadForAccount(owner.Account).Single(c => c.Id == owner.CharacterId).ToRecord();
                            notify(current);
                        }
                        var request = new Dictionary<string, object> {
                            ["v"] = 2, ["request_id"] = Id(), ["op"] = "progression_committed", ["reservation_id"] = owner.Id,
                            ["world_id"] = reward.WorldId, ["player_id"] = reward.PlayerId, ["reward_id"] = reward.RewardId,
                            ["progression"] = new { experience = (uint)result.Experience, level = result.Level, skill_points = result.SkillPoints },
                            ["match_experience"] = (uint)result.MatchExperience
                        };
                        if (result.MatchFragoPoints is int fragoPoints) request["match_frago_points"] = (uint)fragoPoints;
                        if (result.Leadership is LeadershipRewardOutcome leadership)
                        {
                            request["leadership"] = new GameplayLeadershipRecord(leadership);
                            if (leadership.MatchLeadershipPoints is uint matchPoints) request["match_leadership_points"] = matchPoints;
                        }
                        var ack = await _bridge.RequestProgressionAsync(request);
                        bool ackRouted;
                        lock (_gate) { ackRouted = Route(ack); if (!ackRouted) retry = true; else pending = ack.Rewards.Count > 0; }
                        if (ackRouted) await ObserveLoadoutSelectionsAsync(ack);
                        Logger.Info($"Gameplay reward committed: player={owner.Client.DmeId} kind={reward.Kind} units={reward.Units} xp={result.Experience} level={result.Level} skill_points={result.SkillPoints} match_xp={result.MatchExperience} duplicate={result.Duplicate}");
                    }
                    catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                    { retry = true; Logger.Warn($"Gameplay reward persistence deferred: player={owner.Client.DmeId} error={e.GetType().Name}"); break; }
                }
                lock (_gate) _rewardRetry = (_independentMatchCohorts && _rewardRetry) || retry || pending;
            }
            finally { _rewardAcknowledgements.Release(); }
        }
        public async Task TickAsync()
        {
            if (_disposed || !_ready) return;
            StartLoadoutRefreshPump();
            await ScheduleBridgeMaintenanceAsync();
            Reservation[] release;
            lock (_gate) release = _reservations.Values.Where(r => r.Invalidated || r.Deadline <= DateTime.UtcNow || r.Client.IsDestroyed || r.Client.Disconnected ||
                (r.Claimed && r.Client.Tcp != null && !r.Client.Tcp.Active)).ToArray();
            await ReleaseReservationsAsync(release);
            SendQueueNotices();StartMatchmakingPump();
            await DmeProgram.TcpServer.HandleIncomingMessages();
            DmeClient[] clients; lock (_gate) clients = _reservations.Values.Select(r => r.Client).ToArray();
            await Task.WhenAll(clients.Select(client => client.HandleIncomingMessages()));
            await DmeProgram.TcpServer.HandleOutgoingMessages();
            foreach (var client in clients) client.HandleOutgoingMessages();
        }
        Task ReleaseAsync(Reservation r) => ReleaseReservationAsync(r, true);

        async Task ReleaseReservationAsync(Reservation r, bool notifyGameplay)
        {
            PendingRelease pending = null;
            bool defer;
            lock (_gate)
            {
                // Only a definitive pre-admission refusal proves this identity
                // could never own a reward, even if local teardown won the race.
                if (!notifyGameplay) _rewardOwners.Remove(r.Id);
                if (!Current(r)) return;
                defer = IsResultsCohort(r);
                r.Invalidated = true;
                _reservations.Remove(r.Id);
                _loadoutSelections.Forget(r.Id);
                _loadoutRefreshes.Forget(r.Id);
                if (notifyGameplay)
                {
                    pending = new PendingRelease { Reservation = r, RequestId = Id() };
                    _pendingReleases.Add(r.Id, pending);
                }
                r.Client.DmeWorld.Manager.RemoveClient(r.Client);
                r.Client.DmeWorld.RemoveLocalClient(r.Client);
            }
            Logger.Warn($"Gameplay reservation released: player={r.Client.DmeId} reason={r.ReleaseReason ?? (r.Deadline <= DateTime.UtcNow ? "application_inactivity" : "transport_closed")}");
            await r.Client.Stop();
            if (!defer && pending != null) await RetryReleaseAsync(pending);
        }
        async Task RetryReleaseAsync(PendingRelease pending)
        {
            lock (_gate)
            {
                if (!_pendingReleases.TryGetValue(pending.Reservation.Id, out var current) || current != pending ||
                    pending.InFlight || pending.NextAttempt > DateTime.UtcNow) return;
                pending.InFlight = true;
            }
            try
            {
                var reply = await _bridge.RequestReleaseAsync(new { v = 2, request_id = pending.RequestId, op = "release", reservation_id = pending.Reservation.Id });
                bool routed;
                lock (_gate)
                {
                    routed = Route(reply);
                    if (routed) _pendingReleases.Remove(pending.Reservation.Id);
                    else
                    {
                        var now = DateTime.UtcNow;
                        bool transient = reply == null || reply.IsTransientFailure;
                        _rewardRetry = true;
                        if (transient)
                        {
                            pending.NextAttempt = now.AddSeconds(1);
                            Logger.Warn($"Gameplay release acknowledgement pending: player={pending.Reservation.Client.DmeId} retry=True failure={reply?.Failure.ToString() ?? "Unavailable"}");
                        }
                        else
                        {
                            // Rust answered but did not acknowledge (for example a reservation it
                            // already evicted). Never hold the account's admission forever.
                            pending.FirstRejected ??= now;
                            var next = NextRejectedReleaseAttempt(pending.FirstRejected.Value, now);
                            if (next == null)
                            {
                                _pendingReleases.Remove(pending.Reservation.Id);
                                Logger.Warn($"Gameplay release abandoned after rejection grace: player={pending.Reservation.Client.DmeId} error={reply.Error ?? "none"} failure={reply.Failure}");
                            }
                            else
                            {
                                pending.NextAttempt = next.Value;
                                Logger.Warn($"Gameplay release acknowledgement pending: player={pending.Reservation.Client.DmeId} retry=Rejected error={reply.Error ?? "none"} failure={reply.Failure} next_s={(int)RejectedReleaseRetryInterval.TotalSeconds}");
                            }
                        }
                    }
                }
                await ObserveReceiptsAsync(reply, routed);
            }
            finally { lock (_gate) pending.InFlight = false; }
        }
        public async ValueTask DisposeAsync()
        {
            lock (_gate) { if (_disposed) return; _disposed = true; _ready = false; foreach (var r in _reservations.Values) r.Invalidated = true; }
            await StopFactionMatchmakingAsync();
            _loadoutRefreshCancellation.Cancel();
            Task refresh; lock (_gate) refresh = _loadoutRefreshWorker;
            try { await refresh.WaitAsync(TimeSpan.FromSeconds(3)); }
            catch (TimeoutException) { Logger.Error("Active Armory refresh disposal exceeded its bounded RPC deadline."); }
            _worldStreamCancellation.Cancel(); await _worldStreamWorker;
            try { await Task.WhenAll(_receiptWorker, _releaseWorker).WaitAsync(TimeSpan.FromSeconds(5)); }
            catch (TimeoutException) { Logger.Error("Gameplay outbox disposal exceeded its bounded bridge deadline."); }
            _events.Writer.TryComplete(); _resultsEvents.Writer.TryComplete();
            await Task.WhenAll(_worker, _resultsEventWorker);
            Reservation[] release; lock (_gate) release = _reservations.Values.ToArray();
            await ReleaseReservationsAsync(release);
            if (_started && _independentMatchCohorts) await FlushRetiredReleasesAsync();
            if (World != null) { await World.Stop(); DmeProgram.Managers.Remove(_applicationId); }
            if (_started) await DmeProgram.TcpServer.Stop(); if (DmeProgram.LocalGameplay == this) DmeProgram.LocalGameplay = null;
        }
    }
}
