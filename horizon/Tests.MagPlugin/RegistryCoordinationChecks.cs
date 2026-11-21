using System.Collections;
using System.Reflection;
using System.Text.Json.Nodes;
using RT.Models;
using RT.Models.MAG;
using Server.Medius;

internal static class RegistryCoordinationChecks
{
    public static void Run(){LeadershipProgressionChecks.RegistryCoordination();LeadershipProgressionChecks.RegistryReviewBoundaries();LeadershipProgressionChecks.RegistryTransportFailure();}
}
internal static partial class LeadershipProgressionChecks
{
    public static void RegistryCoordination()=>WithDirectory(directory=>
    {
        int app=Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId=20231;
        try {RunAsync().GetAwaiter().GetResult();}
        finally {Server.Medius.Program.Settings.MagApplicationId=app;}
        async Task RunAsync()
        {
            var store=new LocalCharacterStore(Path.Combine(directory,"characters.json"));
            foreach(var (account,faction) in new[]{("alpha",(byte)0),("bravo",(byte)1),("charlie",(byte)0),("delta",(byte)1),("echo",(byte)1),("foxtrot",(byte)0),("golf",(byte)1)})
                Check(store.CreateForAccount(new MagCreateCharacterRequest {AccountName=account,Faction=faction,UniverseId=26,NpId=new byte[36]},account).Result==MagNetResult.Success,"Synthetic registry character creation failed");
            await using var a=new RegistryWorker("first"); await using var b=new RegistryWorker("second");
            JsonObject Worker(string id,int port)=>new(){["worker_id"]=id,["worker_host"]="127.0.0.1",["worker_port"]=port,["minimum_players"]=2,["maximum_players"]=2,["minimum_players_per_team"]=new JsonArray(1,1),["draining"]=false};
            var config=new JsonObject {["format"]="mag_world_registry_v1",["application_id"]=20231,["client_address"]="192.0.2.9",["client_tcp_port"]=12073,["workers"]=new JsonArray(Worker("first",a.Port),Worker("second",b.Port))};
            await using var registry=new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()),store);
            await registry.InitializeAsync();
            Check(registry.Worlds.Count==2 && registry.Worlds.All(w=>w.Ready) && registry.Worlds.Select(w=>w.WorldId).Distinct().Count()==2,"Independent registry worlds failed initialization");
            var hosts=((IEnumerable)typeof(MagWorldRegistry).GetField("_slots",HostInternals)!.GetValue(registry)!).Cast<object>().Select(s=>(LocalDmeHost)s.GetType().GetField("Host")!.GetValue(s)!).ToArray();
            MagMlsSession Session(string account)=>MakeSession(store,20231,account,store.ReadForAccount(account).Single().Id);
            int transaction=0;
            Task<MagJoinGameResponse> Queue(MagMlsSession session)
            {
                var completion=new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                registry.QueueJoin(session,new MagRetailJoinGameRequest {TransactionId=++transaction,Preference=1,Value=2},m=>{if(m is MagJoinGameResponse r)completion.TrySetResult(r);},()=>true);
                return completion.Task;
            }
            async Task Until(Func<bool> condition)
            {
                using var deadline=new CancellationTokenSource(TimeSpan.FromSeconds(10));
                while(!condition()) {await registry.TickAsync();await Task.Delay(10,deadline.Token);}
            }
            var alpha=Session("alpha"); var first=Queue(alpha);
            await registry.TickAsync(); await Task.Delay(30);
            Check(a.Reservations.Count==0 && b.Reservations.Count==0 && registry.Status(1).QueueSizes[6]==1,"Global queue scattered or admitted a lone faction");
            var second=Queue(Session("bravo"));await Until(()=>first.IsCompleted&&second.IsCompleted);
            Check((await first).Result==0 && (await second).Result==0 && (await first).WorldId==(uint)hosts[0].World.WorldId,"First global cohort was not admitted to its acknowledged world");
            var third=Queue(Session("charlie"));var fourth=Queue(Session("delta"));await Until(()=>third.IsCompleted&&fourth.IsCompleted);
            Check((await third).Result==0 && (await fourth).Result==0 && (await third).WorldId==(uint)hosts[1].World.WorldId && (await third).WorldId!=(await first).WorldId,"Full first world blocked the second simultaneous match");
            Check(registry.Worlds.All(w=>w.ReservedPlayers==2),"Two simultaneous live worlds were not retained");
            Check((await Queue(alpha)).Result==6 && a.Reservations.Count==2 && b.Reservations.Count==2,"One account was admitted into two live workers");
            var old=a.Reservations.Single(r=>r["account_id"]!.GetValue<uint>()==alpha.CharacterId);
            var client=ConnectFixture(hosts[0],old);
            Check(registry.CanUse(client),"Registry refused its exact admitted client");
            registry.Event(client,"tcp",61,1,new byte[]{0x02,0x1c,0x03,0x80,0,0});
            await Until(()=>a.Requests.Any(r=>r["op"]!.GetValue<string>()=="event"));
            Check(!b.Requests.Any(r=>r["op"]!.GetValue<string>()=="event"),"Client gameplay crossed worker endpoints");
            Check(!registry.TryBind(client,new RT_MSG_CLIENT_CONNECT_TCP_AUX_UDP {AppId=20231,ARG1=(uint)hosts[1].World.WorldId,SessionKey=client.SessionKey,AccessToken=client.Token}),"A stale/wrong world token crossed registry ownership");

            await a.PublishAsync(TerminalReply("synthetic-registry-first",a.Reservations.ToArray()));
            await Until(()=>hosts[0].GetAccountOwnership("alpha")==LocalDmeAccountOwnership.Terminal);
            config["workers"]![0]!["draining"]=true; registry.ApplyDrainPolicy(MagWorldRegistryConfig.Parse(config.ToJsonString()));
            var transferred=Queue(alpha); var partner=Queue(Session("echo"));
            await registry.TickAsync(); await Task.Delay(30);
            Check(!transferred.IsCompleted && a.Reservations.Count==2,"Drained worker accepted a successor");
            await b.PublishAsync(TerminalReply("synthetic-registry-second",b.Reservations.ToArray()));
            await Until(()=>transferred.IsCompleted&&partner.IsCompleted);
            Check((await transferred).Result==0 && (await transferred).WorldId==(uint)hosts[1].World.WorldId && client.IsDestroyed,"Terminal transfer did not close old transport and reserve the independent next world");
            Check(hosts[0].GetAccountOwnership("bravo")==LocalDmeAccountOwnership.Terminal,"Unrelated old results were removed during transfer");

            // An identity fault cannot be cleared by an operator drain toggle;
            // the healthy worker remains selectable for another fresh cohort.
            var wrong=CohortReply(); wrong["worker_identity"]=new JsonObject {["worker_id"]="first",["incarnation_id"]="replacement-process"};
            await a.PublishAsync(wrong); await Until(()=>!registry.Worlds[0].Ready);
            config["workers"]![0]!["draining"]=false; registry.ApplyDrainPolicy(MagWorldRegistryConfig.Parse(config.ToJsonString()));
            Check(registry.Worlds[0].Draining && registry.Worlds[1].Ready,"Worker restart was silently adopted or stopped the unrelated world");
            await b.PublishAsync(TerminalReply("synthetic-registry-third",b.Reservations.Skip(2).ToArray()));
            var fifth=Queue(Session("foxtrot"));var sixth=Queue(Session("golf"));await Until(()=>fifth.IsCompleted&&sixth.IsCompleted);
            Check((await fifth).Result==0 && (await sixth).Result==0 && (await fifth).WorldId==(uint)hosts[1].World.WorldId,"Failed worker blocked healthy-world placement");
        }
    });
    public static void RegistryReviewBoundaries()=>WithDirectory(directory=>
    {
        int app=Server.Medius.Program.Settings.MagApplicationId;Server.Medius.Program.Settings.MagApplicationId=20231;
        try {RunAsync().GetAwaiter().GetResult();} finally {Server.Medius.Program.Settings.MagApplicationId=app;}
        async Task RunAsync()
        {
            var store=new LocalCharacterStore(Path.Combine(directory,"characters.json"));
            foreach(var (name,faction) in new[]{("alpha",(byte)0),("bravo",(byte)1)})
                Check(store.CreateForAccount(new MagCreateCharacterRequest {AccountName=name,Faction=faction,UniverseId=26,NpId=new byte[36]},name).Result==MagNetResult.Success,"Review fixture creation failed");
            await using var a=new RegistryWorker("first");await using var b=new RegistryWorker("second");
            JsonObject Row(string id,int port,bool drain)=>new(){["worker_id"]=id,["worker_host"]="127.0.0.1",["worker_port"]=port,["minimum_players"]=2,["maximum_players"]=2,["minimum_players_per_team"]=new JsonArray(1,1),["draining"]=drain};
            var config=new JsonObject {["format"]="mag_world_registry_v1",["application_id"]=20231,["client_address"]="192.0.2.9",["client_tcp_port"]=12073,["workers"]=new JsonArray(Row("first",a.Port,true),Row("second",b.Port,false))};
            await using var registry=new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()),store);await registry.InitializeAsync();
            var hosts=((IEnumerable)typeof(MagWorldRegistry).GetField("_slots",HostInternals)!.GetValue(registry)!).Cast<object>().Select(o=>(LocalDmeHost)o.GetType().GetField("Host")!.GetValue(o)!).ToArray();
            config["workers"]![0]!["draining"]=false;config["workers"]![1]!["draining"]=true;
            Task update;
            // Hold the second host gate: publication must not toggle workers
            // one by one and expose the first worker before finishing the swap.
            var secondGate=typeof(LocalDmeHost).GetField("_gate",HostInternals)!.GetValue(hosts[1])!;
            lock(secondGate) {
                update=Task.Run(()=>registry.ApplyDrainPolicy(MagWorldRegistryConfig.Parse(config.ToJsonString())));
                Check(update.Wait(TimeSpan.FromSeconds(1)),"Drain reload waited on a host after partially publishing its new policy");
            }
            await update;
            Check(!registry.Worlds[0].Draining&&registry.Worlds[1].Draining,"Atomic drain policy did not swap both workers");
            var alpha=MakeSession(store,20231,"alpha",store.ReadForAccount("alpha").Single().Id);
            var bravo=MakeSession(store,20231,"bravo",store.ReadForAccount("bravo").Single().Id);
            using var deliver=new ManualResetEventSlim();var entered=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            registry.QueueJoin(alpha,new MagRetailJoinGameRequest {TransactionId=1,Preference=1,Value=2},message=>{
                if(message is MagJoinGameResponse {Result:0}) {entered.TrySetResult();if(!deliver.Wait(AdmissionTimeout))throw new TimeoutException("Blocked success callback expired");}
            },()=>true);
            registry.QueueJoin(bravo,new MagRetailJoinGameRequest {TransactionId=2,Preference=1,Value=2},_=>{},()=>true);
            try {
                using var deadline=new CancellationTokenSource(AdmissionTimeout);
                while(!entered.Task.IsCompleted){await registry.TickAsync();await Task.Delay(10,deadline.Token);}
                registry.CancelQueuedJoin(alpha);
                while((hosts[0].GetAccountOwnership("alpha")&(LocalDmeAccountOwnership.Live|LocalDmeAccountOwnership.PendingRelease))!=0){await registry.TickAsync();await Task.Delay(10,deadline.Token);}
                Check(a.Requests.Any(r=>r["op"]!.GetValue<string>()=="release"),"Cancel lost the acknowledged-but-unclaimed owner before success delivery");
            } finally {deliver.Set();}
        }
    });

    sealed class RefusingRegistryTcp : Server.Dme.TcpServer {
        public int Stops;
        public override Task StartAsync()=>Task.FromException(new IOException("Synthetic bind refusal"));
        public override Task Stop(){Stops++;return Task.CompletedTask;}
    }
    public static void RegistryTransportFailure()=>WithDirectory(directory=>
    {
        RunAsync().GetAwaiter().GetResult();
        async Task RunAsync() {
            await using var a=new RegistryWorker("first");await using var b=new RegistryWorker("second");
            JsonObject Row(string id,int port)=>new(){["worker_id"]=id,["worker_host"]="127.0.0.1",["worker_port"]=port,["minimum_players"]=2,["maximum_players"]=2,["minimum_players_per_team"]=new JsonArray(1,1),["draining"]=false};
            var config=new JsonObject {["format"]="mag_world_registry_v1",["application_id"]=20231,["client_address"]="192.0.2.9",["client_tcp_port"]=12073,["workers"]=new JsonArray(Row("first",a.Port),Row("second",b.Port))};
            var prior=Server.Dme.Program.TcpServer;var refused=new RefusingRegistryTcp();Server.Dme.Program.TcpServer=refused;
            try {
                await using var registry=new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()),new LocalCharacterStore(Path.Combine(directory,"characters.json")));
                try {await registry.StartAsync();throw new Exception("Synthetic bind failure was ignored");} catch(IOException) { }
                Check(refused.Stops==1 && Server.Dme.Program.LocalGameplay==null && !Server.Dme.Program.Managers.ContainsKey(20231),"Failed bind left owned global transport or manager resources alive");
            } finally {Server.Dme.Program.TcpServer=prior;}
        }
    });

}
