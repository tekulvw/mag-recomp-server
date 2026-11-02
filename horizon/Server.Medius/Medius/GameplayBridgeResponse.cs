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
 public sealed class GameplayLoadoutSelection {
  public string SelectionId {get;}
  public string ReservationId {get;}
  public uint WorldId {get;}
  public ushort PlayerId {get;}
  public string AccountId {get;}
  public uint CharacterId {get;}
  public ulong Revision {get;}
  public byte InventoryIndex {get;}
  internal GameplayLoadoutSelection(string id,string reservation,uint world,ushort player,string account,uint character,ulong revision,byte index) {
   SelectionId=id;ReservationId=reservation;WorldId=world;PlayerId=player;AccountId=account;CharacterId=character;Revision=revision;InventoryIndex=index;
  }
  public bool MatchesOwner(string reservation,uint world,ushort player,string account,uint character)=>
   string.Equals(ReservationId,reservation,StringComparison.Ordinal)&&WorldId==world&&PlayerId==player&&
   string.Equals(AccountId,account,StringComparison.Ordinal)&&CharacterId==character;
  internal bool Same(GameplayLoadoutSelection other)=>other!=null&&SelectionId==other.SelectionId&&ReservationId==other.ReservationId&&WorldId==other.WorldId&&PlayerId==other.PlayerId&&AccountId==other.AccountId&&CharacterId==other.CharacterId&&Revision==other.Revision&&InventoryIndex==other.InventoryIndex;
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
  public IReadOnlyList<GameplayLoadoutSelection> LoadoutSelections {get;}
  public IReadOnlyList<string> Capabilities {get;}
  public IReadOnlyList<MagMatchProfile> MatchProfiles {get;}
  public GameplaySelectedMatch SelectedMatch {get;}
  public bool SupportsActiveLoadoutRefresh => Capabilities.Contains("active_loadout_refresh_v1",StringComparer.Ordinal);
  public bool SupportsCommittedLoadoutSelections => Capabilities.Contains("committed_loadout_selection_v1",StringComparer.Ordinal);
  internal GameplayBridgeResponse(bool ok,string error,IReadOnlyList<GameplayBridgeMessage> messages,GameplayBridgeFailure failure=GameplayBridgeFailure.None,IReadOnlyList<GameplayReward> rewards=null,GameplayMatchTerminal matchTerminal=null,IReadOnlyList<GameplayLoadoutSelection> loadoutSelections=null,IReadOnlyList<string> capabilities=null,IReadOnlyList<MagMatchProfile> matchProfiles=null,GameplaySelectedMatch selectedMatch=null){MatchProfiles=matchProfiles??Array.Empty<MagMatchProfile>();SelectedMatch=selectedMatch;LoadoutSelections=loadoutSelections??Array.Empty<GameplayLoadoutSelection>();Capabilities=capabilities??Array.Empty<string>();Ok=ok;Error=error;Messages=messages;Failure=failure;Rewards=rewards??Array.Empty<GameplayReward>();MatchTerminal=matchTerminal;}
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
    bool hasSelections=root.TryGetProperty("loadout_selections",out var selections);
    bool hasCapabilities=root.TryGetProperty("capabilities",out var capabilities);
    bool hasProfiles=root.TryGetProperty("match_profiles",out var profiles);
    bool hasSelectedMatch=root.TryGetProperty("selected_match",out var selectedMatch);
    var fields=new List<string>{"v","request_id","ok","error","messages"};if(hasProfiles)fields.Add("match_profiles");if(hasSelectedMatch)fields.Add("selected_match");if(hasSelections)fields.Add("loadout_selections");if(hasCapabilities)fields.Add("capabilities");if(hasRewards)fields.Add("rewards");if(hasTerminal)fields.Add("match_terminal");
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
    var parsedCapabilities=new List<string>();var capabilityNames=new HashSet<string>(StringComparer.Ordinal);
    if(hasCapabilities) {
     if(capabilities.ValueKind!=JsonValueKind.Array||capabilities.GetArrayLength()>16)return false;
     foreach(var capability in capabilities.EnumerateArray()) {
      if(capability.ValueKind!=JsonValueKind.String)return false;
      var name=capability.GetString();if(name.Length==0||name.Length>64||!name.All(c=>char.IsAsciiLetterOrDigit(c)||c=='_'||c=='-')||!capabilityNames.Add(name))return false;
      parsedCapabilities.Add(name);
     }
    }
    var parsedSelections=new List<GameplayLoadoutSelection>();var selectionIds=new HashSet<string>(StringComparer.Ordinal);var selectionOwners=new HashSet<string>(StringComparer.Ordinal);
    if(hasSelections) {
     if(selections.ValueKind!=JsonValueKind.Array||selections.GetArrayLength()>32||(!success&&selections.GetArrayLength()!=0))return false;
     foreach(var selection in selections.EnumerateArray()) {
      if(!ExactFields(selection,"selection_id","reservation_id","world_id","player_id","account_id","character_id","revision","inventory_index"))return false;
      string id=selection.GetProperty("selection_id").GetString(),reservation=selection.GetProperty("reservation_id").GetString(),account=selection.GetProperty("account_id").GetString();
      if(id==null||id.Length==0||id.Length>128||!id.All(c=>char.IsAsciiLetterOrDigit(c)||c=='-'||c=='_')||!selectionIds.Add(id)||!Identifier(reservation)||!selectionOwners.Add(reservation)||!MagDevelopmentAccountPolicy.ValidAccount(account)||
       !selection.GetProperty("world_id").TryGetUInt32(out var world)||!selection.GetProperty("player_id").TryGetUInt16(out var player)||!selection.GetProperty("character_id").TryGetUInt32(out var character)||character==0||
       !selection.GetProperty("revision").TryGetUInt64(out var revision)||revision==0||!selection.GetProperty("inventory_index").TryGetByte(out var index)||index>=5)return false;
      parsedSelections.Add(new GameplayLoadoutSelection(id,reservation,world,player,account,character,revision,index));
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
    IReadOnlyList<MagMatchProfile> parsedProfiles=Array.Empty<MagMatchProfile>();GameplaySelectedMatch parsedMatch=null;
    if(hasProfiles&&!GameplayMatchProfileCodec.Profiles(profiles,success,out parsedProfiles))return false;
    if(hasSelectedMatch&&!GameplayMatchProfileCodec.Selection(selectedMatch,success,out parsedMatch))return false;
    response=new GameplayBridgeResponse(success,error.ValueKind==JsonValueKind.String?error.GetString():null,parsed.AsReadOnly(),rewards:parsedRewards.AsReadOnly(),matchTerminal:parsedTerminal,loadoutSelections:parsedSelections.AsReadOnly(),capabilities:parsedCapabilities.AsReadOnly(),matchProfiles:parsedProfiles,selectedMatch:parsedMatch);return true;
   }catch(Exception e)when(e is JsonException||e is InvalidOperationException||e is FormatException||e is ArgumentException){return false;}
  }
 }
}
