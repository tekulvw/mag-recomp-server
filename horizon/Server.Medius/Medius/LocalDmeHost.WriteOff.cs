using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;

namespace Server.Medius
{
    /// <summary>What writing off one worker incarnation released and lost.</summary>
    public sealed record GameplayIncarnationWriteOff(string WorkerId, string IncarnationId, string FormationId, string ProfileId,
        int ReleasedPlayers, int DroppedReleases, int LostRewards, int LostLoadoutSelections, int UntrackedReceipts);

    public sealed partial class LocalDmeHost
    {
        static readonly TimeSpan WriteOffSettlementWait = TimeSpan.FromSeconds(10);
        readonly GameplayReceiptLedger _receipts = new();
        bool _writtenOff;

        /// <summary>The contract the connected incarnation advertised in its startup discovery.</summary>
        public MagWorkerContract WorkerContract { get; private set; }

        /// <summary>Remember receipts a configured worker reported. Called with _gate held after
        /// Route accepted the reply, so they belong to this host's incarnation.</summary>
        void NoteReceipts(GameplayBridgeResponse reply)
        {
            if (!InRegistry || reply == null) return;
            foreach (var reward in reply.Rewards)
                _receipts.Observe(GameplayReceiptLedger.RewardKey(reward), new GameplayReceiptLedger.Receipt(
                    reward.Kind, reward.RewardId, reward.AccountId, reward.CharacterId, reward.Units.ToString(System.Globalization.CultureInfo.InvariantCulture),
                    reward.MatchId, true));
            foreach (var selection in reply.LoadoutSelections)
                _receipts.Observe(GameplayReceiptLedger.SelectionKey(selection), new GameplayReceiptLedger.Receipt(
                    "loadout_selection", $"{selection.SelectionId}@{selection.Revision}", selection.AccountId, selection.CharacterId,
                    $"inventory_slot={selection.InventoryIndex}", null, false));
        }

        /// <summary>Write off this host's worker incarnation after another incarnation answered at
        /// its endpoint. The old process and everything it held are gone, so nothing is sent to it:
        /// every reservation is released locally through the ordinary teardown path (its DME
        /// connection closes), pending remote releases are dropped, queued tickets return to the
        /// registry, the match cohort is marked terminal and every reported but unsettled receipt
        /// is logged as lost. Returns null when the incarnation was already written off.</summary>
        public async Task<GameplayIncarnationWriteOff> WriteOffIncarnationAsync(string replacementIncarnation)
        {
            if (!InRegistry) throw new InvalidOperationException("Only a configured worker incarnation can be written off.");
            // Let an in-flight durable commit finish first, so a receipt that just became durable
            // is not reported lost. Both waits are bounded; the bridge to a gone process fails fast.
            bool rewards = await _rewardAcknowledgements.WaitAsync(WriteOffSettlementWait);
            bool selections = await _loadoutSelectionSettlement.WaitAsync(WriteOffSettlementWait);
            try
            {
                Reservation[] reservations;
                int dropped;
                string formation, profile;
                (IReadOnlyList<GameplayReceiptLedger.Receipt> Open, int Untracked) lost;
                lock (_gate)
                {
                    if (_writtenOff) return null;
                    _writtenOff = true;
                    _ready = false;
                    // The cohort can never finish: no further admission, results or rewards.
                    if (_independentMatchCohorts) _admissionTerminal = true;
                    _rewardRetry = false;
                    _selectionRetry = false;
                    formation = _formationId;
                    profile = _formedProfile?.ProfileId;
                    reservations = _reservations.Values.ToArray();
                    foreach (var reservation in reservations) { reservation.Invalidated = true; reservation.ReleaseReason = "worker_restarted"; }
                    dropped = _pendingReleases.Count;
                    _pendingReleases.Clear();
                    _provisionalPartyAccounts.Clear();
                    foreach (var ticket in _humanQueue.Snapshot()) _humanQueue.Remove(ticket);
                    lost = _receipts.Drain();
                }
                await Task.WhenAll(reservations.Select(reservation => ReleaseReservationAsync(reservation, false)));
                string worker = _options.WorkerId, incarnation = _bridge.WorkerIdentity?.IncarnationId ?? "none";
                foreach (var receipt in lost.Open)
                    Logger.Warn($"Gameplay receipt lost with worker incarnation: worker={worker} incarnation={incarnation} account={receipt.Account} character={receipt.Character} kind={receipt.Kind} amount={receipt.Amount} receipt={receipt.Id} match={receipt.Match ?? "none"}");
                var result = new GameplayIncarnationWriteOff(worker, incarnation, formation, profile, reservations.Length, dropped,
                    lost.Open.Count(r => r.Reward), lost.Open.Count(r => !r.Reward), lost.Untracked);
                Logger.Warn($"Configured worker incarnation written off: worker={worker} incarnation={incarnation} replaced_by={replacementIncarnation ?? "none"} cohort=terminal formation={formation ?? "none"} profile={profile ?? "none"} released_players={result.ReleasedPlayers} dropped_releases={dropped} lost_rewards={result.LostRewards} lost_loadout_selections={result.LostLoadoutSelections} untracked_receipts={result.UntrackedReceipts}; receipts the worker never reported cannot be listed");
                return result;
            }
            finally
            {
                if (selections) _loadoutSelectionSettlement.Release();
                if (rewards) _rewardAcknowledgements.Release();
            }
        }
    }
}
