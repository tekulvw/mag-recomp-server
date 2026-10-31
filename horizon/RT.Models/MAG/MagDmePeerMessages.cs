using System;
using System.IO;

namespace RT.Models.MAG
{
    // DME plugin1 has no 70-byte Medius header. Message numbers below are retail1740.
    // Source: gameplay/src/protocol.rs; docs/protocol/retail-join/DME-MESSAGE-IDS.md.
    public abstract class MagDmePeerMessage { public abstract ushort MessageType { get; } }
    public sealed class MagDmeClaimReservation : MagDmePeerMessage
    {
        public override ushort MessageType => 4;
        public byte[] NpIdentity { get; set; } = new byte[36];
        public string Clan { get; set; } = string.Empty;
        public uint LocalIp { get; set; }
        public string PublicKey { get; set; } = string.Empty;
        public string PrivateKey { get; set; } = string.Empty;
        public string SymmetricKey { get; set; } = string.Empty;
    }
    public sealed class MagDmeReady : MagDmePeerMessage { public override ushort MessageType => 6; public bool Ready { get; set; } }
    public sealed class MagDmeEnter : MagDmePeerMessage { public override ushort MessageType => 7; }
    public sealed class MagDmeTimeRequest : MagDmePeerMessage { public override ushort MessageType => 11; public uint ClientTimeMs { get; set; } }
    public sealed class MagDmeTimeResponse : MagDmePeerMessage { public override ushort MessageType => 12; public uint ServerTimeMs { get; set; } public uint ClientTimeMs { get; set; } }
    public sealed class MagDmeGameInfo : MagDmePeerMessage
    {
        public override ushort MessageType => 5;
        public ushort ClientIndex { get; set; }
        public string LevelName { get; set; } = string.Empty;
        public uint StartTimeMs { get; set; }
        public byte AttackingFaction { get; set; }
        public byte DefendingFaction { get; set; }
        public short[] MaximumPoints { get; set; } = new short[2];
        public short[] CurrentPoints { get; set; } = new short[2];
        public sbyte GameType { get; set; }
    }
    public static class MagDmePeerCodec
    {
        public static byte[] Serialize(MagDmePeerMessage message)
        {
            var writer = new MagBitWriter(); writer.WriteUInt16(message.MessageType);
            switch (message)
            {
                case MagDmeClaimReservation claim:
                    if (claim.NpIdentity == null || claim.NpIdentity.Length != 36) throw new ArgumentException("Claim NP identity must be 36 bytes.");
                    writer.WriteBytes(claim.NpIdentity); writer.WriteString(claim.Clan); writer.WriteUInt32(claim.LocalIp);
                    writer.WriteString(claim.PublicKey); writer.WriteString(claim.PrivateKey); writer.WriteString(claim.SymmetricKey); break;
                case MagDmeReady ready: writer.WriteBoolean(ready.Ready); break;
                case MagDmeEnter _: break;
                case MagDmeTimeRequest time: writer.WriteUInt32(time.ClientTimeMs); break;
                case MagDmeTimeResponse time: writer.WriteUInt32(time.ServerTimeMs); writer.WriteUInt32(time.ClientTimeMs); break;
                case MagDmeGameInfo info:
                    writer.WriteUInt16(info.ClientIndex); writer.WriteString(info.LevelName); writer.WriteUInt32(info.StartTimeMs);
                    writer.WriteByte(info.AttackingFaction); writer.WriteByte(info.DefendingFaction);
                    if (info.MaximumPoints.Length != 2 || info.CurrentPoints.Length != 2) throw new ArgumentException("GameInfo score width.");
                    foreach (var point in info.MaximumPoints) writer.WriteUInt16(unchecked((ushort)point));
                    foreach (var point in info.CurrentPoints) writer.WriteUInt16(unchecked((ushort)point));
                    writer.WriteByte(unchecked((byte)info.GameType)); break;
                default: throw new ArgumentException("Unsupported typed DME peer message.");
            }
            return writer.ToArray();
        }
        public static bool TryParse(byte[] payload, out MagDmePeerMessage message)
        {
            message = null;
            if (payload == null || payload.Length < 2 || payload.Length > 10237) return false;
            try
            {
                var reader = new MagBitReader(payload);
                switch (reader.ReadUInt16())
                {
                    case 4: message = new MagDmeClaimReservation { NpIdentity = reader.ReadBytes(36), Clan = reader.ReadString(), LocalIp = reader.ReadUInt32(), PublicKey = reader.ReadString(), PrivateKey = reader.ReadString(), SymmetricKey = reader.ReadString() }; break;
                    case 6: message = new MagDmeReady { Ready = reader.ReadBoolean() }; break;
                    case 7: message = new MagDmeEnter(); break;
                    case 11: message = new MagDmeTimeRequest { ClientTimeMs = reader.ReadUInt32() }; break;
                    case 12: message = new MagDmeTimeResponse { ServerTimeMs = reader.ReadUInt32(), ClientTimeMs = reader.ReadUInt32() }; break;
                    case 5:
                        var info = new MagDmeGameInfo { ClientIndex = reader.ReadUInt16(), LevelName = reader.ReadString(), StartTimeMs = reader.ReadUInt32(), AttackingFaction = reader.ReadByte(), DefendingFaction = reader.ReadByte() };
                        for (int i = 0; i < 2; i++) info.MaximumPoints[i] = unchecked((short)reader.ReadUInt16());
                        for (int i = 0; i < 2; i++) info.CurrentPoints[i] = unchecked((short)reader.ReadUInt16());
                        info.GameType = unchecked((sbyte)reader.ReadByte()); message = info; break;
                    default: return false;
                }
                if (reader.HasOnlyZeroPadding()) return true;
            }
            catch (InvalidDataException) { }
            message = null; return false;
        }
    }
}
