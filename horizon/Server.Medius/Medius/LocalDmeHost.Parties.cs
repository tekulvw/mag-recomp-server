using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using RT.Common;
using RT.Models;
using RT.Models.MAG;
using DmeClient = Server.Dme.Models.ClientObject;

namespace Server.Medius
{
    internal sealed record MagPartyAdmissionMember(MagMlsSession Session, Action<MagCharacterRecord> Notify);

    public sealed partial class LocalDmeHost
    {
        bool _supportsPartyReservations;
        readonly HashSet<string> _provisionalPartyAccounts = new(StringComparer.Ordinal);
        ulong? _formationAdmissionEpoch;

        internal bool QueueRegistryParty(MagJoinGameRequest request, MagPartyAdmissionMember[] members,
            byte faction, Func<bool> current, Func<Func<bool>, bool> withOwnership, Action<IReadOnlyList<MagJoinGameResponse>> complete)
        {
            lock (_gate)
            {
                if (!InRegistry || !_supportsPartyReservations || !ReadyForAdmission || !current() ||
                    !_registryFormationStaged || _formedProfile?.SquadCapacity is not int cap || members.Length < 2 ||
                    members.Length > cap || request.Value != _formedProfile.GameType ||
                    (faction != _formedProfile.AttackingFaction && faction != _formedProfile.DefendingFaction))
                    return false;
                var leader = members[0].Session;
                _humanQueue.Put(leader.AuthenticatedAccount, (int)request.Value, faction,
                    new QueuedJoin { Session = leader, Request = request, Account = leader.AuthenticatedAccount,
                        Character = leader.CharacterId, Connected = current, Party = members, WithPartyOwnership = withOwnership, CompleteParty = complete }, members.Length);
                return true;
            }
        }

        internal bool CancelPublishedParty(MagMlsSession leader, MagJoinGameRequest request)
        {
            if (request is not MagRetailJoinGameRequest { PartyId: > 0 }) return false;
            Reservation[] pending;
            lock (_gate)
            {
                pending = _reservations.Values.Where(owner => IsLiveCohort(owner) && owner.Accepted &&
                    !owner.Claimed && !owner.Invalidated && OwnsQueueRequest(owner, leader, request)).ToArray();
                foreach (var owner in pending)
                {
                    // Share the claim lock so a transport cannot take an owner
                    // after cancellation selected it for ordinary release.
                    owner.Invalidated = true;
                    owner.ReleaseReason = "queued_party_cancelled";
                }
            }
            if (pending.Length != 0) _ = ReleaseCancelledPartyAsync(pending);
            return pending.Length != 0;
        }

        async Task ReleaseCancelledPartyAsync(Reservation[] owners)
        {
            try { await ReleaseReservationsAsync(owners); }
            catch (Exception error) { Logger.Warn($"Published party cancellation cleanup: error={error.GetType().Name}"); }
        }

        bool AdmittedPartyReply(GameplayBridgeResponse reply, string formation, string profile, string cohort,
            uint world, ulong epoch, IEnumerable<string> ids, string status, byte? team = null, bool emptyReleaseAllowed = false)
        {
            var record = reply?.PartyReservation;
            return reply?.Ok == true && record != null && record.Status == status && record.FormationId == formation &&
                record.ProfileId == profile && record.CohortId == cohort && record.WorldId == world && record.AdmissionEpoch == epoch &&
                (status == "released" ? (emptyReleaseAllowed && record.ReservationIds.Count == 0) || record.ReservationIds.SequenceEqual(ids) :
                    record.Team == team && record.ReservationIds.SequenceEqual(ids));
        }

        sealed class PreparedParty
        {
            public readonly MagPendingHumanQueue<QueuedJoin>.Entry Ticket;
            public readonly string Cohort = Id();
            public readonly List<Reservation> Owners = new();
            public readonly List<MagJoinGameResponse> Descriptors = new();
            public readonly List<object> Requests = new();
            public bool Attempted;
            public bool Acknowledged;

            public PreparedParty(MagPendingHumanQueue<QueuedJoin>.Entry ticket) => Ticket = ticket;
            public IEnumerable<string> ReservationIds => Owners.Select(owner => owner.Id);
        }

