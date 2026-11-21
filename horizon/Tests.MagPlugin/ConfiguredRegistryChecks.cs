using System.Text.Json.Nodes;
using Server.Medius;

internal static class ConfiguredRegistryChecks
{
    static void Check(bool ok,string message) {if(!ok)throw new Exception(message);}
    static JsonObject Configuration()=>new() {
        ["format"]="mag_world_registry_v1",["application_id"]=20231,["client_address"]="127.0.0.1",["client_tcp_port"]=10473,
        ["workers"]=new JsonArray(Worker("first","127.0.0.1",31001),Worker("second","127.0.0.1",31002))
    };
    static JsonObject Worker(string id,string host,int port)=>new() {
        ["worker_id"]=id,["worker_host"]=host,["worker_port"]=port,["minimum_players"]=2,["maximum_players"]=2,
        ["minimum_players_per_team"]=new JsonArray(1,1),["draining"]=false
    };
    static void Refused(JsonObject value) {try {MagWorldRegistryConfig.Parse(value.ToJsonString());} catch(ArgumentException) {return;} throw new Exception("Invalid registry was accepted");}
    public static void Run()
    {
        var original=Configuration();var config=MagWorldRegistryConfig.Parse(original.ToJsonString());
        Check(config.Workers.Count==2,"Two explicit workers were not loaded");
        Check(config.Workers[0].WorkerId=="first" && config.Workers[1].WorkerPort==31002,"Worker endpoint/identity changed");
        foreach(var field in new[]{"format","application_id","client_address","client_tcp_port","workers"}) {var bad=Configuration();bad.Remove(field);Refused(bad);}
        foreach(var badPort in new[]{0,65536}) {var bad=Configuration();bad["workers"]![0]!["worker_port"]=badPort;Refused(bad);}
        foreach(var field in new[]{"worker_id","worker_host","worker_port","minimum_players","maximum_players","minimum_players_per_team","draining"}) {var bad=Configuration();bad["workers"]![0]!.AsObject().Remove(field);Refused(bad);}
        var duplicate=Configuration();duplicate["workers"]![1]!["worker_id"]="first";Refused(duplicate);
        duplicate=Configuration();duplicate["workers"]![1]!["worker_port"]=31001;Refused(duplicate);
        var wrong=Configuration();wrong["workers"]![0]!["minimum_players_per_team"]=new JsonArray(0,1);Refused(wrong);
        wrong=Configuration();wrong["client_address"]="0.0.0.0";Refused(wrong);
        wrong=Configuration();wrong["workers"]![0]!["worker_host"]="https://worker";Refused(wrong);
        wrong=Configuration();wrong["unexpected"]=true;Refused(wrong);
        var drain=Configuration();drain["workers"]![1]!["draining"]=true;
        config.ValidateDrainReload(MagWorldRegistryConfig.Parse(drain.ToJsonString()));
        drain["workers"]![1]!["worker_port"]=31003;
        try {config.ValidateDrainReload(MagWorldRegistryConfig.Parse(drain.ToJsonString()));throw new Exception("Structural live reload accepted");} catch(ArgumentException) { }
        try {MagWorldRegistryConfig.Parse(original.ToJsonString().Replace("\"application_id\":20231","\"application_id\":20231,\"application_id\":20231"));throw new Exception("Duplicate property accepted");} catch(ArgumentException) { }
        Placement();
    }
    static MagWorldPlacementState World(string id,bool ready=true,bool draining=false,MagMatchProfile current=null,int[] counts=null)=>new(
        id,ready,draining,2,2,new[]{1,1},
        new[]{new MagMatchProfile("a-b",1,0,1,2,1),new MagMatchProfile("a-c",1,0,2,2,1),new MagMatchProfile("b-c",1,1,2,2,1)},current,
        counts??new[]{0,0,0},new[]{0,0,0},counts?.Sum()??0,0,current==null);
    static void Placement()
    {
        var humans=new[]{new MagQueuedHuman("oldest",1,2,0),new MagQueuedHuman("opponent",1,0,1),new MagQueuedHuman("third",1,1,2)};
        var plan=MagWorldPlacement.PlanNext(humans,new[]{World("dead",false),World("drain",true,true),World("ready")},n=>0);
        Check(plan!=null && plan.WorkerId=="ready" && plan.NewFormation,"Unhealthy/drained worker received a new cohort");
        Check(plan.Players.Select(p=>p.TicketId).SequenceEqual(new[]{"oldest","opponent"}),"Oldest eligible faction lost priority");
        var live=new MagMatchProfile("a-b",1,0,1,2,1);
        plan=MagWorldPlacement.PlanNext(humans,new[]{World("idle"),World("live",current:live,counts:new[]{1,0,0})},n=>0);
        Check(plan!=null && plan.WorkerId=="live" && !plan.NewFormation && plan.Players.Single().TicketId=="third","Compatible running world was not filled first");
        plan=MagWorldPlacement.PlanNext(humans,new[]{World("full",current:live,counts:new[]{1,1,0}),World("idle")},n=>0);
        Check(plan!=null && plan.WorkerId=="idle" && plan.NewFormation,"Full world blocked a second independent match");
        Check(MagWorldPlacement.PlanNext(humans.Take(1).ToArray(),new[]{World("idle")},n=>0)==null,"One faction formed a new match");
    }
}
