using System;

namespace RT.Models.MAG
{
    // These bodies follow the application type directly.
    public abstract class MagRetailPartyMessage : MagMessage { }

    public sealed class MagRetailPartyInvite : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x001a;
        public uint PartyId { get; set; }
        public uint Invitee { get; set; }
        public uint Inviter { get; set; }
    }

    public sealed class MagRetailPartyInviteResponse : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x001b;
        public byte Response { get; set; }
        public uint PartyId { get; set; }
        public uint Responder { get; set; }
        public uint Inviter { get; set; }
    }

    public sealed class MagRetailPartyLeave : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x001c;
        public uint PartyId { get; set; }
    }

    public sealed class MagRetailPartyJoined : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x001f;
        public uint PartyId { get; set; }
    }

    public sealed class MagRetailPartyLeft : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x0020;
        public uint PartyId { get; set; }
    }

    public sealed class MagRetailPartyRoster : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x0021;
        public uint[] Members { get; set; } = Array.Empty<uint>();
        public uint PartyId { get; set; }
        public sbyte Status { get; set; }
        public sbyte GameType { get; set; }
        public bool OpenInvite { get; set; }
    }
}
