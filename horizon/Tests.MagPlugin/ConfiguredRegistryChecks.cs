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
        // One human may start a bot-filled worker: minimum 1 and zero team minima are valid.
        var solo=Configuration();solo["workers"]![0]!["minimum_players"]=1;solo["workers"]![0]!["minimum_players_per_team"]=new JsonArray(0,0);
        var soloConfig=MagWorldRegistryConfig.Parse(solo.ToJsonString());
        Check(soloConfig.Workers[0].MinimumPlayers==1 && soloConfig.Workers[0].MinimumPlayersPerTeam.SequenceEqual(new[]{0,0}),"One-human registry minima were not loaded");
        var wrong=Configuration();wrong["workers"]![0]!["minimum_players"]=0;Refused(wrong);
        wrong=Configuration();wrong["workers"]![0]!["minimum_players_per_team"]=new JsonArray(-1,1);Refused(wrong);
        wrong=Configuration();wrong["workers"]![0]!["minimum_players_per_team"]=new JsonArray(33,0);Refused(wrong);
        wrong=Configuration();wrong["workers"]![0]!["minimum_players_per_team"]=new JsonArray(2,1);Refused(wrong);
        wrong=Configuration();wrong["client_address"]="0.0.0.0";Refused(wrong);
        wrong=Configuration();wrong["workers"]![0]!["worker_host"]="https://worker";Refused(wrong);
        wrong=Configuration();wrong["unexpected"]=true;Refused(wrong);
        var drain=Configuration();drain["workers"]![1]!["draining"]=true;
        config.ValidateDrainReload(MagWorldRegistryConfig.Parse(drain.ToJsonString()));
        drain["workers"]![1]!["worker_port"]=31003;
        try {config.ValidateDrainReload(MagWorldRegistryConfig.Parse(drain.ToJsonString()));throw new Exception("Structural live reload accepted");} catch(ArgumentException) { }
        try {MagWorldRegistryConfig.Parse(original.ToJsonString().Replace("\"application_id\":20231","\"application_id\":20231,\"application_id\":20231"));throw new Exception("Duplicate property accepted");} catch(ArgumentException) { }
        AdvertisedAddress();
        Placement();
    }
    static void AdvertisedAddress()
    {
        var placeholder=Configuration();placeholder["client_address"]=MagWorldRegistryConfig.AdvertisedAddressPlaceholder;
        Func<string,string> Env(string value)=>name=>name=="MAG_ADVERTISE_IPV4"?value:null;
        var config=MagWorldRegistryConfig.Parse(placeholder.ToJsonString(),Env("203.0.113.10"));
        Check(config.ClientAddress.ToString()=="203.0.113.10","Placeholder client address did not follow MAG_ADVERTISE_IPV4");
        var drain=Configuration();drain["client_address"]=MagWorldRegistryConfig.AdvertisedAddressPlaceholder;drain["workers"]![0]!["draining"]=true;
        config.ValidateDrainReload(MagWorldRegistryConfig.Parse(drain.ToJsonString(),Env("203.0.113.10")));
        try {config.ValidateDrainReload(MagWorldRegistryConfig.Parse(drain.ToJsonString(),Env("203.0.113.11")));throw new Exception("Changed advertised address reloaded live");} catch(ArgumentException) { }
        foreach(var bad in new[]{null,"","0.0.0.0","203.0.113","not-an-address"})
        {try {MagWorldRegistryConfig.Parse(placeholder.ToJsonString(),Env(bad));throw new Exception("Invalid advertised address accepted");} catch(ArgumentException) { }}
        // Only the exact placeholder reads the environment; a literal address ignores it.
        Check(MagWorldRegistryConfig.Parse(Configuration().ToJsonString(),Env("203.0.113.10")).ClientAddress.ToString()=="127.0.0.1","Literal client address was replaced");
        var near=Configuration();near["client_address"]="$MAG_ADVERTISE_IPV4 ";
        try {MagWorldRegistryConfig.Parse(near.ToJsonString(),Env("203.0.113.10"));throw new Exception("Inexact placeholder accepted");} catch(ArgumentException) { }
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
        // A worker that fills its profiles with bots forms from the lone oldest human.
        var solo=new MagWorldPlacementState("bots",true,false,1,2,new[]{0,0},
            new[]{new MagMatchProfile("a-b",1,0,1,2,1,BotFillTotal:2),new MagMatchProfile("a-c",1,0,2,2,1,BotFillTotal:2),
                new MagMatchProfile("b-c",1,1,2,2,1,BotFillTotal:2)},null,new[]{0,0,0},new[]{0,0,0},0,0,true);
        plan=MagWorldPlacement.PlanNext(humans.Take(1).ToArray(),new[]{World("idle"),solo},n=>0);
        Check(plan!=null && plan.WorkerId=="bots" && plan.NewFormation && plan.Players.Single().TicketId=="oldest" &&
            (plan.Profile.AttackingFaction==2 || plan.Profile.DefendingFaction==2),"Bot-filled worker did not start a one-human match");
    }
}
