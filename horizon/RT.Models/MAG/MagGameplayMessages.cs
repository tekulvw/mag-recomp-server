using System;
namespace RT.Models.MAG {
 public sealed class MagGamePluginGreeting : MagMessage { public override ushort MessageType => 0x0010; }
 /// <summary>Retail NetMessageTypePluginId (original mage_g.elf 0x15B360): retail inserted KeepAlive at 0x0003, so the prototype 0x0010 greeting is 0x0011.</summary>
 public sealed class MagRetailGamePluginGreeting : MagMessage { public override ushort MessageType => 0x0011; }
 public class MagJoinGameRequest : MagPluginClientMessage {
  public override ushort MessageType => 0x0011;
  public byte Preference {get;set;} public uint Value {get;set;} public byte ClientType {get;set;}
 }
 public class MagJoinGameResponse : MagPluginClientMessage {
  public override ushort MessageType => 0x0012;
  public byte Result {get;set;} public uint MediusWorldUid {get;set;}
  public ushort ConnectionType {get;set;} public ushort AddressType {get;set;}
  public byte[] ServerAddress {get;set;}=new byte[16]; public ushort ServerPort {get;set;}
  public uint WorldId {get;set;} public byte[] AccessKey {get;set;}=new byte[17]; public uint Piuid {get;set;}
 }
 public sealed class MagServerStatusRequest : MagPluginClientMessage { public override ushort MessageType => 0x024b; }
 public sealed class MagServerStatusResponse : MagPluginClientMessage {
  public override ushort MessageType => 0x024c;
  public uint[] PlayersInGame {get;set;}=new uint[12]; public uint[] ShadowWarScores {get;set;}=new uint[12];
  public byte[] ContractOwners {get;set;}=new byte[9]; public uint[] QueueSizes {get;set;}=new uint[12];
  public uint[] MinimumPlayers {get;set;}=new uint[4]; public bool[] QueueEnabled {get;set;}=new bool[12];
 }
}

namespace RT.Models.MAG {
 // Retail001E8C2C/0012D4B8: two preference words, client type and trailing party ID.
 public sealed class MagRetailJoinGameRequest : MagJoinGameRequest {
  public override ushort MessageType => 0x0012;
  // Value inherited from prototype is preference payload word0. Word1 semantics remain unproven.
  public uint Value1 {get;set;} public uint PartyId {get;set;}
 }
 public sealed class MagRetailJoinGameResponse : MagJoinGameResponse { public override ushort MessageType => 0x0013; }
}
