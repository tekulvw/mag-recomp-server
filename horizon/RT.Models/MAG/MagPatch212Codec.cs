using System;
using System.Collections.Generic;
using System.IO;

namespace RT.Models.MAG
{
    public sealed class MagPatch212Request : MagMessage
    {
        public override ushort MessageType => 0x01f5;
        public uint[] InstalledLevels { get; internal set; } = Array.Empty<uint>();
        public string BuildString { get; internal set; } = string.Empty;
        public string Language { get; internal set; } = string.Empty;
    }

    /// <summary>
    /// Explicit 2.12 launcher patch format. This offline codec is not a session
    /// version detector and does not enable 1793 login or gameplay routing.
    /// </summary>
    public static class MagPatch212Codec
    {
        private const int MaximumPayloadBytes = 10237;

        public static bool TryParseRequest(byte[] payload, out MagPatch212Request request)
        {
            request = null;
            if (payload == null || payload.Length < 8 || payload.Length > MaximumPayloadBytes) return false;
            try
            {
                var reader = new MagBitReader(payload);
                if (reader.ReadUInt16() != 0x01f5) return false;
                int count = reader.ReadUInt16();
                // Reserve both string-length fields before allocating the vector.
                if (count > (payload.Length - 8) / 4) return false;
                var levels = new uint[count];
                for (int i = 0; i < count; i++) levels[i] = reader.ReadUInt32();
                string build = reader.ReadString();
                string language = reader.ReadString();
                if (!reader.HasOnlyZeroPadding()) return false;
                request = new MagPatch212Request { InstalledLevels = levels, BuildString = build, Language = language };
                return true;
            }
            catch (InvalidDataException) { return false; }
        }

        /// <summary>
        /// Settings followed by the empty-name terminal PatchURL. Call only
        /// after session policy establishes that all required content exists.
        /// Settings retain the existing local no-download policy values.
        /// </summary>
        public static IReadOnlyList<byte[]> NoDownloadReply()
        {
            var settings = new MagBitWriter();
            settings.WriteUInt16(0x01f6);
            for (int i = 0; i < 3; i++) settings.WriteUInt32(uint.MaxValue);
            settings.WriteUInt32(8192);
            settings.WriteByte(0);
            for (int i = 0; i < 6; i++) settings.WriteUInt32(uint.MaxValue);

            var terminal = new MagBitWriter();
            terminal.WriteUInt16(0x01f7);
            terminal.WriteString(string.Empty);
            terminal.WriteByte(0);
            terminal.WriteUInt32(0);
            terminal.WriteBoolean(true);
            return new[] { settings.ToArray(), terminal.ToArray() };
        }
    }
}
