using System.Text.Json;
using Server.Medius;

// Synthetic reserve replies: a join whose gameplay reserve is refused must be
// released under a named admission reason, never the transport_closed default.
static class AdmissionFailureChecks
{
    static void Check(bool value,string message) {if(!value)throw new Exception(message);}
    static GameplayBridgeResponse Reply(bool ok,string error)
    {
        var line=JsonSerializer.SerializeToUtf8Bytes(new {v=2,request_id="synthetic-reserve",ok,error,messages=Array.Empty<object>()});
        Check(GameplayBridgeResponseCodec.TryParse(line,"synthetic-reserve",false,out var reply),"Synthetic reserve reply rejected");
        return reply;
    }
    public static void Run()
    {
        Check(LocalDmeHost.AdmissionFailureReason(null,true)=="admission_unanswered","Missing reply");
        var rejected=Reply(false,"synthetic_reserve_refusal");
        Check(rejected.Error=="synthetic_reserve_refusal","Reserve error preserved for logging");
        Check(LocalDmeHost.AdmissionFailureReason(rejected,true)=="admission_rejected","Rejected reserve");
        Check(LocalDmeHost.AdmissionFailureReason(rejected,false)=="admission_rejected","Rejection outranks readiness");
        var accepted=Reply(true,null);
        Check(LocalDmeHost.AdmissionFailureReason(accepted,false)=="admission_not_ready","Accepted while not ready");
        Check(LocalDmeHost.AdmissionFailureReason(accepted,true)!="transport_closed","Never the transport default");
    }
}
