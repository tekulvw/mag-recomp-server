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
        bool _supportsPartyLateEntry;
        readonly HashSet<string> _provisionalPartyAccounts = new(StringComparer.Ordinal);
        ulong? _formationAdmissionEpoch;
        // A refused or unconfirmed late entry closes this world to parties briefly,
        // so placement can try another world or a new formation instead.
        static readonly TimeSpan LatePartyBackoff = TimeSpan.FromSeconds(5);
        DateTime _nextLatePartyAttempt;

        // Called with _gate held. One party at a time joins this running match.
        bool LatePartyOpen() => InRegistry && _supportsPartyReservations && _supportsPartyLateEntry &&
            !_registryFormationStaged && _formationConfigured && _formationEverAdmitted &&
            _formedProfile?.SquadCapacity != null && _formationAdmissionEpoch != null && !_admissionTerminal &&
            DateTime.UtcNow >= _nextLatePartyAttempt && _provisionalPartyAccounts.Count == 0 &&
            _reservations.Values.Any(IsLiveCohort) && !_humanQueue.Snapshot().Any(ticket => ticket.Value.Party != null);

        // Called with _gate held: room for a whole late party on its side.
        bool LatePartyFits(int size, byte faction)
        {
            var live = _reservations.Values.Where(IsLiveCohort).ToArray();
            int pending = _humanQueue.Snapshot().Sum(ticket => ticket.Human.Size);
            int sidePending = _humanQueue.Snapshot().Where(ticket => ticket.Human.Faction == faction).Sum(ticket => ticket.Human.Size);
            return live.Length + pending + size <= Math.Min(_maximumPlayers, _formedProfile.HumanCapacity) &&
                live.Count(r => r.Faction == faction) + sidePending + size <= _formedProfile.HumanTeamCapacity;
        }

        internal bool QueueRegistryParty(MagJoinGameRequest request, MagPartyAdmissionMember[] members,
            byte faction, Func<bool> current, Func<Func<bool>, bool> withOwnership, Action<IReadOnlyList<MagJoinGameResponse>> complete)
        {
            lock (_gate)
            {
                PurgeHumanQueue();
                bool late = !_registryFormationStaged && LatePartyOpen();
                if (!InRegistry || !_supportsPartyReservations || !ReadyForAdmission || !current() ||
                    !(_registryFormationStaged || late) || _formedProfile?.SquadCapacity is not int cap || members.Length < 2 ||
                    members.Length > cap || request.Value != _formedProfile.GameType ||
                    (faction != _formedProfile.AttackingFaction && faction != _formedProfile.DefendingFaction) ||
                    (late && !LatePartyFits(members.Length, faction)))
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
            public string Cohort = Id();
            public readonly List<Reservation> Owners = new();
            public readonly List<MagJoinGameResponse> Descriptors = new();
            public readonly List<Dictionary<string, object>> Requests = new();
            public bool Attempted;
            public bool Acknowledged;

            public PreparedParty(MagPendingHumanQueue<QueuedJoin>.Entry ticket) => Ticket = ticket;
            public IEnumerable<string> ReservationIds => Owners.Select(owner => owner.Id);

            /// <summary>A released, never-published cohort is retried under a new cohort and new
            /// reservation IDs; the worker fences the old ones. DME clients and descriptors stay.</summary>
            public void Renew()
            {
                Cohort = Id();
                for (int i = 0; i < Owners.Count; i++)
                {
                    Owners[i].Id = Id();
                    Requests[i]["reservation_id"] = Owners[i].Id;
                }
                Attempted = false;
            }
        }

        async Task<bool> AdmitRegistryPartiesAsync(MagPendingHumanQueue<QueuedJoin>.Entry[] tickets,
            MagMatchProfile profile, string formation, bool late = false)
        {
            var parties = tickets.Select(ticket => new PreparedParty(ticket)).ToArray();
            var stagedClients = new List<DmeClient>();
            bool published = false;
            bool provisional = false;
            MagDmePortBudget.Lease[] leases = null;
            uint world = 0;
            ulong epoch = 0;
            Server.Dme.Models.World admissionWorld;
            bool Current() => tickets.All(ticket => _humanQueue.IsCurrent(ticket) && ticket.Value.Alive);
            await _admission.WaitAsync();
            try
            {
                lock (_gate)
                {
                    // A forming match admits its parties before anyone; a late
                    // party joins the running match beside its live owners.
                    if (!ReadyForAdmission || !Current() || _formationAdmissionEpoch is not ulong selectedEpoch ||
                        _formationId != formation || _formationEverAdmitted != late || _admissionTerminal ||
                        (!late && (_pendingReleases.Values.Any(pending => !IsResultsCohort(pending.Reservation)) ||
                            _reservations.Values.Any(IsLiveCohort))) ||
                        _rewardOwners.Count + tickets.Sum(ticket => ticket.Human.Size) > 4096)
                        return false;
                    var accounts = tickets.SelectMany(ticket => ticket.Value.Accounts).ToArray();
                    if (accounts.Length != accounts.Distinct(StringComparer.Ordinal).Count() ||
                        _provisionalPartyAccounts.Count != 0) return false;
                    if (late && (tickets.Length != 1 || !_reservations.Values.Any(IsLiveCohort) ||
                        _reservations.Values.Any(owner => accounts.Contains(owner.Account, StringComparer.Ordinal)) ||
                        _pendingReleases.Values.Any(pending => accounts.Contains(pending.Reservation.Account, StringComparer.Ordinal)) ||
                        _reservations.Values.Count(IsLiveCohort) + tickets[0].Human.Size > Math.Min(_maximumPlayers, profile.HumanCapacity) ||
                        _reservations.Values.Count(r => IsLiveCohort(r) && r.Faction == tickets[0].Human.Faction) + tickets[0].Human.Size > profile.HumanTeamCapacity))
                        return false;
                    // The whole group gets DME ports together or not at all.
                    leases = _portBudget.TryAcquire(accounts.Length);
                    if (leases == null)
                    {
                        LogPortHold($"Retail party admission held: reason=dme_port_capacity members={accounts.Length}");
                        return false;
                    }
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
                        prepared.Owner.PortLease = leases[stagedClients.Count - 1];
                        // The native queue request belongs to the leader. Every
                        // member still has its own authenticated DME reservation.
                        prepared.Owner.QueueSession = party.Ticket.Value.Session;
                        party.Owners.Add(prepared.Owner);
                        var descriptor = PrepareJoinDescriptor(party.Ticket.Value.Request.TransactionId, client);
                        MagMessageCodec.SerializeForApplication(descriptor, _applicationId);
                        party.Descriptors.Add(descriptor);
                        party.Requests.Add(new Dictionary<string, object> { ["reservation_id"] = prepared.Owner.Id,
                            ["player_id"] = (ushort)client.DmeId, ["account_id"] = character.Id, ["session_key"] = client.SessionKey,
                            ["access_key"] = client.Token, ["character"] = prepared.Character });
                    }
                }
                finally { _rewardSettlement.Release(); }

                var squads = new HashSet<ushort>();
                foreach (var party in parties)
                {
                    // The same attempt limit and per-exchange deadline as a solo reserve.
                    var attempts = new AdmissionAttempts(AdmissionPolicy.Attempts);
                    byte team = party.Ticket.Human.Faction == profile.AttackingFaction ? (byte)0 : (byte)1;
                    while (!_matchmakingCancellation.IsCancellationRequested)
                    {
                        lock (_gate) if (!ReadyForAdmission || !Current()) return false;
                        party.Attempted = true;
                        var reply = await _bridge.RequestControlAsync(new { v = 2, request_id = Id(), op = "reserve_party",
                            formation_id = formation, profile_id = profile.ProfileId, cohort_id = party.Cohort, world_id = world,
                            admission_epoch = epoch, members = party.Requests }, AdmissionPolicy.Deadline);
                        if (AdmittedPartyReply(reply, formation, profile.ProfileId, party.Cohort, world, epoch,
                            party.ReservationIds, "reserved", team))
                        {
                            party.Acknowledged = true;
                            if (!squads.Add(reply.PartyReservation.Squad.Value)) return false;
                            break;
                        }
                        // A definite refusal is final.
                        if (reply?.Ok == false && reply.Failure == GameplayBridgeFailure.None) break;
                        if (!attempts.TryTake())
                        {
                            Logger.Warn($"Retail party admission unanswered after {attempts.Used} attempt(s): members={party.Owners.Count} failure={reply?.Failure.ToString() ?? "Unavailable"}; the party returns to the queue");
                            break;
                        }
                        // Never written: the worker cannot hold it, so send it again as it is.
                        if (reply?.IsTransientFailure == true && reply.RequestNotSent)
                        {
                            Logger.Warn($"Retail party admission not delivered: members={party.Owners.Count} failure={reply.Failure}; sending it again over a new connection (attempt {attempts.Used} of {attempts.Total})");
                            continue;
                        }
                        // Sent but not answered (or answered unexpectedly): release this exact
                        // cohort, which also fences one the worker never saw, then reserve again
                        // under a new cohort and new reservation IDs.
                        if (!await ReleaseUnansweredPartyAsync(party, formation, profile.ProfileId, world, epoch))
                        {
                            Logger.Warn($"Retail party admission retry abandoned: reason=release_unacknowledged members={party.Owners.Count}; the party returns to the queue");
                            break;
                        }
                        party.Renew();
                        Logger.Warn($"Retail party admission retrying: members={party.Owners.Count} failure={reply?.Failure.ToString() ?? "Unavailable"} attempt={attempts.Used}/{attempts.Total} after release of the unanswered cohort");
                    }
                    if (!party.Acknowledged) return false;
                }

                var owners = parties.SelectMany(party => party.Owners).ToArray();
                bool CommitAll()
                {
                    lock (_gate)
                    {
                        if (!ReadyForAdmission || !Current() || _formationId != formation || _formationAdmissionEpoch != epoch ||
                            _admissionTerminal || owners.Any(owner => owner.Client.IsDestroyed || owner.Client.Disconnected ||
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
                    Logger.Info($"Retail party admitted: members={owners.Length} late={late} dme_ports={_portBudget.InUse}/{_portBudget.Capacity?.ToString() ?? "unbounded"}");
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
                    // Unpublished members' ports return after their clients stopped above.
                    if (!published && leases != null) foreach (var lease in leases) lease.Dispose();
                    if (provisional) lock (_gate) _provisionalPartyAccounts.Clear();
                    _admission.Release();
                }
            }
        }

        /// <summary>Release an unacknowledged cohort within the admission deadline. True once the
        /// worker acknowledges it (an empty release fences a cohort it never recorded).</summary>
        async Task<bool> ReleaseUnansweredPartyAsync(PreparedParty party, string formation, string profile, uint world, ulong epoch)
        {
            var until = DateTime.UtcNow + AdmissionPolicy.Deadline + AdmissionPolicy.Deadline + TimeSpan.FromSeconds(1);
            while (!_matchmakingCancellation.IsCancellationRequested && !_bridge.HasWorkerIdentityFault)
            {
                var reply = await _bridge.RequestControlAsync(new { v = 2, request_id = Id(), op = "release_party",
                    formation_id = formation, profile_id = profile, cohort_id = party.Cohort,
                    world_id = world, admission_epoch = epoch }, AdmissionPolicy.Deadline);
                if (AdmittedPartyReply(reply, formation, profile, party.Cohort, world, epoch, party.ReservationIds, "released", emptyReleaseAllowed: true))
                    return true;
                if (DateTime.UtcNow >= until) return false;
                await Task.Delay(250);
            }
            return false;
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
