using System;
using System.Collections.Generic;
using System.Linq;
using DotNetty.Common.Internal.Logging;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Private retail party membership. Access is serialized by the lobby gate.</summary>
    public sealed class MagPartyDirectory
    {
        /// <summary>One authored squad: a party always fits a single squad, so it
        /// can enter a running match together (late entry).</summary>
        public const int MaximumMembers = 8;
        static readonly IInternalLogger Logger = InternalLoggerFactory.GetInstance<MagPartyDirectory>();
        public sealed record Member(string Account, uint AccountId, byte Faction, Func<bool> Current,
            Action<MagRetailPartyMessage> Send);

        sealed class Party
        {
            public uint Id;
            public readonly List<Member> Members = new();
            public readonly HashSet<uint> InvitedAccounts = new();
            public readonly Dictionary<uint, (Member Target, DateTime Expires)> Pending = new();
            public sbyte Status = 1;
            public sbyte GameType = -1;
        }

        readonly Dictionary<uint, Member> _online = new();
        readonly Dictionary<uint, Party> _membership = new();
        readonly TimeSpan _lifetime;
        uint _nextId;
        const int MaximumOnline = 4096;
        const int MaximumInvitations = 64;

        public MagPartyDirectory(TimeSpan lifetime)
        {
            if (lifetime < TimeSpan.FromSeconds(1) || lifetime > TimeSpan.FromMinutes(5))
                throw new ArgumentOutOfRangeException(nameof(lifetime));
            _lifetime = lifetime;
        }

        bool Owns(Member member) => member != null && _online.TryGetValue(member.AccountId, out var current) &&
            ReferenceEquals(current, member) && member.Current();

        public void Prune(DateTime now)
        {
            foreach (var member in _online.Values.Where(member => !member.Current()).ToArray())
                Remove(member);
            foreach (var party in _membership.Values.Distinct())
                foreach (var target in party.Pending.Where(x => x.Value.Expires <= now || !Owns(x.Value.Target)).ToArray())
                    party.Pending.Remove(target.Key);
        }

        public bool Register(Member member)
        {
            if (member == null || member.AccountId == 0 || member.Faction > 2 ||
                string.IsNullOrWhiteSpace(member.Account) || !member.Current()) return false;
            if (_online.TryGetValue(member.AccountId, out var current))
            {
                if (ReferenceEquals(current, member)) return true;
                if (current.Account != member.Account || current.Current()) return false;
                Remove(current);
            }
            if (_online.Count >= MaximumOnline || _online.Values.Any(other => other.Account == member.Account)) return false;
            _online.Add(member.AccountId, member);
            return true;
        }

        public void Remove(Member member)
        {
            if (member == null || !_online.TryGetValue(member.AccountId, out var current) ||
                !ReferenceEquals(current, member)) return;
            _online.Remove(member.AccountId);
            Leave(member, _membership.GetValueOrDefault(member.AccountId));
        }

        Party LeaderParty(Member member, uint partyId) =>
            Owns(member) && _membership.TryGetValue(member.AccountId, out var party) &&
            party.Id == partyId && ReferenceEquals(party.Members[0], member) ? party : null;

        public bool IsCurrentLeader(Member member, uint partyId) => LeaderParty(member, partyId) != null;

        public IReadOnlyList<Member> GetLeaderMembers(Member member, uint partyId)
        {
            var party = LeaderParty(member, partyId);
            return party != null && party.Members.All(Owns) ? party.Members.ToArray() : null;
        }

        public bool SetQueueState(Member member, uint partyId, IReadOnlyList<Member> expected, sbyte status,
            sbyte gameType)
        {
            var current = GetLeaderMembers(member, partyId);
            if (current == null || current.Count != expected.Count ||
                current.Where((value, i) => !ReferenceEquals(value, expected[i])).Any()) return false;
            var party = _membership[member.AccountId];
            party.Status = status;
            party.GameType = gameType;
            SendRoster(party);
            return true;
        }

        public bool Apply(Member member, MagRetailPartyMessage message, DateTime now)
        {
            Prune(now);
            if (!Owns(member)) return false;
            var current = _membership.GetValueOrDefault(member.AccountId);
            switch (message)
            {
                case MagRetailPartyInvite invite:
                    return Invite(member, current, invite, now);
                case MagRetailPartyInviteResponse answer:
                    return Answer(member, current, answer);
                case MagRetailPartyLeave leave when current != null && current.Id == leave.PartyId:
                    Leave(member, current);
                    return true;
                default:
                    return false;
            }
        }

        bool Invite(Member owner, Party party, MagRetailPartyInvite invite, DateTime now)
        {
            if (invite.Inviter != owner.AccountId || invite.PartyId != (party?.Id ?? 0) ||
                invite.Invitee == owner.AccountId || !_online.TryGetValue(invite.Invitee, out var target) ||
                !Owns(target) || target.Faction != owner.Faction || _membership.ContainsKey(invite.Invitee))
                return false;
            if (party == null)
            {
                if (_nextId == 1u << 24) return false;
                party = new Party { Id = ++_nextId };
                party.Members.Add(owner);
                _membership.Add(owner.AccountId, party);
                owner.Send(new MagRetailPartyJoined { PartyId = party.Id });
                SendRoster(party);
            }
            if (!ReferenceEquals(party.Members[0], owner) || party.Status != 1 ||
                party.InvitedAccounts.Count >= MaximumInvitations || party.InvitedAccounts.Contains(invite.Invitee))
                return false;
            // Outstanding invitations hold seats, so an accepted invite never overfills.
            if (party.Members.Count + party.Pending.Count >= MaximumMembers)
            {
                Logger.Warn($"Retail party invite refused: reason=party_full party={party.Id} members={party.Members.Count} pending={party.Pending.Count} maximum={MaximumMembers}");
                return false;
            }
            party.InvitedAccounts.Add(invite.Invitee);
            // The response has no invite nonce. Never reuse an inviter/party/target tuple.
            party.Pending.Add(target.AccountId, (target, now + _lifetime));
            target.Send(new MagRetailPartyInvite { PartyId = party.Id, Inviter = owner.AccountId, Invitee = target.AccountId });
            return true;
        }

        bool Answer(Member target, Party existing, MagRetailPartyInviteResponse answer)
        {
            if (existing != null || answer.Responder != target.AccountId || answer.Response > 4 ||
                !_membership.TryGetValue(answer.Inviter, out var party) || party.Id != answer.PartyId ||
                party.Status != 1 || party.Members[0].AccountId != answer.Inviter ||
                !party.Pending.TryGetValue(target.AccountId, out var pending) || !ReferenceEquals(pending.Target, target))
                return false;
            party.Pending.Remove(target.AccountId);
            if (answer.Response != 0) return true;
            if (party.Members.Count >= MaximumMembers)
            {
                Logger.Warn($"Retail party join refused: reason=party_full party={party.Id} maximum={MaximumMembers}");
                return false;
            }
            party.Members.Add(target);
            _membership.Add(target.AccountId, party);
            target.Send(new MagRetailPartyJoined { PartyId = party.Id });
            SendRoster(party);
            return true;
        }

        void Leave(Member member, Party party)
        {
            if (party == null) return;
            if (ReferenceEquals(party.Members[0], member))
            {
                foreach (var recipient in party.Members)
                {
                    _membership.Remove(recipient.AccountId);
                    if (Owns(recipient)) recipient.Send(new MagRetailPartyLeft { PartyId = party.Id });
                }
                return;
            }
            _membership.Remove(member.AccountId);
            party.Members.Remove(member);
            party.Status = 1;
            party.GameType = -1;
            if (Owns(member)) member.Send(new MagRetailPartyLeft { PartyId = party.Id });
            SendRoster(party);
        }

        static void SendRoster(Party party)
        {
            foreach (var recipient in party.Members.Where(member => member.Current()))
                recipient.Send(new MagRetailPartyRoster { PartyId = party.Id,
                    Members = party.Members.Select(member => member.AccountId).ToArray(),
                    Status = party.Status, GameType = party.GameType });
        }
    }
}
