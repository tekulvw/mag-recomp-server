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
            public bool Alive => Connected() && Session.State==MagMlsSession.SessionState.Ready &&
                !Session.IsExpired(DateTime.UtcNow) && Session.AuthenticatedAccount==Account && Session.CharacterId==Character;
        }
        IReadOnlyList<MagMatchProfile> _matchProfiles=Array.Empty<MagMatchProfile>();
        readonly MagPendingHumanQueue<QueuedJoin> _humanQueue=new();
        readonly CancellationTokenSource _matchmakingCancellation=new();
        Task _matchmakingWorker=Task.CompletedTask;
        MagMatchStartupPolicy _formationStartup;
        int[] _formationTeamMinima;
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
            var configured=Program.Settings.MagMinimumPlayersPerTeam??new[]{1,1};
            if(configured.Length!=2||configured.Any(n=>n<0))throw new IOException("Matchmaking requires two nonnegative human team minima.");
            _formationTeamMinima=configured.Select(n=>Math.Max(1,n)).ToArray();
            _formationStartup=new MagMatchStartupPolicy(Math.Max(2,_minimumPlayers),_maximumPlayers,_formationTeamMinima);
            _matchProfiles=hello.MatchProfiles;
        }
        bool RequestedModeIsCurrent(uint mode)=>UsesFactionMatchmaking ?
            _formedProfile!=null && _formationConfigured && mode==(uint)_formedProfile.GameType : mode==(uint)_gameType;

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
            if(!UsesFactionMatchmaking||session==null||request is not MagRetailJoinGameRequest retail||retail.PartyId!=0||
                session.ApplicationId!=20231||session.State!=MagMlsSession.SessionState.Ready||session.IsExpired(DateTime.UtcNow)||
                session.AccountId==0||session.CharacterId==0||!MagDevelopmentAccountPolicy.ValidAccount(session.AuthenticatedAccount)||
                request.Preference!=1||request.ClientType!=0||!_matchProfiles.Any(p=>(uint)p.GameType==request.Value)||!IsReady||!connected())
            {send(JoinResponse(request.TransactionId,2));return;}
            try
            {
                var character=_store.ReadForAccount(session.AuthenticatedAccount).SingleOrDefault(c=>c.Id==session.CharacterId);
                if(character==null||character.Faction>2){send(JoinResponse(request.TransactionId,2));return;}
                var queued=new QueuedJoin {Session=session,Request=request,Account=session.AuthenticatedAccount,
                    Character=session.CharacterId,Connected=connected,Send=send,Notify=notify};
                lock(_gate)
                {
                    if(_disposed||_reservations.Values.Any(r=>r.Account==queued.Account&&r.TerminalMatchId==null))
                    {send(JoinResponse(request.TransactionId,6));return;}
                    _humanQueue.Put(queued.Account,(int)request.Value,character.Faction,queued);
                }
                SendQueueNotices(true);
                StartMatchmakingPump();
            }
            catch(Exception e) when(e is IOException||e is InvalidDataException||e is UnauthorizedAccessException||e is ArgumentException||e is InvalidOperationException)
            {send(JoinResponse(request.TransactionId,2));}
        }
        public void CancelQueuedJoin(MagMlsSession session)
        {
            if(session==null)return;
            lock(_gate)
            {
                var ticket=_humanQueue.Snapshot().FirstOrDefault(e=>ReferenceEquals(e.Value.Session,session));
                if(ticket!=null)_humanQueue.Remove(ticket);
            }
            // A cancel can race the successful join reply. Release only this
            // session's still-unclaimed reservation, never an active human.
            _=ReleaseCancelledQueueAdmissionAsync(session);
        }
        async Task ReleaseCancelledQueueAdmissionAsync(MagMlsSession session)
        {
            try
            {
                Task release=null;
                lock(_gate)
                {
                    var pending=_reservations.Values.FirstOrDefault(r=>r.Account==session.AuthenticatedAccount&&
                        r.CharacterId==session.CharacterId&&!r.Claimed);
                    if(pending!=null){pending.ReleaseReason="queued_join_cancelled";release=ReleaseAsync(pending);}
                }
                if(release!=null)await release;
            }
            catch(Exception e){Logger.Error($"Queued cancellation cleanup: {e.GetType().Name}");}
        }
        void PurgeHumanQueue()
        {
            foreach(var ticket in _humanQueue.Snapshot())if(!ticket.Value.Alive)_humanQueue.Remove(ticket);
        }
        void AddQueuedStatus(MagServerStatusResponse status)
        {
            if(!UsesFactionMatchmaking)return;
            PurgeHumanQueue();
            status.MinimumPlayers=Enumerable.Repeat((uint)_formationStartup.MinimumPlayers,4).ToArray();
            foreach(var ticket in _humanQueue.Snapshot())if(!_reservations.Values.Any(r=>r.Accepted&&
                r.Account==ticket.Value.Account&&r.CharacterId==ticket.Value.Character))
                status.QueueSizes[ticket.Human.GameType*3+ticket.Human.Faction]++;
            if(_ready&&_worldStreamHealthy&&!_disposed)
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
                if(!UsesFactionMatchmaking||_disposed||!_ready||!_worldStreamHealthy||!_matchmakingWorker.IsCompleted||
                    DateTime.UtcNow<_nextFormationAttempt||_humanQueue.Count==0)return;
                _matchmakingWorker=Task.Run(PumpFactionMatchmakingAsync);
            }
        }
        void ResetFormation()
        {_formedProfile=null;_formationId=null;_formationConfigured=false;_formationEverAdmitted=false;}
        async Task PumpFactionMatchmakingAsync()
        {
            try
            {
                Reservation[] results;
                lock(_gate)results=_reservations.Values.Where(r=>r.TerminalMatchId!=null&&
                    _humanQueue.Snapshot().Any(q=>q.Value.Account==r.Account&&q.Value.Character==r.CharacterId)).ToArray();
                foreach(var r in results){r.ReleaseReason="results_requeue";await ReleaseAsync(r);}
                MagMatchProfile profile;string formation;
                lock(_gate)
                {
                    PurgeHumanQueue();
                    if(_disposed||!_worldStreamHealthy||_pendingReleases.Count!=0||_reservations.Values.Any(r=>r.TerminalMatchId!=null))return;
                    if(_formationEverAdmitted&&_reservations.Count==0)ResetFormation();
                    if(_formedProfile==null)
                    {
                        var selection=MagFactionMatchmaking.Select(_humanQueue.Snapshot().Select(e=>e.Human).ToArray(),
                            _matchProfiles,_formationStartup.MinimumPlayers,_maximumPlayers,_formationTeamMinima,RandomNumberGenerator.GetInt32);
                        if(selection==null)return;
                        _formedProfile=selection.Profile;_formationId=Id();
                    }
                    profile=_formedProfile;formation=_formationId;
                }
                if(!_formationConfigured)
                {
                    var reply=await _bridge.RequestControlAsync(new {v=2,request_id=Id(),op="select_match_profile",formation_id=formation,profile_id=profile.ProfileId});
                    if(!MatchesFormationAcknowledgement(reply,formation,profile))
                    {
                        lock(_gate){_nextFormationAttempt=DateTime.UtcNow.AddSeconds(1);if(reply!=null&&!reply.Ok&&reply.Failure==GameplayBridgeFailure.None)ResetFormation();}
                        return; // An uncertain selection is retried with the SAME formation identity.
                    }
                    lock(_gate)if(_disposed||_matchmakingCancellation.IsCancellationRequested)return;
                    if(!await _bridge.RequestAsync(_formationStartup.ConfigureRequest(Id(),_applicationId)))
                    {lock(_gate)_nextFormationAttempt=DateTime.UtcNow.AddSeconds(1);return;}
                    lock(_gate){if(_disposed||_matchmakingCancellation.IsCancellationRequested)return;_formationConfigured=true;}
                }
                MagPendingHumanQueue<QueuedJoin>.Entry[] cohort;
                lock(_gate)
                {
                    PurgeHumanQueue();
                    if(_disposed||_matchmakingCancellation.IsCancellationRequested)return;
                    if(_reservations.Count==0)
                    {
                        var selection=MagFactionMatchmaking.ForProfile(_humanQueue.Snapshot().Select(e=>e.Human).ToArray(),profile,
                            _formationStartup.MinimumPlayers,_maximumPlayers,_formationTeamMinima);
                        if(selection==null){ResetFormation();return;}
                        var ids=selection.Players.Select(p=>p.TicketId).ToHashSet(StringComparer.Ordinal);
                        cohort=_humanQueue.Snapshot().Where(e=>ids.Contains(e.Human.TicketId)).ToArray();
                    }
                    else
                    {
                        // Late admission preserves the chosen pair and observes
                        // existing per-side occupancy before considering a ticket.
                        cohort=_humanQueue.Snapshot().Where(e=>e.Human.GameType==profile.GameType&&
                            (e.Human.Faction==profile.AttackingFaction||e.Human.Faction==profile.DefendingFaction)&&
                            _reservations.Values.Count(r=>r.Faction==e.Human.Faction)<profile.TeamCapacity)
                            .Take(_reservations.Count<Math.Min(_maximumPlayers,profile.MaximumPlayers)?1:0).ToArray();
                    }
                }
                foreach(var ticket in cohort)
                {
                    lock(_gate)if(_disposed||_matchmakingCancellation.IsCancellationRequested||!_humanQueue.IsCurrent(ticket)||!ticket.Value.Alive)continue;
                    var reply=await JoinAsync(ticket.Value.Session,ticket.Value.Request,ticket.Value.Notify);
                    bool current;lock(_gate){current=_humanQueue.IsCurrent(ticket)&&ticket.Value.Alive;
                        if(reply.Result!=6||!current)_humanQueue.Remove(ticket);
                        if(reply.Result==0)_formationEverAdmitted=true;}
                    if(!current&&reply.Result==0)
                    {
                        Reservation owned;lock(_gate)owned=_reservations.Values.FirstOrDefault(r=>r.Account==ticket.Value.Account&&r.CharacterId==ticket.Value.Character);
                        if(owned!=null){owned.ReleaseReason="queued_join_cancelled";await ReleaseAsync(owned);}
                    }
                    else if(current&&reply.Result!=6)lock(_gate)
                    {
                        if(!_disposed&&ticket.Value.Alive&&(reply.Result!=0||_reservations.Values.Any(r=>
                            r.Account==ticket.Value.Account&&r.CharacterId==ticket.Value.Character&&!r.Invalidated)))ticket.Value.Send(reply);
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
