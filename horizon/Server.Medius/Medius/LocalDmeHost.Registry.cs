using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using DmeWorld = Server.Dme.Models.World;

namespace Server.Medius
{
    [Flags]
    public enum LocalDmeAccountOwnership
    {
        None = 0,
        Queued = 1,
        Live = 2,
        PendingRelease = 4,
        Terminal = 8
    }

    public sealed record LocalDmeWorkerPlacement(bool Ready, bool Draining, bool CanForm,
        IReadOnlyList<MagMatchProfile> Profiles, MagMatchStartupPolicy StartupPolicy,
        IReadOnlyList<int> MinimumPlayersPerTeam, MagMatchProfile CurrentProfile,
        IReadOnlyList<int> LiveTeamCounts, int LiveCount, IReadOnlyList<int> PendingTeamCounts, int PendingQueueCount);

    public sealed partial class LocalDmeHost
    {
        static readonly object WorldRegistrationGate = new object();
        static readonly TimeSpan WorldStreamLease = TimeSpan.FromSeconds(3);
        readonly TimeProvider _timeProvider;
        long _lastWorldFrameTimestamp;
        bool _operatorDraining;
        Func<bool> _readDrainPolicy;
        bool _registryFormationStaged;
        int _registryExpectedParties;
        int _registryBatchDepth;
        Task _worldCleanupWorker = Task.CompletedTask;

        bool InRegistry => _options.WorkerId != null;
        bool Draining => _operatorDraining || (InRegistry && (_bridge.HasWorkerIdentityFault || _readDrainPolicy?.Invoke() == true));
        bool HasWorldStreamLease => _worldStreamHealthy && (!InRegistry ||
            (!_bridge.HasWorkerIdentityFault && _timeProvider.GetElapsedTime(_lastWorldFrameTimestamp) < WorldStreamLease));
        // A legacy stream that is resuming keeps its sessions usable (client events
        // still reach the worker) but admits no one until frames flow again.
        bool ReadyForGameplay => _ready && !_disposed && (HasWorldStreamLease || (_worldStreamResuming && !InRegistry));
        bool ReadyForAdmission => _ready && !_disposed && HasWorldStreamLease && !Draining;

        public void SetDraining(bool draining)
        {
            if (!InRegistry) throw new InvalidOperationException("Drain requires a configured gameplay worker.");
            lock (_gate) _operatorDraining = draining;
        }

        /// <summary>Read the registry's published drain policy without taking a registry lock.</summary>
        public void SetDrainPolicy(Func<bool> read)
        {
            if (!InRegistry) throw new InvalidOperationException("Drain requires a configured gameplay worker.");
            ArgumentNullException.ThrowIfNull(read);
            lock (_gate) _readDrainPolicy = read;
        }

        sealed class RegistryBatchLease : IDisposable
        {
            LocalDmeHost _host;
            public RegistryBatchLease(LocalDmeHost host) => _host = host;
            public void Dispose()
            {
                var host = Interlocked.Exchange(ref _host, null);
                if (host != null) lock (host._gate) host._registryBatchDepth--;
            }
        }

        /// <summary>Prevent a tick from consuming an incomplete registry cohort.</summary>
        public IDisposable BeginRegistryBatch()
        {
            if (!InRegistry) throw new InvalidOperationException("A registry batch requires a configured gameplay worker.");
            lock (_gate) _registryBatchDepth++;
            return new RegistryBatchLease(this);
        }

        public async Task StartInRegistryAsync(Server.Dme.MediusManager manager)
        {
            if (!InRegistry || manager?.LocalOnly != true || manager.ApplicationId != _applicationId)
                throw new ArgumentException("Configured gameplay requires the local DME manager for its application.", nameof(manager));
            BeginStart();
            try
            {
                var hello = await _bridge.RequestHelloAsync(new { v = 2, request_id = Id(), op = "hello" });
                InitializeGameplayHello(hello);
                if (!_independentMatchCohorts || !UsesFactionMatchmaking || !hello.SupportsWorkerRegistry ||
                    _bridge.WorkerIdentity?.WorkerId != _options.WorkerId)
                    throw new IOException("Configured gameplay requires worker identity, faction profiles and independent result cohorts.");
                CreateWorld(manager);
                await StartWorldStreamAsync();
                lock (_gate) _ready = true;
            }
            catch (TimeoutException error)
            {
                await DisposeAsync();
                throw new IOException("Configured gameplay did not establish its ordered world stream.", error);
            }
            catch { await DisposeAsync(); throw; }
        }

