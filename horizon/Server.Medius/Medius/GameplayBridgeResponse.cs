using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using RT.Models;
namespace Server.Medius {
 public sealed class GameplayBridgeMessage {
  public string ReservationId {get;}
  public uint WorldId {get;}
  public ushort PlayerId {get;}
  public byte PluginId {get;}
  public byte[] Payload {get;}
  internal GameplayBridgeMessage(string reservationId,uint worldId,ushort playerId,byte pluginId,byte[] payload) {
   ReservationId=reservationId;WorldId=worldId;PlayerId=playerId;PluginId=pluginId;Payload=payload;
  }
  public RT_MSG_SERVER_PLUGIN_TO_APP ToScertMessage()=>new RT_MSG_SERVER_PLUGIN_TO_APP{PluginId=PluginId,Payload=Payload};
 }
 public sealed class GameplayReward {
  public string RewardId {get;}
  public string MatchId {get;}
  public string ReservationId {get;}
  public uint WorldId {get;}
  public ushort PlayerId {get;}
  public string AccountId {get;}
  public uint CharacterId {get;}
  public string Kind {get;}
  public uint Units {get;}
  internal GameplayReward(string id,string reservation,string match,uint world,ushort player,string account,uint character,string kind,uint units) {
   RewardId=id;ReservationId=reservation;MatchId=match;WorldId=world;PlayerId=player;AccountId=account;CharacterId=character;Kind=kind;Units=units;
  }
 }
 public enum GameplayBridgeFailure { None, Timeout, Unavailable, InvalidRequest, MalformedResponse }
 public sealed class GameplayMatchTerminal {
  public uint WorldId {get;}
  public string MatchId {get;}
  public string Reason {get;}
  public IReadOnlyList<string> ReservationIds {get;}
  internal GameplayMatchTerminal(uint world,string match,string reason,IReadOnlyList<string> reservations) { WorldId=world;MatchId=match;Reason=reason;ReservationIds=reservations; }
 }
 public sealed class GameplayBridgeResponse {
  public bool Ok {get;}
  public string Error {get;}
  public GameplayBridgeFailure Failure {get;}
  public long ElapsedMilliseconds {get; internal set;}
  public bool IsTransientFailure => Failure == GameplayBridgeFailure.Timeout || Failure == GameplayBridgeFailure.Unavailable;
  public IReadOnlyList<GameplayBridgeMessage> Messages {get;}
  public IReadOnlyList<GameplayReward> Rewards {get;}
  public GameplayMatchTerminal MatchTerminal {get;}
  internal GameplayBridgeResponse(bool ok,string error,IReadOnlyList<GameplayBridgeMessage> messages,GameplayBridgeFailure failure=GameplayBridgeFailure.None,IReadOnlyList<GameplayReward> rewards=null,GameplayMatchTerminal matchTerminal=null){Ok=ok;Error=error;Messages=messages;Failure=failure;Rewards=rewards??Array.Empty<GameplayReward>();MatchTerminal=matchTerminal;}
  internal static GameplayBridgeResponse Failed(GameplayBridgeFailure failure,long elapsed)=>new GameplayBridgeResponse(false,null,Array.Empty<GameplayBridgeMessage>(),failure){ElapsedMilliseconds=elapsed};
 }
 /// <summary>Strict bridge v2 reply codec. Event and release replies may carry bounded, independently addressed reliable messages.</summary>
 public static class GameplayBridgeResponseCodec {
  public const int MaximumMessageBytes=1397;
  public const int MaximumResponseBytes=131072;
  public const int MaximumMessages=128;
  static bool ExactFields(JsonElement value,params string[] expected) {
   if(value.ValueKind!=JsonValueKind.Object)return false;
   var names=new HashSet<string>(StringComparer.Ordinal);
   foreach(var property in value.EnumerateObject())if(!names.Add(property.Name)||!expected.Contains(property.Name))return false;
   return names.Count==expected.Length;
  }
  static bool Identifier(string value)=>value!=null&&value.Length>=16&&value.Length<=64&&value.All(c=>c>='a'&&c<='z'||c>='A'&&c<='Z'||c>='0'&&c<='9'||c=='-'||c=='_');
  public static bool TryParse(byte[] line,string requestId,bool eventReply,out GameplayBridgeResponse response) {
   response=null;if(line==null||line.Length>MaximumResponseBytes||string.IsNullOrEmpty(requestId))return false;
   try {
    using var document=JsonDocument.Parse(line);var root=document.RootElement;
    bool hasRewards=root.TryGetProperty("rewards",out var rewards);
    bool hasTerminal=root.TryGetProperty("match_terminal",out var terminal);
    var fields=new List<string>{"v","request_id","ok","error","messages"};if(hasRewards)fields.Add("rewards");if(hasTerminal)fields.Add("match_terminal");
    if(!ExactFields(root,fields.ToArray())||!root.GetProperty("v").TryGetInt32(out var version)||version!=2||
     root.GetProperty("request_id").ValueKind!=JsonValueKind.String||root.GetProperty("request_id").GetString()!=requestId)return false;
    var ok=root.GetProperty("ok");if(ok.ValueKind!=JsonValueKind.True&&ok.ValueKind!=JsonValueKind.False)return false;
    var error=root.GetProperty("error");if(error.ValueKind!=JsonValueKind.Null&&error.ValueKind!=JsonValueKind.String)return false;
    bool success=ok.GetBoolean();if(success&&error.ValueKind!=JsonValueKind.Null)return false;
    var messages=root.GetProperty("messages");if(messages.ValueKind!=JsonValueKind.Array||messages.GetArrayLength()>MaximumMessages||
     ((!success||!eventReply)&&messages.GetArrayLength()!=0))return false;
    var parsed=new List<GameplayBridgeMessage>();int totalBytes=0;
    foreach(var message in messages.EnumerateArray()) {
     if(!ExactFields(message,"reservation_id","world_id","player_id","plugin_id","payload_hex"))return false;
     var reservation=message.GetProperty("reservation_id");var hex=message.GetProperty("payload_hex");
     if(reservation.ValueKind!=JsonValueKind.String||!Identifier(reservation.GetString())||hex.ValueKind!=JsonValueKind.String||
      !message.GetProperty("world_id").TryGetUInt32(out var world)||!message.GetProperty("player_id").TryGetUInt16(out var player)||
      !message.GetProperty("plugin_id").TryGetByte(out var plugin)||plugin!=1)return false;
     string text=hex.GetString();if(text.Length==0||text.Length>MaximumMessageBytes*2||text.Length%2!=0)return false;
     totalBytes+=text.Length/2;if(totalBytes>24000)return false;
     parsed.Add(new GameplayBridgeMessage(reservation.GetString(),world,player,plugin,Convert.FromHexString(text)));
    }
    var parsedRewards=new List<GameplayReward>();var rewardIds=new HashSet<string>(StringComparer.Ordinal);
    if(hasRewards) {
     if(rewards.ValueKind!=JsonValueKind.Array||rewards.GetArrayLength()>32||(!success&&rewards.GetArrayLength()!=0))return false;
     foreach(var reward in rewards.EnumerateArray()) {
      if(!ExactFields(reward,"reward_id","reservation_id","match_id","world_id","player_id","account_id","character_id","kind","units"))return false;
      string id=reward.GetProperty("reward_id").GetString(),reservation=reward.GetProperty("reservation_id").GetString(),match=reward.GetProperty("match_id").GetString(),account=reward.GetProperty("account_id").GetString(),kind=reward.GetProperty("kind").GetString();
      if(id==null||id.Length<1||id.Length>128||!id.All(c=>char.IsAsciiLetterOrDigit(c)||c=='-'||c=='_')||!rewardIds.Add(id)||!Identifier(reservation)||!Identifier(match)||!MagDevelopmentAccountPolicy.ValidAccount(account)||
       !reward.GetProperty("world_id").TryGetUInt32(out var world)||!reward.GetProperty("player_id").TryGetUInt16(out var player)||!reward.GetProperty("character_id").TryGetUInt32(out var character)||character==0||
       !reward.GetProperty("units").TryGetUInt32(out var units)|| (units>int.MaxValue||!MagProgressionCatalog.ValidKindUnits(kind,(int)units))) return false;
      parsedRewards.Add(new GameplayReward(id,reservation,match,world,player,account,character,kind,units));
     }
    }
    GameplayMatchTerminal parsedTerminal=null;
    if(hasTerminal&&terminal.ValueKind!=JsonValueKind.Null) {
     if(!success||!eventReply||!ExactFields(terminal,"world_id","match_id","reason","reservation_ids")||
      !terminal.GetProperty("world_id").TryGetUInt32(out var world)||terminal.GetProperty("match_id").ValueKind!=JsonValueKind.String||
      !Identifier(terminal.GetProperty("match_id").GetString())||terminal.GetProperty("reason").ValueKind!=JsonValueKind.String)return false;
     var reason=terminal.GetProperty("reason").GetString();if(reason!="attacker_victory"&&reason!="defender_victory")return false;
     var members=terminal.GetProperty("reservation_ids");if(members.ValueKind!=JsonValueKind.Array||members.GetArrayLength()>64)return false;
     var identities=new HashSet<string>(StringComparer.Ordinal);var ordered=new List<string>();
     foreach(var member in members.EnumerateArray()) {if(member.ValueKind!=JsonValueKind.String||!Identifier(member.GetString())||!identities.Add(member.GetString()))return false;ordered.Add(member.GetString());}
     parsedTerminal=new GameplayMatchTerminal(world,terminal.GetProperty("match_id").GetString(),reason,ordered.AsReadOnly());
    }
    response=new GameplayBridgeResponse(success,error.ValueKind==JsonValueKind.String?error.GetString():null,parsed.AsReadOnly(),rewards:parsedRewards.AsReadOnly(),matchTerminal:parsedTerminal);return true;
   }catch(Exception e)when(e is JsonException||e is InvalidOperationException||e is FormatException||e is ArgumentException){return false;}
  }
 }
}
