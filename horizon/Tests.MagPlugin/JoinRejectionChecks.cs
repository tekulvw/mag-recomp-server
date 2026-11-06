using RT.Models.MAG;
using Server.Medius;

// Synthetic join requests: an unsupported request is rejected with a named
// reason instead of a silent failure response.
static class JoinRejectionChecks
{
    static void Check(bool value,string message) {if(!value)throw new Exception(message);}
    static string Reason(MagJoinGameRequest request,bool retail=true,bool modeCurrent=true)=>
        LocalDmeHost.JoinRequestRejection(request,retail,_=>modeCurrent);
    public static void Run()
    {
        Check(Reason(new MagRetailJoinGameRequest{Preference=1})==null,"Supported retail join accepted");
        Check(Reason(new MagRetailJoinGameRequest{Preference=5})=="preference_unsupported","Unsupported preference named");
        Check(Reason(new MagRetailJoinGameRequest{Preference=1,PartyId=7})=="party_join_unsupported","Party join named");
        Check(Reason(new MagRetailJoinGameRequest{Preference=1},modeCurrent:false)=="mode_not_current","Mode named");
        Check(Reason(new MagRetailJoinGameRequest{Preference=1,ClientType=1})=="client_type_unsupported","Client type named");
        Check(Reason(new MagJoinGameRequest{Preference=1})=="request_application_mismatch","Prototype request on retail named");
        Check(Reason(new MagJoinGameRequest{Preference=1},retail:false)==null,"Prototype request on prototype accepted");
    }
}
