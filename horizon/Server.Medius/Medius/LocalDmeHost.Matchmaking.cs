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
    public sealed partial class LocalDmeHost
    {
        sealed class QueuedJoin
        {
            public MagMlsSession Session;
            public MagJoinGameRequest Request;
            public string Account;
            public uint Character;
            public Func<bool> Connected;
            public Action<MagPluginClientMessage> Send;
            public Action<MagCharacterRecord> Notify;
            public MagPartyAdmissionMember[] Party;
            public Func<Func<bool>, bool> WithPartyOwnership;
            public Action<IReadOnlyList<MagJoinGameResponse>> CompleteParty;
            public IEnumerable<string> Accounts => Party == null ? new[] { Account } : Party.Select(member => member.Session.AuthenticatedAccount);
            public bool Alive => Connected() && Session.State==MagMlsSession.SessionState.Ready &&
                !Session.IsExpired(DateTime.UtcNow) && Session.AuthenticatedAccount==Account && Session.CharacterId==Character;
        }
        IReadOnlyList<MagMatchProfile> _matchProfiles=Array.Empty<MagMatchProfile>();
        readonly MagPendingHumanQueue<QueuedJoin> _humanQueue=new();
        readonly CancellationTokenSource _matchmakingCancellation=new();
        Task _matchmakingWorker=Task.CompletedTask;
        MagMatchStartupPolicy _formationStartup;
        int[] _formationTeamMinima;
        // The operator's minima without the two-human floor. Only profiles the
        // worker fills with bots (bot_fill_v1) form under these.
        MagMatchStartupPolicy _botFillStartup;
        int[] _botFillTeamMinima;
        bool _anyBotFill;
        MagMatchProfile _formedProfile;
        string _formationId;
        bool _formationConfigured;
        bool _formationEverAdmitted;
        DateTime _nextQueueNotice;
        DateTime _nextFormationAttempt;
        public bool UsesFactionMatchmaking => _matchProfiles.Count!=0;

        void InitializeFactionMatchmaking(GameplayBridgeResponse hello)
        {
            bool enabled=hello.Capabilities.Contains("match_profiles_v1",StringComparer.Ordinal);
            if(!enabled && hello.MatchProfiles.Count==0)return;
            if(!enabled || _applicationId!=20231 || hello.MatchProfiles.Count==0)
                throw new IOException("Incomplete retail match-profile capability.");
            foreach(var mode in hello.MatchProfiles.Select(p=>p.GameType).Distinct())
                for(byte a=0;a<3;a++)for(byte d=0;d<3;d++)if(a!=d &&
                    !hello.MatchProfiles.Any(p=>p.GameType==mode&&p.AttackingFaction==a&&p.DefendingFaction==d))
                    throw new IOException("Faction matchmaking requires every ordered faction pair for each advertised mode.");
            if(hello.MatchProfiles.Any(p=>p.BotFill)&&!hello.SupportsBotFill)
                throw new IOException("Bot-filled match profiles require the bot_fill_v1 capability.");
            var configured=_options.MinimumPlayersPerTeam?.ToArray()??new[]{1,1};
            if(configured.Length!=2||configured.Any(n=>n<0))throw new IOException("Matchmaking requires two nonnegative human team minima.");
            _formationTeamMinima=MagFactionMatchmaking.FormationMinima(null,configured);
            _formationStartup=new MagMatchStartupPolicy(Math.Max(2,_minimumPlayers),_maximumPlayers,_formationTeamMinima);
            _botFillTeamMinima=configured.ToArray();
            _botFillStartup=new MagMatchStartupPolicy(Math.Max(1,_minimumPlayers),_maximumPlayers,_botFillTeamMinima);
            var profiles = hello.MatchProfiles;
            if (InRegistry)
            {
                profiles = Array.AsReadOnly(profiles.Where(profile =>
                    _allowedFactions.Contains(profile.AttackingFaction) && _allowedFactions.Contains(profile.DefendingFaction) &&
                    profile.MaximumPlayers >= StartupFor(profile).MinimumPlayers &&
                    MinimaFor(profile).All(minimum => minimum <= profile.TeamCapacity) &&
                    (long)profile.TeamCapacity * 2 >= StartupFor(profile).MinimumPlayers).ToArray());
            }
            _anyBotFill = profiles.Any(profile => profile.BotFill);
            _matchProfiles = _supportsPartyReservations ? profiles : Array.AsReadOnly(profiles.Select(profile => profile with { SquadCapacity = null, SquadCounts = null }).ToArray());
        }
        MagMatchStartupPolicy StartupFor(MagMatchProfile profile) => profile?.BotFill == true ? _botFillStartup : _formationStartup;
        int[] MinimaFor(MagMatchProfile profile) => profile?.BotFill == true ? _botFillTeamMinima : _formationTeamMinima;
        // Placement planning applies the per-profile floor itself, so a worker
        // with bot-filled profiles reports the operator's unfloored minima.
        MagMatchStartupPolicy PlacementStartup => _anyBotFill ? _botFillStartup : _formationStartup;
        int[] PlacementTeamMinima => _anyBotFill ? _botFillTeamMinima : _formationTeamMinima;
        bool RequestedModeIsCurrent(uint mode)
        {
            lock (_gate) return UsesFactionMatchmaking
                ? _formedProfile != null && _formationConfigured && mode == (uint)_formedProfile.GameType
                : mode == (uint)_gameType;
        }

        public static bool MatchesFormationAcknowledgement(GameplayBridgeResponse reply,string formation,MagMatchProfile profile)
        {
            var selected=reply?.SelectedMatch;
            return reply?.Ok==true&&selected!=null&&selected.FormationId==formation&&selected.ProfileId==profile.ProfileId&&
                selected.GameType==profile.GameType&&selected.AttackingFaction==profile.AttackingFaction&&selected.DefendingFaction==profile.DefendingFaction;
        }

        /// <summary>Records intent and returns immediately; MLS keeps processing keepalives/cancel.</summary>
        public void QueueJoin(MagMlsSession session,MagJoinGameRequest request,
            Action<MagPluginClientMessage> send,Func<bool> connected,Action<MagCharacterRecord> notify=null)
        {
            var rejection=!UsesFactionMatchmaking?"faction_matchmaking_disabled":
                JoinRequestRejection(request,true,mode=>_matchProfiles.Any(p=>(uint)p.GameType==mode))??HostJoinSessionRejection(session)??
                (!connected()?"lobby_disconnected":null);
            if(rejection!=null)
            {
                LogJoinRejection(rejection,request);
                send(JoinResponse(request.TransactionId, InRegistry && rejection == "host_not_ready" ? (byte)6 : (byte)2));
                return;
            }
            try
            {
                var character=_store.ReadForAccount(session.AuthenticatedAccount).SingleOrDefault(c=>c.Id==session.CharacterId);
                if(character==null||character.Faction>2){send(JoinResponse(request.TransactionId,2));return;}
                var queued=new QueuedJoin {Session=session,Request=request,Account=session.AuthenticatedAccount,
                    Character=session.CharacterId,Connected=connected,Send=send,Notify=notify};
                byte result = 0;
                lock(_gate)
                {
                    if(!ReadyForAdmission || _reservations.Values.Any(r=>IsLiveCohort(r)&&r.Account==queued.Account&&r.TerminalMatchId==null))
                        result = 6;
                    else if (InRegistry && (_formedProfile == null ||
                        request.Value != (uint)_formedProfile.GameType ||
                        (character.Faction != _formedProfile.AttackingFaction && character.Faction != _formedProfile.DefendingFaction)))
                        result = 6;
                    else _humanQueue.Put(queued.Account,(int)request.Value,character.Faction,queued);
                }
                if (result != 0) { send(JoinResponse(request.TransactionId, result)); return; }
                // The registry stages and forwards a batch synchronously, then
                // starts the worker on its next world tick.
                if (InRegistry) return;
                SendQueueNotices(true);
                StartMatchmakingPump();
            }
            catch(Exception e) when(e is IOException||e is InvalidDataException||e is UnauthorizedAccessException||e is ArgumentException||e is InvalidOperationException)
            {send(JoinResponse(request.TransactionId,2));}
        }
        public void CancelQueuedJoin(MagMlsSession session) => CancelQueuedJoinCore(session, null);

        public void CancelQueuedJoin(MagMlsSession session, MagJoinGameRequest expectedRequest)
        {
            ArgumentNullException.ThrowIfNull(expectedRequest);
            CancelQueuedJoinCore(session, expectedRequest);
        }

        void CancelQueuedJoinCore(MagMlsSession session, MagJoinGameRequest expectedRequest)
        {
            if(session==null)return;
            lock(_gate)
            {
                var ticket = _humanQueue.Snapshot().FirstOrDefault(e => ReferenceEquals(e.Value.Session, session) &&
                    (expectedRequest == null || ReferenceEquals(e.Value.Request, expectedRequest)));
                if(ticket!=null)_humanQueue.Remove(ticket);
            }
            // A cancel can race the successful join reply. Release only this
            // session's still-unclaimed reservation, never an active human.
            _ = ReleaseCancelledQueueAdmissionAsync(session, expectedRequest);
        }
        async Task ReleaseCancelledQueueAdmissionAsync(MagMlsSession session, MagJoinGameRequest expectedRequest)
        {
            try
            {
                Task release=null;
                lock(_gate)
                {
                    var pending=_reservations.Values.FirstOrDefault(r=>IsLiveCohort(r)&&r.Account==session.AuthenticatedAccount&&
                        r.CharacterId == session.CharacterId && r.Client.SessionKey == session.SessionKey && !r.Claimed &&
                        (expectedRequest == null || OwnsQueueRequest(r, session, expectedRequest)));
                    if(pending!=null){pending.ReleaseReason="queued_join_cancelled";release=ReleaseAsync(pending);}
                }
                if(release!=null)await release;
            }
            catch(Exception e){Logger.Error($"Queued cancellation cleanup: {e.GetType().Name}");}
        }
        static bool OwnsQueueRequest(Reservation reservation, MagMlsSession session, MagJoinGameRequest request) =>
            ReferenceEquals(reservation.QueueSession, session) && ReferenceEquals(reservation.QueueRequest, request);
        void PurgeHumanQueue()
        {
            foreach(var ticket in _humanQueue.Snapshot())if(!ticket.Value.Alive)_humanQueue.Remove(ticket);
        }
        void AddQueuedStatus(MagServerStatusResponse status)
        {
            if(!UsesFactionMatchmaking)return;
            PurgeHumanQueue();
            status.MinimumPlayers=Enumerable.Repeat((uint)PlacementStartup.MinimumPlayers,4).ToArray();
            foreach(var ticket in _humanQueue.Snapshot())if(!_reservations.Values.Any(r=>IsLiveCohort(r)&&r.Accepted&&
                r.Account==ticket.Value.Account&&r.CharacterId==ticket.Value.Character))
                status.QueueSizes[ticket.Human.GameType*3+ticket.Human.Faction] += (uint)ticket.Human.Size;
            if(ReadyForAdmission)
                foreach(var profile in _matchProfiles)
                {status.QueueEnabled[profile.GameType*3+profile.AttackingFaction]=true;status.QueueEnabled[profile.GameType*3+profile.DefendingFaction]=true;}
        }
        void SendQueueNotices(bool force=false)
        {
            MagPendingHumanQueue<QueuedJoin>.Entry[] tickets;var counts=new uint[12];
            lock(_gate)
            {
                if(!UsesFactionMatchmaking||_disposed||(!force&&DateTime.UtcNow<_nextQueueNotice))return;
                _nextQueueNotice=DateTime.UtcNow.AddSeconds(5);PurgeHumanQueue();tickets=_humanQueue.Snapshot().ToArray();
                foreach(var ticket in tickets)counts[ticket.Human.GameType*3+ticket.Human.Faction]++;
            }
            foreach(var ticket in tickets)lock(_gate)if(_humanQueue.IsCurrent(ticket)&&ticket.Value.Alive)
                ticket.Value.Send(new MagRetailJoinGameUpdate {
                    TransactionId=ticket.Value.Request.TransactionId,Success=true,
                    // Local compatibility policy: installed MAG callback ignores this
                    // raw enum; the original consumer still refreshes its30s timeout.
                    QueuedType=0,QueueSizes=(uint[])counts.Clone()});
        }
        void StartMatchmakingPump()
        {
            lock(_gate)
            {
                if (!UsesFactionMatchmaking || !_matchmakingWorker.IsCompleted) return;
                if (InRegistry)
                {
                    if (_registryBatchDepth > 0) return;
                    PurgeHumanQueue();
                    if (_registryFormationStaged && _humanQueue.Count == 0) ResetFormation();
                }
                if (!ReadyForAdmission || DateTime.UtcNow < _nextFormationAttempt || _humanQueue.Count == 0) return;
                _matchmakingWorker=Task.Run(PumpFactionMatchmakingAsync);
            }
        }
        void ResetFormation()
        {
            _formedProfile = null;
            _formationId = null;
            _formationConfigured = false;
            _formationAdmissionEpoch = null;
            _formationEverAdmitted = false;
            _registryFormationStaged = false;
            _registryExpectedParties = 0;
        }
        void AbandonRegistryFormation()
        {
            MagPendingHumanQueue<QueuedJoin>.Entry[] abandoned;
            lock (_gate)
            {
                abandoned = _humanQueue.Snapshot().ToArray();
                foreach (var ticket in abandoned) _humanQueue.Remove(ticket);
                ResetFormation();
            }
            foreach (var ticket in abandoned.Where(ticket => ticket.Value.Alive))
                if (ticket.Value.Party != null) ticket.Value.CompleteParty(null);
                else ticket.Value.Send(JoinResponse(ticket.Value.Request.TransactionId, 6));
        }
        async Task PumpFactionMatchmakingAsync()
        {
            try
            {
                MagPendingHumanQueue<QueuedJoin>.Entry[] waiting;
                lock(_gate){PurgeHumanQueue();waiting=_humanQueue.Snapshot().ToArray();}
                foreach(var ticket in waiting)
                {
                    Task release;
                    lock(_gate)
                    {
                        if(!ReadyForAdmission||!_humanQueue.IsCurrent(ticket)||!ticket.Value.Alive)continue;
                        release = ticket.Value.Party == null ? ReleasePriorResultsAsync(ticket.Value.Account,ticket.Value.Character) : Task.CompletedTask;
                    }
                    await release;
                }
                MagMatchProfile profile;string formation;
                lock(_gate)
                {
                    PurgeHumanQueue();
                    if(!ReadyForAdmission||_pendingReleases.Values.Any(p=>!IsResultsCohort(p.Reservation))||
                        (!_independentMatchCohorts&&_reservations.Values.Any(r=>r.TerminalMatchId!=null)))return;
                    if(!InRegistry && (_admissionTerminal||(_formationEverAdmitted&&!_reservations.Values.Any(IsLiveCohort))))ResetFormation();
                    if(_formedProfile==null)
                    {
                        if (InRegistry) return;
                        var selection=MagFactionMatchmaking.Select(_humanQueue.Snapshot().Select(e=>e.Human).ToArray(),
                            _matchProfiles,PlacementStartup.MinimumPlayers,_maximumPlayers,PlacementTeamMinima,RandomNumberGenerator.GetInt32);
                        if(selection==null)return;
                        if(!AdvanceTerminalCohort())return;
                        _formedProfile=selection.Profile;_formationId=Id();
                    }
                    profile=_formedProfile;formation=_formationId;
                }
                if(!_formationConfigured)
                {
                    lock (_gate) if (!ReadyForAdmission || _matchmakingCancellation.IsCancellationRequested) return;
                    var reply=await _bridge.RequestControlAsync(new {v=2,request_id=Id(),op="select_match_profile",formation_id=formation,profile_id=profile.ProfileId});
                    if(!MatchesFormationAcknowledgement(reply,formation,profile))
                    {
                        bool refused = reply != null && !reply.Ok && reply.Failure == GameplayBridgeFailure.None;
                        lock (_gate)
                        {
                            _nextFormationAttempt=DateTime.UtcNow.AddSeconds(1);
                            if (refused && !InRegistry) ResetFormation();
                        }
                        if (refused && InRegistry) AbandonRegistryFormation();
                        return; // An uncertain selection is retried with the SAME formation identity.
                    }
                    lock(_gate)if(!ReadyForAdmission||_matchmakingCancellation.IsCancellationRequested)return;
                    var startup=StartupFor(profile).WithProfileMaximum(profile.MaximumPlayers);
                    if(!await _bridge.RequestAsync(startup.ConfigureRequest(Id(),_applicationId)))
                    {lock(_gate)_nextFormationAttempt=DateTime.UtcNow.AddSeconds(1);return;}
                    lock(_gate){if(!ReadyForAdmission||_matchmakingCancellation.IsCancellationRequested)return;_formationAdmissionEpoch = reply.SelectedMatch.AdmissionEpoch; _formationConfigured=true;}
                }
                MagPendingHumanQueue<QueuedJoin>.Entry[] cohort;
                bool lateParty = false;
                lock(_gate)
                {
                    PurgeHumanQueue();
                    if(!ReadyForAdmission||_matchmakingCancellation.IsCancellationRequested)return;
                    if (InRegistry && _admissionTerminal)
                        cohort = null;
                    else if(!_reservations.Values.Any(IsLiveCohort))
                    {
                        var selection=MagFactionMatchmaking.ForProfile(_humanQueue.Snapshot().Select(e=>e.Human).ToArray(),profile,
                            StartupFor(profile).MinimumPlayers,_maximumPlayers,MinimaFor(profile));
                        if (selection == null)
                        {
                            if (!InRegistry) { ResetFormation(); return; }
                            cohort = null;
                        }
                        else
                        {
                            var ids=selection.Players.Select(p=>p.TicketId).ToHashSet(StringComparer.Ordinal);
                            cohort=_humanQueue.Snapshot().Where(e=>ids.Contains(e.Human.TicketId)).OrderByDescending(e=>e.Human.Size).ToArray();
                            if (InRegistry && cohort.Count(ticket => ticket.Value.Party != null) != _registryExpectedParties)
                                cohort = null;
                        }
                    }
                    else if (InRegistry && _humanQueue.Snapshot().FirstOrDefault(e => e.Value.Party != null) is { } waitingParty)
                    {
                        // A forwarded party enters the running match alone and
                        // atomically; solos wait for the next pass.
                        cohort = new[] { waitingParty };
                        lateParty = true;
                    }
                    else
                    {
                        // Late admission preserves the chosen pair and observes
                        // existing per-side occupancy before considering a ticket.
                        cohort=_humanQueue.Snapshot().Where(e=>e.Human.Size==1&&e.Human.GameType==profile.GameType&&
                            (e.Human.Faction==profile.AttackingFaction||e.Human.Faction==profile.DefendingFaction)&&
                            _reservations.Values.Count(r=>IsLiveCohort(r)&&r.Faction==e.Human.Faction)<profile.TeamCapacity)
                            .Take(_reservations.Values.Count(IsLiveCohort)<Math.Min(_maximumPlayers,profile.MaximumPlayers)?1:0).ToArray();
                    }
                }
                if (cohort == null) { AbandonRegistryFormation(); return; }
                var parties = cohort.Where(ticket => ticket.Value.Party != null).ToArray();
                if (parties.Length != 0)
                {
                    if (!await AdmitRegistryPartiesAsync(parties, profile, formation, lateParty))
                    {
                        if (lateParty)
                        {
                            // Never abandon a running match for one refused party.
                            // Its registry ticket is already returned for placement.
                            lock (_gate)
                            {
                                foreach (var party in parties) _humanQueue.Remove(party);
                                _nextLatePartyAttempt = DateTime.UtcNow + LatePartyBackoff;
                            }
                            return;
                        }
                        lock (_gate) _nextFormationAttempt = DateTime.UtcNow.AddSeconds(1);
                        AbandonRegistryFormation();
                        return;
                    }
                    lock (_gate) foreach (var party in parties) _humanQueue.Remove(party);
                }
                foreach(var ticket in cohort)
                {
                    bool TicketCurrent() => _humanQueue.IsCurrent(ticket) && ticket.Value.Alive;
                    lock(_gate)if(!ReadyForAdmission||_matchmakingCancellation.IsCancellationRequested||!TicketCurrent())continue;
                    if (ticket.Value.Party != null) continue;
                    var reply=await JoinAsync(ticket.Value.Session,ticket.Value.Request,ticket.Value.Notify,TicketCurrent);
                    bool current;lock(_gate){current=_humanQueue.IsCurrent(ticket)&&ticket.Value.Alive;
                        if(InRegistry||reply.Result!=6||!current)_humanQueue.Remove(ticket);
                        if(reply.Result==0){_formationEverAdmitted=true;_registryFormationStaged=false;}}
                    if(!current&&reply.Result==0)
                    {
                        await ReleaseCancelledQueueAdmissionAsync(ticket.Value.Session, ticket.Value.Request);
                    }
                    else if (current && InRegistry)
                    {
                        bool send;
                        lock (_gate) send = !_disposed && ticket.Value.Alive && (reply.Result != 0 ||
                            _reservations.Values.Any(r => IsLiveCohort(r) && !r.Invalidated &&
                                OwnsQueueRequest(r, ticket.Value.Session, ticket.Value.Request)));
                        if (send) ticket.Value.Send(reply);
                    }
                    else if(current&&reply.Result!=6)lock(_gate)
                    {
                        if(!_disposed&&ticket.Value.Alive&&(reply.Result!=0||_reservations.Values.Any(r=>
                            IsLiveCohort(r)&&r.Account==ticket.Value.Account&&r.CharacterId==ticket.Value.Character&&
                            r.Client.SessionKey==ticket.Value.Session.SessionKey&&!r.Invalidated)))ticket.Value.Send(reply);
                    }
                }
            }
            catch(Exception e){Logger.Error($"Faction matchmaking stopped this attempt: {e.GetType().Name}");lock(_gate)_nextFormationAttempt=DateTime.UtcNow.AddSeconds(1);}
        }
        async Task StopFactionMatchmakingAsync()
        {
            _matchmakingCancellation.Cancel();Task worker;
            lock(_gate){foreach(var q in _humanQueue.Snapshot())_humanQueue.Remove(q);worker=_matchmakingWorker;}
            try{await worker.WaitAsync(TimeSpan.FromSeconds(5));}
            catch(TimeoutException){Logger.Error("Faction matchmaking disposal exceeded its bounded bridge deadline.");}
        }
    }
}
