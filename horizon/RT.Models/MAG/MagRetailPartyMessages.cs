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

    /// <summary>Invite answer. Wire order: response, inviter, responder (the invitee itself), party id.</summary>
    public sealed class MagRetailPartyInviteResponse : MagRetailPartyMessage
    {
        public const byte Accept = 0, Decline = 1, Error = 2, InParty = 3, Timeout = 4;
        public override ushort MessageType => 0x001b;
        public byte Response { get; set; }
        public uint Inviter { get; set; }
        public uint Responder { get; set; }
        public uint PartyId { get; set; }
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

    /// <summary>Tells an inviter that an invitation was not taken up. Reason uses the invite response codes.</summary>
    public sealed class MagRetailPartyDecline : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x0022;
        public uint Declined { get; set; }
        public uint PartyId { get; set; }
        public byte Reason { get; set; }
    }

    /// <summary>Client request to change the party's open-invite flag. The flag is a single bit, so the id is unaligned.</summary>
    public sealed class MagRetailPartyOpenInvite : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x0025;
        public bool OpenInvite { get; set; }
        public uint PartyId { get; set; }
    }

    /// <summary>Client request for the party voice world, sent after its game connection drops.</summary>
    public sealed class MagRetailPartyRejoinRequest : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x0026;
        public uint PartyId { get; set; }
    }

    /// <summary>Party voice world descriptor. Codec support only: this server hosts no party voice world.</summary>
    public sealed class MagRetailPartyRejoinResponse : MagRetailPartyMessage
    {
        public override ushort MessageType => 0x0027;
        public uint PartyId { get; set; }
        public uint MediusWorldUid { get; set; }
        public ushort ConnectionType { get; set; }
        public ushort AddressType { get; set; }
        public byte[] ServerAddress { get; set; } = new byte[16];
        public ushort ServerPort { get; set; }
        public uint WorldId { get; set; }
        public byte[] AccessKey { get; set; } = new byte[17];
        public uint Piuid { get; set; }
    }
}
