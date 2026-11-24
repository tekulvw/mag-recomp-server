using System.Text.Json;
using Server.Medius;

static class MatchProfileBridgeChecks
{
    static void Check(bool value,string message) {if(!value)throw new Exception(message);}
    public static void Run()
    {
        var profile=new {profile_id="synthetic-sab-raven-valor",game_type=0,attacking_faction=1,
            defending_faction=0,maximum_players=64,team_capacity=32};
        var selected=new {formation_id="a".PadLeft(32,'a'),profile_id=profile.profile_id,game_type=0,
            attacking_faction=1,defending_faction=0};
        byte[] Reply(object profiles,object selection=null,bool ok=true)=>JsonSerializer.SerializeToUtf8Bytes(new {
            v=2,request_id="synthetic",ok,error=ok?null:"rejected",messages=Array.Empty<object>(),
            capabilities=new[]{"match_profiles_v1"},match_profiles=profiles,selected_match=selection});
        Check(GameplayBridgeResponseCodec.TryParse(Reply(new[]{profile}),"synthetic",true,out var hello),"Profile hello rejected");
        Check(hello.MatchProfiles.Count==1&&hello.MatchProfiles[0].DefendingFaction==0,"Typed profile identity");
        Check(GameplayBridgeResponseCodec.TryParse(Reply(Array.Empty<object>(),selected),"synthetic",false,out var ack),"Selection ack rejected");
        Check(ack.SelectedMatch.FormationId==selected.formation_id&&ack.SelectedMatch.ProfileId==profile.profile_id,"Selection identity");
        var expected=new MagMatchProfile(profile.profile_id,0,1,0,64,32);
        Check(LocalDmeHost.MatchesFormationAcknowledgement(ack,selected.formation_id,expected),"Exact formation ack");
        Check(!LocalDmeHost.MatchesFormationAcknowledgement(ack,"b".PadLeft(32,'b'),expected),"Wrong formation ack accepted");
        Check(!LocalDmeHost.MatchesFormationAcknowledgement(ack,selected.formation_id,expected with {AttackingFaction=2}),"Wrong faction ack accepted");
        Check(!GameplayBridgeResponseCodec.TryParse(Reply(new[]{profile,profile}),"synthetic",true,out _),"Duplicate profile accepted");
        Check(!GameplayBridgeResponseCodec.TryParse(Reply(new[]{profile with { }}),"wrong",true,out _),"Correlation mismatch accepted");
        Check(!GameplayBridgeResponseCodec.TryParse(Reply(Array.Empty<object>(),selected,false),"synthetic",false,out _),"Failed selection carried success identity");
        var dominationProfile=profile with {game_type=2};
        var dominationSelection=selected with {game_type=2};
        Check(GameplayBridgeResponseCodec.TryParse(Reply(new[]{dominationProfile}),"synthetic",true,out var dominationHello)
            && dominationHello.MatchProfiles[0].GameType==2,"Domination hello rejected");
        Check(GameplayBridgeResponseCodec.TryParse(Reply(Array.Empty<object>(),dominationSelection),"synthetic",false,out var dominationAck)
            && LocalDmeHost.MatchesFormationAcknowledgement(dominationAck,selected.formation_id,expected with {GameType=2}),
            "Domination selection rejected");
        var filled=new {profile_id=profile.profile_id,game_type=0,attacking_faction=1,defending_faction=0,maximum_players=64,team_capacity=32,bot_fill_total=64};
        Check(GameplayBridgeResponseCodec.TryParse(Reply(new[]{filled}),"synthetic",true,out var filledHello) &&
            filledHello.MatchProfiles[0].BotFillTotal==64 && hello.MatchProfiles[0].BotFillTotal==null,"Bot fill total was not typed per profile");
        foreach(int total in new[]{1,65})
            Check(!GameplayBridgeResponseCodec.TryParse(Reply(new[]{filled with {bot_fill_total=total}}),"synthetic",true,out _),"Out-of-range bot fill accepted");
        var bad=new {profile_id="bad",game_type=0,attacking_faction=0,defending_faction=0,maximum_players=64,team_capacity=32};
        Check(!GameplayBridgeResponseCodec.TryParse(Reply(new[]{bad}),"synthetic",true,out _),"Same faction accepted");
    }
}
