using System.Linq;
namespace RT.Models.MAG
{
    public abstract class MagMessage
    {
        public abstract ushort MessageType { get; }
    }

    public sealed class MagHello : MagMessage
    {
        public override ushort MessageType => 0x0001;
    }

    // The same short message type is sent by the MAS service and acknowledged by the client.
    public sealed class MagProtocolInfo : MagMessage
    {
        public override ushort MessageType => 0x0002;
        public uint ProtocolVersion { get; set; }
        public uint BuildNumber { get; set; }
    }

    public abstract class MagPluginClientMessage : MagMessage
    {
        public uint[] RsaWords { get; set; } = new uint[16];
        public int TransactionId { get; set; }
    }

    public class MagNpAuthenticateRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0031;
        public ushort FullTicketSize { get; set; }
        public byte[] TicketChunk { get; set; } = new byte[900];
        public ushort TicketChunkSize { get; set; }
        public bool LastMessage { get; set; }
    }

    public class MagNpAuthenticateResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0032;
        public bool Success { get; set; }
    }
}

namespace RT.Models.MAG
{
    public class MagMapsHelloResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0033;
        public bool Success { get; set; }
        public bool Online { get; set; }
        public uint FactionMask { get; set; }
    }

    public class MagUniverseListRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0034;
    }

    public class MagUniverseListResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0035;
        public uint UniverseId { get; set; }
        public string Name { get; set; } = string.Empty;
        public string AuthDns { get; set; } = string.Empty;
        public string AuthIp { get; set; } = string.Empty;
        public uint AuthPort { get; set; }
        public string SvoUrl { get; set; } = string.Empty;
        public bool Success { get; set; }
        public bool IsLast { get; set; }
    }

    public sealed class MagNewsEulaRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x023e;
        public string Language { get; set; } = string.Empty;
    }

    public enum MagContentType : byte { News = 0, Eula = 1 }

    public sealed class MagNewsEulaResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x023f;
        public bool Finished { get; set; }
        public MagContentType ContentType { get; set; }
        public string Content { get; set; } = string.Empty;
        public uint Timestamp { get; set; }
    }

    public sealed class MagCharacterListRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0236;
    }

    public class MagCharacterListResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0237;
        public bool Success { get; set; }
        public byte AllowedCharacters { get; set; }
        public byte CharacterCount { get; set; }
        public System.Collections.Generic.List<MagCharacterSummary> Characters { get; set; } = new System.Collections.Generic.List<MagCharacterSummary>();
    }
    // Retail app20231 appends VeteranLevel to each summary; the list header is shared.
    public sealed class MagRetailCharacterListResponse : MagCharacterListResponse { }
}

namespace RT.Models.MAG
{
    public enum MagNetResult : int { Success = 0, Error = 1, NameTaken = 6, NameInvalid = 7 }
    public sealed class MagCreateCharacterRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x023a;
        public string AccountName { get; set; } = string.Empty;
        public byte Faction { get; set; }
        public byte HeadId { get; set; }
        public byte VoiceId { get; set; }
        public uint UniverseId { get; set; }
        public byte[] NpId { get; set; } = new byte[36];
    }
    public sealed class MagCreateCharacterResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x023b;
        public MagNetResult Result { get; set; }
    }
    public sealed class MagLoadout
    {
        public byte UpdateId { get; set; } = 1;
        public short[] Items { get; set; } = System.Linq.Enumerable.Repeat((short)-1, 22).ToArray();
    }
    public class MagCharacterSummary
    {
        public string Name { get; set; } = string.Empty;
        public uint AccountId { get; set; }
        public byte Faction { get; set; }
        public byte SkillSpecialty { get; set; } = 4;
        public byte Level { get; set; } = 1;
        public MagLoadout Loadout { get; set; } = new MagLoadout();
        public uint UniverseId { get; set; }
        public byte FactionHistory { get; set; }
        public bool RespecAvailable { get; set; }
        public byte HeadId { get; set; }
        public byte VoiceId { get; set; }
    }
    public sealed class MagRetailCharacterSummary : MagCharacterSummary
    {
        // Native retail Clear008BD688 initializes this field to zero.
        public byte VeteranLevel { get; set; }
        public static MagRetailCharacterSummary FromSummary(MagCharacterSummary summary) => new MagRetailCharacterSummary
        {
            Name = summary.Name, AccountId = summary.AccountId, Faction = summary.Faction,
            SkillSpecialty = summary.SkillSpecialty, Level = summary.Level, Loadout = summary.Loadout,
            UniverseId = summary.UniverseId, FactionHistory = summary.FactionHistory,
            RespecAvailable = summary.RespecAvailable, HeadId = summary.HeadId, VoiceId = summary.VoiceId
        };
    }
}

