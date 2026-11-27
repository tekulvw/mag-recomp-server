using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace Server.Medius
{
    /// <summary>Where an account stands for an operator kill on one host.</summary>
    public enum LocalDmeAdminKillStatus
    {
        /// <summary>This host holds no reservation for the account.</summary>
        NotHere,
        /// <summary>A reservation exists but is not a placed, connected player of the running match
        /// (still joining, showing results, or released).</summary>
        NotInMatch,
        /// <summary>The world stream is down, so nothing can reach the worker now.</summary>
        WorkerUnavailable,
        /// <summary>The worker's hello did not list admin_kill_v1.</summary>
        Unsupported,
        Ready
    }

    /// <summary>The live player an operator kill names: its reservation, world and DME player slot.</summary>
    public sealed record LocalDmeAdminKillTarget(string Account, string ReservationId, uint WorldId, ushort PlayerId);

    public sealed partial class LocalDmeHost
    {
        /// <summary>The longest an operator kill waits behind queued client events before giving up unsent.</summary>
        public static readonly TimeSpan AdminKillQueueWait = TimeSpan.FromSeconds(2);

        bool _supportsAdminKill;

        /// <summary>An operator kill waiting in the event queue, so it reaches the worker in order with
        /// the players' own events and its replies are delivered in that order too.</summary>
        sealed class AdminKillWork
        {
            public object Message;
            public bool Started, Abandoned;
            public readonly TaskCompletionSource<GameplayBridgeResponse> Done =
                new TaskCompletionSource<GameplayBridgeResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
        }

        /// <summary>The account's placed player in this host's running match, if any; with
        /// <paramref name="characterId"/>, only while it plays that character.</summary>
        public (LocalDmeAdminKillStatus Status, LocalDmeAdminKillTarget Target) FindAdminKillTarget(string account, uint? characterId = null)
        {
            lock (_gate)
            {
                bool Named(string rowAccount, uint rowCharacter) =>
                    string.Equals(rowAccount, account, StringComparison.Ordinal) && (characterId == null || rowCharacter == characterId);
                var rows = _reservations.Values.Where(r => Named(r.Account, r.CharacterId)).ToArray();
                if (rows.Length == 0 && !_pendingReleases.Values.Any(r => Named(r.Reservation.Account, r.Reservation.CharacterId)))
                    return (LocalDmeAdminKillStatus.NotHere, null);
                var now = DateTime.UtcNow;
                var live = rows.FirstOrDefault(r => IsLiveCohort(r) && r.Accepted && r.Claimed && !r.Invalidated &&
                    r.TerminalMatchId == null && r.Deadline > now && r.Client != null && !r.Client.IsDestroyed &&
                    !r.Client.Disconnected && r.Client.IsAuthenticated && World != null);
                if (live == null) return (LocalDmeAdminKillStatus.NotInMatch, null);
                var target = new LocalDmeAdminKillTarget(live.Account, live.Id, (uint)World.WorldId, (ushort)live.Client.DmeId);
                if (!ReadyForGameplay) return (LocalDmeAdminKillStatus.WorkerUnavailable, target);
                if (!_supportsAdminKill) return (LocalDmeAdminKillStatus.Unsupported, target);
                return (LocalDmeAdminKillStatus.Ready, target);
            }
        }

        /// <summary>
        /// Ask the worker to apply <paramref name="target"/>'s KillSelf on the operator's behalf. The request
        /// joins this host's event queue and its replies are routed like an event's. Null when nothing was
        /// sent: the reservation changed, the host is not ready, or the queue did not reach it within
        /// <see cref="AdminKillQueueWait"/>. A transient failure in the reply means the outcome is unknown.
        /// </summary>
        public async Task<GameplayBridgeResponse> AdminKillAsync(LocalDmeAdminKillTarget target, string operatorName, string reason)
        {
            ArgumentNullException.ThrowIfNull(target);
            var work = new AdminKillWork();
            lock (_gate)
            {
                if (_disposed || !_supportsAdminKill || !ReadyForGameplay ||
                    !_reservations.TryGetValue(target.ReservationId, out var reservation) || reservation.Invalidated ||
                    !IsLiveCohort(reservation) || World == null || (uint)World.WorldId != target.WorldId ||
                    (ushort)reservation.Client.DmeId != target.PlayerId)
                    return null;
                var message = new System.Collections.Generic.Dictionary<string, object>
                {
                    ["v"] = 2, ["request_id"] = Id(), ["op"] = "admin_kill", ["reservation_id"] = target.ReservationId,
                    ["world_id"] = target.WorldId, ["player_id"] = target.PlayerId, ["operator"] = operatorName
                };
                if (!string.IsNullOrEmpty(reason)) message["reason"] = reason;
                work.Message = message;
                if (!_events.Writer.TryWrite((reservation, work))) return null;
            }
            var finished = await Task.WhenAny(work.Done.Task, Task.Delay(AdminKillQueueWait));
            if (finished != work.Done.Task)
            {
                lock (_gate)
                {
                    // Still queued: withdraw it so it can never apply after the operator was told it did not.
                    if (!work.Started) { work.Abandoned = true; return null; }
                }
            }
            return await work.Done.Task;
        }

        /// <summary>Run one queued operator kill from the event queue (see <see cref="RunEvents"/>).</summary>
        async Task RunAdminKillAsync(Reservation reservation, AdminKillWork work)
        {
            lock (_gate)
            {
                if (work.Abandoned) return;
                if (_disposed || !Current(reservation) || reservation.Invalidated || !ReadyForGameplay)
                { work.Done.TrySetResult(null); return; }
                work.Started = true;
            }
            GameplayBridgeResponse reply;
            try { reply = await _bridge.RequestAdminKillAsync(work.Message); }
            catch (Exception error)
            {
                Logger.Warn($"Operator kill failed: player={reservation.Client.DmeId} error={error.GetType().Name}");
                work.Done.TrySetResult(null);
                return;
            }
            bool routed = false;
            lock (_gate)
            {
                // A refusal applied nothing. Unlike a client event, it never invalidates the player.
                if (reply?.Ok == true)
                {
                    routed = Route(reply);
                    if (!routed) Logger.Warn($"Operator kill reply could not be routed: player={reservation.Client.DmeId}");
                }
            }
            work.Done.TrySetResult(reply);
            if (routed) await ObserveReceiptsAsync(reply, true);
        }
    }
}
