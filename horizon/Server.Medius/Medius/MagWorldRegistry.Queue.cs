using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Threading;
using System.Threading.Tasks;
using RT.Models.MAG;

namespace Server.Medius
{
    public sealed partial class MagWorldRegistry
    {
        sealed class QueueEntry
        {
            public MagMlsSession Session; public MagJoinGameRequest Request; public string Account; public uint Character;
            public byte Faction; public Func<bool> Connected; public Action<MagPluginClientMessage> Send; public Action<MagCharacterRecord> Notify;
            public Slot Assigned; public int Cancelled; public bool Prepared; public Task Preparation;
            public bool Alive => Volatile.Read(ref Cancelled) == 0 && Connected() && Session.State == MagMlsSession.SessionState.Ready &&
                !Session.IsExpired(DateTime.UtcNow) && Session.AuthenticatedAccount == Account && Session.CharacterId == Character;
        }
        readonly MagPendingHumanQueue<QueueEntry> _humans = new();
        readonly Dictionary<string,QueueEntry> _acknowledged = new(StringComparer.Ordinal);
        Task _placementWorker = Task.CompletedTask;
        DateTime _nextNotice;

        public void QueueJoin(MagMlsSession session, MagJoinGameRequest request, Action<MagPluginClientMessage> send,
            Func<bool> connected, Action<MagCharacterRecord> notify = null)
        {
            try
            {
                if (LocalDmeHost.JoinSessionRejection(session) != null || session.ApplicationId != _config.ApplicationId ||
                    LocalDmeHost.JoinRequestRejection(request, true, mode => mode <= 2) != null || !connected())
                { send(JoinResponse(request.TransactionId, 2)); return; }
                if (_slots.Any(slot => (slot.Host.GetAccountOwnership(session.AuthenticatedAccount) & LocalDmeAccountOwnership.Live) != 0))
                { send(JoinResponse(request.TransactionId, 6)); return; }
                var character = _store.ReadForAccount(session.AuthenticatedAccount).SingleOrDefault(c => c.Id == session.CharacterId);
                if (character == null || character.Faction > 2) { send(JoinResponse(request.TransactionId, 2)); return; }
                var value = new QueueEntry { Session = session, Request = request, Account = session.AuthenticatedAccount, Character = session.CharacterId,
                    Faction = character.Faction, Connected = connected, Send = send, Notify = notify };
                QueueEntry previous;
                lock (_gate)
                {
                    if (_disposed) { send(JoinResponse(request.TransactionId, 6)); return; }
                    previous = _humans.Snapshot().FirstOrDefault(e => e.Owner == value.Account)?.Value;
                    if (previous != null) Volatile.Write(ref previous.Cancelled, 1);
                    _humans.Put(value.Account, (int)request.Value, value.Faction, value);
                    if(_acknowledged.Remove(value.Account,out var accepted))Volatile.Write(ref accepted.Cancelled,1);
                }
                previous?.Assigned?.Host.CancelQueuedJoin(previous.Session,previous.Request);
                SendQueueNotices(true); StartPlacement();
            }
            catch (Exception error) when (error is IOException || error is UnauthorizedAccessException || error is ArgumentException || error is InvalidOperationException)
            { send(JoinResponse(request.TransactionId, 2)); }
        }
        public void CancelQueuedJoin(MagMlsSession session)
        {
            if(session==null)return;
            QueueEntry owner=null;
            lock (_gate)
            {
                var ticket=_humans.Snapshot().FirstOrDefault(e=>ReferenceEquals(e.Value.Session,session));
                if(ticket!=null) {_humans.Remove(ticket);owner=ticket.Value;}
                else if(_acknowledged.TryGetValue(session.AuthenticatedAccount,out var accepted) && ReferenceEquals(accepted.Session,session)) {
                    owner=accepted;_acknowledged.Remove(accepted.Account);
                }
                if(owner!=null)Volatile.Write(ref owner.Cancelled,1);
            }
            owner?.Assigned?.Host.CancelQueuedJoin(session,owner.Request);
        }
        void StartPlacement()
        {
            lock (_gate) if (!_disposed && _initialized && _humans.Count != 0 && _placementWorker.IsCompleted)
                _placementWorker = Task.Run(async()=> {
                    try {await PlaceWaitingAsync();}
                    catch(Exception error) {Logger.Error($"Registry placement deferred: error={error.GetType().Name}");}
                });
        }
        async Task PrepareTerminalTransferAsync(QueueEntry entry)
        {
            try
            {
                await Task.WhenAll(_slots.Select(slot => slot.Host.ReleaseTerminalAccountAsync(entry.Account)));
                lock (_gate) entry.Prepared = true;
            }
            catch (Exception error) { Logger.Warn($"Terminal transfer deferred: error={error.GetType().Name}"); }
        }
        async Task PlaceWaitingAsync()
        {
            await Task.Yield();
            MagPendingHumanQueue<QueueEntry>.Entry[] waiting;
            lock (_gate) waiting = _humans.Snapshot().ToArray();
            KeyValuePair<string,QueueEntry>[] acknowledged;
            lock(_gate)acknowledged=_acknowledged.ToArray();
            foreach(var item in acknowledged)if(item.Value.Assigned.Host.GetAccountOwnership(item.Key)==LocalDmeAccountOwnership.None) {
                lock(_gate)if(_acknowledged.GetValueOrDefault(item.Key)==item.Value)_acknowledged.Remove(item.Key);
            }
            foreach (var ticket in waiting)
            {
                var entry = ticket.Value;
                if (!entry.Alive)
                {
                    lock (_gate) _humans.Remove(ticket);
                    entry.Assigned?.Host.CancelQueuedJoin(entry.Session,entry.Request); continue;
                }
                if (entry.Assigned != null)
                {
                    var state = entry.Assigned.Host.GetPlacement();
                    var owned = entry.Assigned.Host.GetAccountOwnership(entry.Account);
                    if (!state.Ready || state.Draining || entry.Assigned.Faulted || (owned & LocalDmeAccountOwnership.Queued) == 0)
                    {
                        if ((owned & LocalDmeAccountOwnership.Live) != 0) continue;
                        entry.Assigned.Host.CancelQueuedJoin(entry.Session,entry.Request);
                        lock (_gate) if (_humans.IsCurrent(ticket)) entry.Assigned = null;
                    }
                }
                if (entry.Assigned == null && !entry.Prepared && (entry.Preparation == null || entry.Preparation.IsCompleted))
                {
                    if (_slots.Any(slot => (slot.Host.GetAccountOwnership(entry.Account) & LocalDmeAccountOwnership.Terminal) != 0))
                        entry.Preparation = PrepareTerminalTransferAsync(entry);
                    else entry.Prepared = true;
                }
            }
            // Each pass can stage at most one batch per configured world. No
            // bridge reply is awaited by this global queue or the MLS keepalive.
            for (int attempt = 0; attempt < _slots.Length; attempt++)
            {
                lock (_gate) { if (_disposed) return; waiting = _humans.Snapshot().Where(e => e.Value.Assigned == null && e.Value.Prepared && e.Value.Alive).ToArray(); }
                var eligible = waiting.Where(t => !_slots.Any(slot => (slot.Host.GetAccountOwnership(t.Owner) &
                    (LocalDmeAccountOwnership.Live | LocalDmeAccountOwnership.PendingRelease | LocalDmeAccountOwnership.Queued)) != 0)).ToArray();
                if (eligible.Length == 0) return;
                var states = _slots.Select(slot => {
                    var s = slot.Host.GetPlacement();
                    return new MagWorldPlacementState(slot.Config.WorkerId, s.Ready && !slot.Faulted, s.Draining || slot.Faulted,
                        s.StartupPolicy.MinimumPlayers, s.StartupPolicy.MaximumPlayers, s.MinimumPlayersPerTeam.ToArray(), s.Profiles, s.CurrentProfile,
                        s.LiveTeamCounts, s.PendingTeamCounts, s.LiveCount, s.PendingTeamCounts.Sum(), s.CanForm);
                }).ToArray();
                var plan = MagWorldPlacement.PlanNext(eligible.Select(e => e.Human).ToArray(), states, RandomNumberGenerator.GetInt32);
                if (plan == null) return;
                var target = _slots.Single(s => s.Config.WorkerId == plan.WorkerId);
                using var batch=target.Host.BeginRegistryBatch();
                if (plan.NewFormation && !target.Host.TryStageRegistryFormation(plan.Profile.ProfileId)) continue;
                var ids = plan.Players.Select(p => p.TicketId).ToHashSet(StringComparer.Ordinal);
                foreach (var ticket in eligible.Where(e => ids.Contains(e.Human.TicketId)))
                {
                    var entry = ticket.Value;
                    lock (_gate) { if (_disposed || !_humans.IsCurrent(ticket) || !entry.Alive) continue; entry.Assigned = target; }
                    target.Host.QueueJoin(entry.Session, entry.Request, message => ForwardReply(ticket, target, message), () => entry.Alive, entry.Notify);
                }
            }
        }
        void ForwardReply(MagPendingHumanQueue<QueueEntry>.Entry ticket, Slot target, MagPluginClientMessage message)
        {
            if (message is MagRetailJoinGameUpdate) return; // Global notices carry aggregate population.
            bool deliver = false, cleanup = false;
            lock (_gate)
            {
                bool current = !_disposed && _humans.IsCurrent(ticket) && ticket.Value.Alive && ticket.Value.Assigned == target;
                if (message is MagJoinGameResponse response)
                {
                    cleanup = !current && response.Result == 0;
                    if (current && response.Result == 6) ticket.Value.Assigned = null;
                    else if (current) {
                        if(response.Result==0)_acknowledged[ticket.Owner]=ticket.Value;
                        _humans.Remove(ticket); deliver = true;
                    }
                }
                else deliver = current;
            }
            if (cleanup) target.Host.CancelQueuedJoin(ticket.Value.Session,ticket.Value.Request);
            if (deliver && ticket.Value.Alive) ticket.Value.Send(message);
        }
        void SendQueueNotices(bool force = false)
        {
            QueueEntry[] entries; var counts = new uint[12];
            lock (_gate)
            {
                if (_disposed || (!force && DateTime.UtcNow < _nextNotice)) return;
                _nextNotice = DateTime.UtcNow.AddSeconds(5);
                entries = _humans.Snapshot().Select(e => e.Value).Where(e => e.Alive).ToArray();
                foreach (var entry in entries) counts[(int)entry.Request.Value * 3 + entry.Faction]++;
            }
            foreach (var entry in entries) if (entry.Alive) entry.Send(new MagRetailJoinGameUpdate {
                TransactionId = entry.Request.TransactionId, Success = true, QueuedType = 0, QueueSizes = (uint[])counts.Clone()
            });
        }
        static MagJoinGameResponse JoinResponse(int transaction, byte result) => new MagRetailJoinGameResponse { TransactionId = transaction, Result = result };
    }
}
