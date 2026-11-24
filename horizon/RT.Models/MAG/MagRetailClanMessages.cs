using System;
using System.Buffers.Binary;

namespace RT.Models.MAG
{
    /// <summary>
    /// Retail clan traffic over the MLS plugin. Both directions use MAG message type 0x01FC: the
    /// big-endian MAG type is followed by a clan body whose first byte is the clan message type.
    /// Clan body fields are little-endian. Only the clan list request and its empty answer are known.
    /// </summary>
    public static class MagRetailClanCodec
    {
        public const ushort MessageType = 0x01FC;
        public const byte GetClanListType = 0x02;
        // Clan status meaning "no results"; the client counts it as success.
        public const uint StatusNoResults = 10;
        private const int GetClanListBodyBytes = 9;

        /// <summary>True when the payload is a retail clan message, giving its clan type byte.</summary>
        public static bool TryReadClanType(byte[] payload, int applicationId, out byte clanType)
        {
            clanType = 0;
            if (applicationId != 20231 || payload == null || payload.Length < 3 ||
                BinaryPrimitives.ReadUInt16BigEndian(payload) != MessageType) return false;
            clanType = payload[2];
            return true;
        }

        /// <summary>Clan list request: type 2, request id and account id; nothing more or less.</summary>
        public static bool TryParseGetClanListRequest(byte[] payload, int applicationId, out uint requestId, out uint accountId)
        {
            requestId = 0; accountId = 0;
            if (!TryReadClanType(payload, applicationId, out var clanType) || clanType != GetClanListType ||
                payload.Length != 2 + GetClanListBodyBytes) return false;
            requestId = BinaryPrimitives.ReadUInt32LittleEndian(payload.AsSpan(3));
            accountId = BinaryPrimitives.ReadUInt32LittleEndian(payload.AsSpan(7));
            return true;
        }

        /// <summary>
        /// The clan list answer for an account in no clan: type 2, the echoed request id, status
        /// "no results" and the last-packet flag. With that status no clan entry follows.
        /// </summary>
        public static byte[] SerializeNoClansResponse(uint requestId)
        {
            var payload = new byte[2 + 10];
            BinaryPrimitives.WriteUInt16BigEndian(payload, MessageType);
            payload[2] = GetClanListType;
            BinaryPrimitives.WriteUInt32LittleEndian(payload.AsSpan(3), requestId);
            BinaryPrimitives.WriteUInt32LittleEndian(payload.AsSpan(7), StatusNoResults);
            payload[11] = 1;
            return payload;
        }
    }
}
