using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using RT.Models.MAG;

namespace Server.Medius
{
    public sealed partial class MagWorldRegistry
    {
        readonly MagPartyDirectory _parties;
        readonly Dictionary<MagMlsSession, MagPartyDirectory.Member> _partySessions = new();
        readonly Dictionary<MagPartyDirectory.Member, PartySession> _partyOwners = new(ReferenceEqualityComparer.Instance);
        readonly Dictionary<uint, (QueueEntry Entry, Slot Target)> _publishedParties = new();
        DateTime _nextPartyStatusCheck;
        sealed record PartySession(MagMlsSession Session, uint Character, Action<MagMessage> Send,
            Action<MagCharacterRecord> Notify);

        public bool PartyMembershipEnabled => _parties != null;

        public void RegisterPartySession(MagMlsSession session, Func<bool> connected, Action<MagMessage> send,
            Action<MagCharacterRecord> notify)
        {
            if (_parties == null || session?.ApplicationId != 20231 || session.State != MagMlsSession.SessionState.Ready ||
                session.IsExpired(DateTime.UtcNow) || !connected()) return;
            lock (_gate)
            {
                if (_disposed || _partySessions.ContainsKey(session)) return;
                try
                {
                    string account = session.AuthenticatedAccount;
                    uint characterId = session.CharacterId;
                    var character = _store.ReadForAccount(account).SingleOrDefault(row => row.Id == characterId);
                    if (character == null) return;
                    bool Current() => connected() && session.State == MagMlsSession.SessionState.Ready &&
                        !session.IsExpired(DateTime.UtcNow) && session.AuthenticatedAccount == account && session.CharacterId == characterId;
                    void Deliver(MagMessage reply)
                    {
                        if (!Current()) return;
                        try { send(reply); }
                        catch (Exception error) { Logger.Warn($"Retail party delivery failed: error={error.GetType().Name}"); }
                    }
                    var member = new MagPartyDirectory.Member(account, session.AccountId, character.Faction, Current, Deliver);
                    if (!_parties.Register(member)) return;
                    _partySessions.Add(session, member);
                    _partyOwners.Add(member, new PartySession(session, characterId, Deliver, notify));
                }
                catch (Exception error) when (error is IOException || error is UnauthorizedAccessException || error is ArgumentException)
                { Logger.Warn($"Retail party session unavailable: error={error.GetType().Name}"); }
            }
        }

        public bool RoutePartyMembership(MagMlsSession session, MagRetailPartyMessage message)
        {
            lock (_gate)
                return !_disposed && _parties != null && _partySessions.TryGetValue(session, out var member) &&
                    _parties.Apply(member, message, DateTime.UtcNow);
        }

        public bool CancelPartyQueue(MagMlsSession session, uint partyId)
        {
            lock (_gate)
            {
                var ticket = _humans.Snapshot().FirstOrDefault(row => row.Value.PartyId == partyId &&
                    ReferenceEquals(row.Value.Session, session));
                if (ticket == null || ticket.Value.Party == null || !PartyTicketCurrent(ticket)) return false;
                RemoveTicket(ticket);
                return true;
            }
        }

        bool PartyTicketCurrent(MagPendingHumanQueue<QueueEntry>.Entry ticket)
        {
            lock (_gate)
            {
                var entry = ticket.Value;
                if (entry.Party == null) return true;
                var members = _parties.GetLeaderMembers(entry.Party[0], entry.PartyId);
                return members != null && members.SequenceEqual(entry.Party, ReferenceEqualityComparer.Instance);
            }
        }

        void RemoveTicket(MagPendingHumanQueue<QueueEntry>.Entry ticket)
        {
            if (!_humans.Remove(ticket)) return;
            var entry = ticket.Value;
            System.Threading.Volatile.Write(ref entry.Cancelled, 1);
            if (entry.Party != null)
                _parties.SetQueueState(entry.Party[0], entry.PartyId, entry.Party, 1, -1);
        }

        bool WithPartyOwnership(MagPendingHumanQueue<QueueEntry>.Entry ticket, Slot target, Func<bool> next)
        {
            lock (_gate)
                return !_disposed && _humans.IsCurrent(ticket) && ticket.Value.Alive &&
                    ticket.Value.Assigned == target && PartyTicketCurrent(ticket) && next();
        }

        void CompleteParty(MagPendingHumanQueue<QueueEntry>.Entry ticket, Slot target,
            IReadOnlyList<MagJoinGameResponse> replies)
        {
            lock (_gate)
            {
                var entry = ticket.Value;
                if (!_humans.IsCurrent(ticket) || entry.Assigned != target) return;
                if (replies == null)
                {
                    entry.Assigned = null;
                    return;
                }
                _humans.Remove(ticket);
                _publishedParties[entry.PartyId] = (entry, target);
                _parties.SetQueueState(entry.Party[0], entry.PartyId, entry.Party, 3, (sbyte)entry.Request.Value);
                for (int i = 0; i < replies.Count; i++) entry.PartySessions[i].Send(replies[i]);
            }
        }

        void RefreshPublishedParties(DateTime now)
        {
            KeyValuePair<uint, (QueueEntry Entry, Slot Target)>[] published;
            lock (_gate)
            {
                if (now < _nextPartyStatusCheck) return;
                _nextPartyStatusCheck = now.AddSeconds(5);
                published = _publishedParties.ToArray();
            }
            foreach (var item in published)
            {
                var (entry, target) = item.Value;
                bool anyLive = entry.Accounts.Any(account => (target.Host.GetAccountOwnership(account) &
                    (LocalDmeAccountOwnership.Live | LocalDmeAccountOwnership.PendingRelease)) != 0);
                lock (_gate)
                {
                    if (_publishedParties.GetValueOrDefault(item.Key).Entry != entry ||
                        (anyLive && entry.Party.All(member => member.Current()))) continue;
                    _publishedParties.Remove(item.Key);
                    if (!_humans.Snapshot().Any(ticket => ticket.Value.PartyId == entry.PartyId))
                        _parties.SetQueueState(entry.Party[0], entry.PartyId, entry.Party, 1, -1);
                }
            }
        }

        void PrunePartySessions()
        {
            if (_parties == null) return;
            var now = DateTime.UtcNow;
            RefreshPublishedParties(now);
            lock (_gate)
            {
                _parties.Prune(now);
                foreach (var ticket in _humans.Snapshot())
                    if (ticket.Value.Party != null && (!ticket.Value.Alive || !PartyTicketCurrent(ticket)))
                        RemoveTicket(ticket);
                foreach (var entry in _partySessions.Where(entry => !entry.Value.Current()).ToArray())
                {
                    _partySessions.Remove(entry.Key);
                    _partyOwners.Remove(entry.Value);
                }
            }
        }
    }
}
