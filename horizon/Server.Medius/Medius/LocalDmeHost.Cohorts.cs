using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using DmeWorld = Server.Dme.Models.World;

namespace Server.Medius
{
    public sealed partial class LocalDmeHost
    {
        const int MaximumResultsCohorts = 8;
        bool _independentMatchCohorts;
        DmeWorld _admissionWorld;
        bool _admissionTerminal;
        DmeWorld AdmissionWorld => _admissionWorld ?? World;
        bool IsLiveCohort(Reservation r) => r.Client.DmeWorld == AdmissionWorld && (!_independentMatchCohorts || !_admissionTerminal);
        bool IsResultsCohort(Reservation r) => _independentMatchCohorts && !IsLiveCohort(r);
        bool IsArchivedTransport(Reservation r) => _independentMatchCohorts && r.Client.DmeWorld != AdmissionWorld;
        bool HasResultsCapacity() =>
            _reservations.Values.Select(r => r.Client.DmeWorld).Distinct().Count() <= MaximumResultsCohorts;

        // Called under _gate. Retained result transports occupy their own slot
        // namespace and continue to use the original subscribed wire world.
        bool AdvanceTerminalCohort()
        {
            if (!_admissionTerminal) return true;
            if (!HasResultsCapacity()) return false;
            foreach (var r in _reservations.Values.Where(r => r.Client.DmeWorld == AdmissionWorld))
            {
                _loadoutSelections.Forget(r.Id);
                _loadoutRefreshes.Forget(r.Id);
                _progressionRefreshes.Remove(r.Id);
            }
            _admissionWorld = World.CreateLocalCohort();
            _admissionTerminal = false;
            return true;
        }
        async Task ReleasePriorResultsAsync(string account, uint character)
        {
            Reservation[] results;
            lock (_gate) results = _reservations.Values.Where(r => r.Account == account &&
                (IsResultsCohort(r) || (r.CharacterId == character && r.TerminalMatchId != null))).ToArray();
            foreach (var r in results) { r.ReleaseReason = "results_requeue"; await ReleaseAsync(r); }
        }

        async Task ReleaseReservationsAsync(Reservation[] reservations)
        {
            if (InRegistry)
            {
                await Task.WhenAll(reservations.Select(ReleaseAsync));
                return;
            }
            // Retired transports already defer their remote release. Close them
            // concurrently so their separate TCP/UDP shutdowns do not accumulate.
            Reservation[] results;
            lock (_gate) results = reservations.Where(IsResultsCohort).ToArray();
            var closeResults = Task.WhenAll(results.Select(ReleaseAsync));
            try
            {
                foreach (var reservation in reservations.Except(results)) await ReleaseAsync(reservation);
            }
            finally { await closeResults; }
        }
        bool TryFindTerminalMembers(GameplayMatchTerminal terminal, out Reservation[] completed)
        {
            completed = Array.Empty<Reservation>();
            if (terminal != null)
            {
                if (terminal.WorldId != (uint)World.WorldId) return false;
                completed = terminal.ReservationIds.Select(id => _reservations.GetValueOrDefault(id) ??
                    _pendingReleases.GetValueOrDefault(id)?.Reservation).Where(r => r != null).ToArray();
                if (_independentMatchCohorts && (completed.Select(r => r.Client.DmeWorld).Distinct().Count() > 1 ||
                    completed.Any(r => r.TerminalMatchId != null && r.TerminalMatchId != terminal.MatchId))) return false;
            }
            return true;
        }
    }
}
