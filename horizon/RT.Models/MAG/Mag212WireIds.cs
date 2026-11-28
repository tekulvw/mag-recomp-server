using System;

namespace RT.Models.MAG
{
    // Selected constructor immediates verified against the exact 2.12 engine
    // SHA256705a6ec6a25458b2401b869e75e2904597304e2c06d9534a171a1dcd54aca2b3.
    // Internal IDs name existing models; unsupported 1793 messages never fall back.
    internal static class Mag212WireIds
    {
        private static readonly (ushort Internal, ushort Wire)[] Selected = {
            (0x0001,0x0001), (0x0002,0x0002), (0x0003,0x0003), (0x002d,0x002d),
            (0x002e,0x0030), (0x002f,0x0031), (0x0030,0x0032), (0x0031,0x0033),
            (0x0034,0x0036), (0x0035,0x0037), (0x0036,0x0038), (0x0037,0x0039), (0x0038,0x003a),
            (0x01f5,0x01f5),
            (0x0236,0x024a), (0x0237,0x024b), (0x0238,0x024c), (0x0239,0x024d),
            (0x023a,0x024e), (0x023b,0x024f), (0x023c,0x0250), (0x023d,0x0251),
            (0x023e,0x0252), (0x023f,0x0253), (0x024f,0x0267),
            (0x0250,0x0268), (0x0251,0x0269)
        };

        public static bool TryDecode(ushort wire, out ushort internalId)
        {
            foreach (var row in Selected)
                if (row.Wire == wire) { internalId = row.Internal; return true; }
            internalId = 0;
            return false;
        }

        public static ushort Encode(ushort internalId)
        {
            foreach (var row in Selected) if (row.Internal == internalId) return row.Wire;
            throw new ArgumentException("This MAG message has no implemented 1793 layout.");
        }
    }
}