namespace RT.Models.MAG
{
    public class MagAccountLoginRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x002b;
        public int VersionMajor { get; set; }
        public int VersionMinor { get; set; }
        public int VersionBuild { get; set; }
        public string AccountName { get; set; } = string.Empty;
        public int CharacterEncoding { get; set; }
        public int Language { get; set; }
        public int DnasSignatureType { get; set; }
        public byte DnasSignatureLength { get; set; }
        public string DnasSignature { get; set; } = string.Empty;
    }
    public class MagAccountLoginResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x002c;
        public uint LobbyWorldUid { get; set; }
        public uint AccountId { get; set; }
        public ushort ConnectionType { get; set; }
        public ushort AddressType { get; set; }
        public byte[] ServerAddress { get; set; } = new byte[16];
        public ushort ServerPort { get; set; }
        public uint WorldId { get; set; }
        public byte[] AccessKey { get; set; } = new byte[17];
        public byte[] SessionKey { get; set; } = new byte[17];
        public byte Result { get; set; }
        public uint Piuid { get; set; }
        public uint QueuePosition { get; set; }
        public uint QueueTimeLeft { get; set; }
        public uint TimestampWord0 { get; set; }
        public uint TimestampWord1 { get; set; }
    }
}

namespace RT.Models.MAG
{
    public sealed class MagCharacterDataRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0238;
        public byte QueryKind { get; set; }
        public string AccountName { get; set; } = string.Empty;
        public uint AccountId { get; set; }
        public byte RequestId { get; set; }
    }
}

namespace RT.Models.MAG
{
    public sealed class MagCharacterDataResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0239;
        public bool Success { get; set; }
        public byte RequestId { get; set; }
        public MagCharacterRecord Record { get; set; } = new MagCharacterRecord();
    }
    // Full record is distinct from the shorter character-list summary. Defaults follow native Clear;
    // counters not initialized there are explicitly zero for this local development profile.
    public sealed class MagCharacterRecord
    {
        public string Name { get; set; } = string.Empty;
        public uint AccountId { get; set; }
        public byte Faction { get; set; } = 255;
        public byte SkillSpecialty { get; set; } = 4;
        public byte Level { get; set; } = 1;
        public uint Experience { get; set; }
        public uint[] AvailableItems { get; set; } = new uint[24];
        public uint[] Skills { get; set; } = new uint[5];
        public byte SkillPoints { get; set; }
        public MagLoadout[] Loadouts { get; set; } = Enumerable.Range(0, 5).Select(_ => new MagLoadout()).ToArray();
        public byte LeadershipLevel { get; set; } = 1;
        public uint LeadershipPoints { get; set; }
        public uint TimePlayed { get; set; }
        public uint LastZonesMux { get; set; } = 65535;
        public byte HeadId { get; set; }
        public byte VoiceId { get; set; }
        public byte LeadershipNominations { get; set; }
        public byte MatchesSinceLastCommand { get; set; }
        public byte RespecCount { get; set; }
        public uint RespecPoints { get; set; }
        public uint UniverseId { get; set; }
        public byte FactionHistory { get; set; }
        public bool FactionRespec { get; set; }
        public byte LastUsedLoadout { get; set; }
        public byte[] NpId { get; set; } = new byte[36];
        // Original retail008BDA40/008BE014 tails; Clear008BDE34 zeros both.
        public byte VeteranLevel { get; set; }
        public byte Flags { get; set; }
    }
}

namespace RT.Models.MAG
{
    // NetMessageCharacterRecord: unlike0239 there is no request header or success bit.
    public sealed class MagCharacterRecordNotification : MagMessage
    {
        public override ushort MessageType => 0x024f;
        public MagCharacterRecord Record { get; set; } = new MagCharacterRecord();
    }
    public sealed class MagKeepAlive : MagMessage
    {
        public override ushort MessageType => 0x0003;
    }
}

namespace RT.Models.MAG
{
    // Original retail001C911C/001E169C: login IDs differ, ordered fields are shared.
    public sealed class MagRetailAccountLoginRequest : MagAccountLoginRequest { public override ushort MessageType => 0x002e; }
    public sealed class MagRetailAccountLoginResponse : MagAccountLoginResponse { public override ushort MessageType => 0x002f; }
}