        void CreateWorld(Server.Dme.MediusManager manager)
        {
            lock (WorldRegistrationGate) World = new DmeWorld(manager, _applicationId, _maximumPlayers);
        }

        public LocalDmeWorkerPlacement GetPlacement()
        {
            lock (_gate)
            {
                PurgeHumanQueue();
                var pending = _humanQueue.Snapshot();
                var liveCounts = new int[3];
                var pendingCounts = new int[3];
                var liveAccounts = new HashSet<string>(StringComparer.Ordinal);
                foreach (var reservation in _reservations.Values.Where(IsLiveCohort))
                {
                    liveCounts[reservation.Faction]++;
                    liveAccounts.Add(reservation.Account);
                }
                // A partially admitted ticket is still owned by the queue until
                // its callback runs. Count that slot only once for capacity.
                foreach (var ticket in pending)
                    if (!liveAccounts.Contains(ticket.Value.Account)) pendingCounts[ticket.Human.Faction] += ticket.Human.Size;
                int live = liveCounts.Sum();
                bool ready = ReadyForGameplay;
                bool draining = Draining;
                return new LocalDmeWorkerPlacement(ready, draining, CanStageRegistryFormation(ready && !draining),
                    _matchProfiles, _formationStartup ?? _startupPolicy,
                    Array.AsReadOnly(_formationTeamMinima ?? new[] { 0, 0 }),
                    live > 0 && _formationConfigured ? _formedProfile : null,
                    Array.AsReadOnly(liveCounts), live, Array.AsReadOnly(pendingCounts), pending.Count);
            }
        }

        public LocalDmeAccountOwnership GetAccountOwnership(string account)
        {
            lock (_gate)
            {
                PurgeHumanQueue();
                var ownership = _humanQueue.Snapshot().Any(ticket => ticket.Value.Accounts.Contains(account, StringComparer.Ordinal))
                    ? LocalDmeAccountOwnership.Queued : LocalDmeAccountOwnership.None;
                if (_provisionalPartyAccounts.Contains(account)) ownership |= LocalDmeAccountOwnership.PendingRelease;
                foreach (var reservation in _reservations.Values.Where(r => r.Account == account))
                    ownership |= IsResultsCohort(reservation) ? LocalDmeAccountOwnership.Terminal : LocalDmeAccountOwnership.Live;
                foreach (var release in _pendingReleases.Values.Where(r => r.Reservation.Account == account))
                    ownership |= IsResultsCohort(release.Reservation)
                        ? LocalDmeAccountOwnership.Terminal : LocalDmeAccountOwnership.PendingRelease;
                return ownership;
            }
        }

        /// <summary>Detach this account's retired transports before another worker allocates the session key.</summary>
        public Task ReleaseTerminalAccountAsync(string account)
        {
            Reservation[] results;
            lock (_gate)
            {
                results = _reservations.Values.Where(r => r.Account == account && IsResultsCohort(r)).ToArray();
                foreach (var reservation in results) reservation.ReleaseReason = "results_requeue";
            }
            return Task.WhenAll(results.Select(ReleaseAsync));
        }

        // Called with _gate held. The registry stages the exact winning profile
        // before forwarding the selected tickets and advancing the world tick.
        bool CanStageRegistryFormation(bool canAdmit) => InRegistry && canAdmit && UsesFactionMatchmaking &&
            _matchmakingWorker.IsCompleted && DateTime.UtcNow >= _nextFormationAttempt &&
            !_registryFormationStaged && _humanQueue.Count == 0 &&
            !_reservations.Values.Any(IsLiveCohort) && !_pendingReleases.Values.Any(r => !IsResultsCohort(r.Reservation)) &&
            (!_admissionTerminal || HasResultsCapacity());

        public bool TryStageRegistryFormation(string profileId, int expectedParties = 0)
        {
            lock (_gate)
            {
                PurgeHumanQueue();
                if (expectedParties < 0 || !CanStageRegistryFormation(ReadyForAdmission)) return false;
                var profile = _matchProfiles.FirstOrDefault(p => p.ProfileId == profileId);
                if (profile == null || (expectedParties > 0 && (!_supportsPartyReservations || profile.SquadCounts == null ||
                    expectedParties > profile.SquadCounts.Sum())) || !AdvanceTerminalCohort()) return false;
                ResetFormation();
                _formedProfile = profile;
                _formationId = Id();
                _registryFormationStaged = true;
                _registryExpectedParties = expectedParties;
                return true;
            }
        }
    }
}