        async Task<bool> AdmitRegistryPartiesAsync(MagPendingHumanQueue<QueuedJoin>.Entry[] tickets,
            MagMatchProfile profile, string formation)
        {
            var parties = tickets.Select(ticket => new PreparedParty(ticket)).ToArray();
            var stagedClients = new List<DmeClient>();
            bool published = false;
            bool provisional = false;
            uint world = 0;
            ulong epoch = 0;
            Server.Dme.Models.World admissionWorld;
            bool Current() => tickets.All(ticket => _humanQueue.IsCurrent(ticket) && ticket.Value.Alive);
            await _admission.WaitAsync();
            try
            {
                lock (_gate)
                {
                    if (!ReadyForAdmission || !Current() || _formationAdmissionEpoch is not ulong selectedEpoch ||
                        _formationId != formation || _formationEverAdmitted || _admissionTerminal ||
                        _pendingReleases.Values.Any(pending => !IsResultsCohort(pending.Reservation)) ||
                        _reservations.Values.Any(IsLiveCohort) || _rewardOwners.Count + tickets.Sum(ticket => ticket.Human.Size) > 4096)
                        return false;
                    var accounts = tickets.SelectMany(ticket => ticket.Value.Accounts).ToArray();
                    if (accounts.Length != accounts.Distinct(StringComparer.Ordinal).Count() ||
                        _provisionalPartyAccounts.Count != 0) return false;
                    _provisionalPartyAccounts.UnionWith(accounts);
                    provisional = true;
                    epoch = selectedEpoch;
                    admissionWorld = AdmissionWorld;
                    world = (uint)admissionWorld.WorldId;
                }
                await _rewardSettlement.WaitAsync();
                try
                {
                    foreach (var party in parties)
                    foreach (var member in party.Ticket.Value.Party)
                    {
                        var session = member.Session;
                        if (JoinSessionRejection(session) != null || !party.Ticket.Value.Alive) return false;
                        var character = _store.ReadForAccount(session.AuthenticatedAccount).SingleOrDefault(c => c.Id == session.CharacterId);
                        if (character == null || character.Faction != party.Ticket.Human.Faction) return false;
                        var record = character.ToRecord();
                        var loadouts = GameplayLoadoutAdmission.Project(character, record, _store.LoadoutCatalog, _supportsLoadoutLayoutChanges);
                        var allocation = await admissionWorld.OnJoinGameRequest(new MediusServerJoinGameRequest
                        {
                            ConnectInfo = new NetConnectionInfo { SessionKey = session.SessionKey }
                        });
                        if (allocation.Confirmation != MGCL_ERROR_CODE.MGCL_SUCCESS ||
                            !admissionWorld.Clients.TryGetValue(allocation.DmeClientIndex, out var client)) return false;
                        stagedClients.Add(client);
                        if (admissionWorld.Manager.GetClientByAccessToken(client.Token) != client ||
                            admissionWorld.Manager.GetClientBySessionKey(client.SessionKey) != client) return false;
                        client.ApplicationId = _applicationId;
                        var prepared = PrepareReservation(session, party.Ticket.Value.Request, member.Notify, character, record, loadouts, client);
                        // The native queue request belongs to the leader. Every
                        // member still has its own authenticated DME reservation.
                        prepared.Owner.QueueSession = party.Ticket.Value.Session;
                        party.Owners.Add(prepared.Owner);
                        var descriptor = PrepareJoinDescriptor(party.Ticket.Value.Request.TransactionId, client);
                        MagMessageCodec.SerializeForApplication(descriptor, _applicationId);
                        party.Descriptors.Add(descriptor);
                        party.Requests.Add(new { reservation_id = prepared.Owner.Id, player_id = (ushort)client.DmeId,
                            account_id = character.Id, session_key = client.SessionKey, access_key = client.Token,
                            character = prepared.Character });
                    }
                }
                finally { _rewardSettlement.Release(); }

                var deadline = DateTime.UtcNow.AddSeconds(30);
                var squads = new HashSet<ushort>();
                foreach (var party in parties)
                {
                    while (DateTime.UtcNow < deadline && !_matchmakingCancellation.IsCancellationRequested)
                    {
                        lock (_gate) if (!ReadyForAdmission || !Current()) return false;
                        party.Attempted = true;
                        var reply = await _bridge.RequestControlAsync(new { v = 2, request_id = Id(), op = "reserve_party",
                            formation_id = formation, profile_id = profile.ProfileId, cohort_id = party.Cohort, world_id = world,
                            admission_epoch = epoch, members = party.Requests });
                        byte team = party.Ticket.Human.Faction == profile.AttackingFaction ? (byte)0 : (byte)1;
                        if (AdmittedPartyReply(reply, formation, profile.ProfileId, party.Cohort, world, epoch,
                            party.ReservationIds, "reserved", team))
                        {
                            party.Acknowledged = true;
                            if (!squads.Add(reply.PartyReservation.Squad.Value)) return false;
                            break;
                        }
                        if (reply?.Ok == false && reply.Failure == GameplayBridgeFailure.None) break;
                        await Task.Delay(250);
                    }
                    if (!party.Acknowledged) return false;
                }

                var owners = parties.SelectMany(party => party.Owners).ToArray();
                bool CommitAll()
                {
                    lock (_gate)
                    {
                        if (!ReadyForAdmission || !Current() || _formationId != formation || _formationAdmissionEpoch != epoch ||
                            owners.Any(owner => owner.Client.IsDestroyed || owner.Client.Disconnected ||
                                admissionWorld.Manager.GetClientByAccessToken(owner.Client.Token) != owner.Client ||
                                admissionWorld.Manager.GetClientBySessionKey(owner.Client.SessionKey) != owner.Client))
                            return false;
                        if (owners.Select(owner => owner.Id).Distinct(StringComparer.Ordinal).Count() != owners.Length ||
                            owners.Any(owner => _reservations.ContainsKey(owner.Id) || _rewardOwners.ContainsKey(owner.Id))) return false;
                        foreach (var owner in owners)
                        {
                            owner.Accepted = true;
                            owner.Deadline = DateTime.UtcNow.AddSeconds(30);
                            _reservations.Add(owner.Id, owner);
                            _rewardOwners.Add(owner.Id, owner);
                        }
                        _formationEverAdmitted = true;
                        _registryFormationStaged = false;
                        published = true;
                    }
                    foreach (var party in parties) party.Ticket.Value.CompleteParty(party.Descriptors);
                    return true;
                }
                // Nested guards refer to the same registry gate. It owns the
                // membership/current tickets through installation and publication.
                bool WithAllOwners(int index) => index == parties.Length ? CommitAll() :
                    parties[index].Ticket.Value.WithPartyOwnership(() => WithAllOwners(index + 1));
                if (!WithAllOwners(0)) return false;
                foreach (var owner in owners) ArmoryLoadoutCommitted(owner.Account, owner.CharacterId);
                return true;
            }
            catch (Exception error) when (error is IOException || error is UnauthorizedAccessException || error is ArgumentException)
            {
                Logger.Warn($"Retail party admission refused: error={error.GetType().Name}");
                return published;
            }
            finally
            {
                try
                {
                    if (!published)
                    {
                        foreach (var party in parties.Reverse().Where(party => party.Attempted))
                            await ReconcilePartyReleaseAsync(party, formation, profile.ProfileId, world, epoch);
                        foreach (var client in stagedClients)
                        {
                            client.DmeWorld.Manager.RemoveClient(client);
                            client.DmeWorld.RemoveLocalClient(client);
                            await client.Stop();
                        }
                        foreach (var party in parties) party.Ticket.Value.CompleteParty(null);
                    }
                }
                finally
                {
                    if (provisional) lock (_gate) _provisionalPartyAccounts.Clear();
                    _admission.Release();
                }
            }
        }

        async Task ReconcilePartyReleaseAsync(PreparedParty party, string formation, string profile, uint world, ulong epoch)
        {
            // An uncertain rollback quarantines this worker formation and its
            // provisional DME allocations while the same key is reconciled.
            while (!_matchmakingCancellation.IsCancellationRequested && !_bridge.HasWorkerIdentityFault)
            {
                var reply = await _bridge.RequestControlAsync(new { v = 2, request_id = Id(), op = "release_party",
                    formation_id = formation, profile_id = profile, cohort_id = party.Cohort,
                    world_id = world, admission_epoch = epoch });
                if (AdmittedPartyReply(reply, formation, profile, party.Cohort, world, epoch, party.ReservationIds, "released", emptyReleaseAllowed: !party.Acknowledged)) return;
                await Task.Delay(1000);
            }
        }
    }
}
