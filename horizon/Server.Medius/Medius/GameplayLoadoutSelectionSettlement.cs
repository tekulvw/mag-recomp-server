using System;
using System.Collections.Generic;
using System.IO;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Current-owner, latest-revision settlement for committed selections.
    /// Settle locks the supplied owner gate across its current-owner check and save;
    /// LocalDmeHost passes the same gate that serializes reservation release.
    /// No client packet is replayed; repeated receipts only repeat their acknowledgement.</summary>
    public sealed class GameplayLoadoutSelectionSettlement
    {
        sealed class Receipt
        {
            public GameplayLoadoutSelection Action;
            public string Status;
            public int Attempts;
            public DateTime NextAttempt;
        }
        readonly object _ownerGate;
        public GameplayLoadoutSelectionSettlement(object ownerGate) { _ownerGate = ownerGate ?? throw new ArgumentNullException(nameof(ownerGate)); }
        readonly Dictionary<string, Receipt> _latest = new Dictionary<string, Receipt>(StringComparer.Ordinal);
        public const int MaximumAttempts = 3;

        public string Settle(GameplayLoadoutSelection action, DateTime now, Func<GameplayLoadoutSelection, bool> currentOwner,
            Func<GameplayLoadoutSelection, CharacterCreationOutcome> save)
        {
            lock (_ownerGate) return SettleUnderGate(action, now, currentOwner, save);
        }

        string SettleUnderGate(GameplayLoadoutSelection action, DateTime now, Func<GameplayLoadoutSelection, bool> currentOwner,
            Func<GameplayLoadoutSelection, CharacterCreationOutcome> save)
        {
            if (action == null) throw new ArgumentNullException(nameof(action));
            if (!currentOwner(action)) return "rejected";
            if (_latest.TryGetValue(action.ReservationId, out var receipt))
            {
                if (action.Revision < receipt.Action.Revision) return "rejected";
                if (action.Revision == receipt.Action.Revision)
                {
                    if (!action.Same(receipt.Action)) return "rejected";
                    if (receipt.Status != null) return receipt.Status;
                    if (now < receipt.NextAttempt) return null;
                }
                else
                {
                    if (action.SelectionId == receipt.Action.SelectionId) return "rejected";
                    receipt = null;
                }
            }
            if (receipt == null)
            {
                receipt = new Receipt { Action = action };
                _latest[action.ReservationId] = receipt;
            }
            receipt.Attempts++;
            try
            {
                receipt.Status = save(action).Result == MagNetResult.Success ? "persisted" : "rejected";
            }
            catch (InvalidDataException) { receipt.Status = "rejected"; }
            catch (UnauthorizedAccessException) { receipt.Status = "rejected"; }
            catch (IOException)
            {
                if (receipt.Attempts >= MaximumAttempts) receipt.Status = "rejected";
                else receipt.NextAttempt = now.AddSeconds(1);
            }
            return receipt.Status;
        }

        public void Forget(string reservationId) { lock (_ownerGate) _latest.Remove(reservationId); }
    }
}
