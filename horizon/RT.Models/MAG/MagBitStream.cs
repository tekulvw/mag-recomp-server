using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

namespace RT.Models.MAG
{
    // Client bit operators are MSB-first and do not align after booleans.
    // Only bounded ASCII text is supported until another wire encoding is established.
    internal sealed class MagBitReader
    {
        private readonly byte[] _bytes;
        private int _position;
        public MagBitReader(byte[] bytes) => _bytes = bytes;
        public uint ReadBits(int count)
        {
            if (count < 1 || count > 32 || _bytes.Length * 8 - _position < count)
                throw new InvalidDataException("Truncated MAG field.");
            uint value = 0;
            for (int i = 0; i < count; i++, _position++)
                value = (value << 1) | (uint)((_bytes[_position / 8] >> (7 - _position % 8)) & 1);
            return value;
        }
        public bool ReadBoolean() => ReadBits(1) != 0;
        public byte ReadByte() => (byte)ReadBits(8);
        public ushort ReadUInt16() => (ushort)ReadBits(16);
        public uint ReadUInt32() => ReadBits(32);
        public byte[] ReadBytes(int count)
        {
            if (count < 0 || count > (_bytes.Length * 8 - _position) / 8)
                throw new InvalidDataException("Truncated MAG byte field.");
            var result = new byte[count];
            for (int i = 0; i < count; i++) result[i] = ReadByte();
            return result;
        }
        public string ReadString()
        {
            int size = ReadUInt16();
            if (size > MagMessageCodec.MaximumStringBytes) throw new InvalidDataException("MAG string exceeds local bound.");
            var bytes = ReadBytes(size);
            foreach (byte value in bytes) if (value > 0x7f) throw new InvalidDataException("Non-ASCII MAG string unsupported.");
            return Encoding.ASCII.GetString(bytes);
        }
        public string ReadCString()
        {
            int size = ReadByte();
            if (size > 31) throw new InvalidDataException("MAG name exceeds CString capacity.");
            var bytes = ReadBytes(size);
            foreach (byte value in bytes) if (value == 0 || value > 0x7f) throw new InvalidDataException("Unsupported MAG name encoding.");
            return Encoding.ASCII.GetString(bytes);
        }
        public bool HasOnlyZeroPadding()
        {
            int remaining = _bytes.Length * 8 - _position;
            return remaining < 8 && (remaining == 0 || ReadBits(remaining) == 0);
        }
    }

    internal sealed class MagBitWriter
    {
        private readonly List<byte> _bytes = new List<byte>();
        private int _position;
        public void WriteBits(uint value, int count)
        {
            if (count < 1 || count > 32 || (count < 32 && value >= (1u << count)))
                throw new ArgumentException("Value does not fit MAG bit field.");
            for (int bit = count - 1; bit >= 0; bit--, _position++)
            {
                if (_position % 8 == 0) _bytes.Add(0);
                _bytes[_bytes.Count - 1] |= (byte)(((value >> bit) & 1) << (7 - _position % 8));
            }
        }
        public void WriteBoolean(bool value) => WriteBits(value ? 1u : 0u, 1);
        public void WriteByte(byte value) => WriteBits(value, 8);
        public void WriteUInt16(ushort value) => WriteBits(value, 16);
        public void WriteUInt32(uint value) => WriteBits(value, 32);
        public void WriteBytes(byte[] bytes)
        {
            foreach (byte value in bytes) WriteByte(value);
        }
        public void WriteString(string value)
        {
            if (value == null || value.Length > MagMessageCodec.MaximumStringBytes)
                throw new ArgumentException("MAG string exceeds local bound.");
            foreach (char character in value) if (character > 0x7f) throw new ArgumentException("Non-ASCII MAG string unsupported.");
            WriteUInt16((ushort)value.Length);
            WriteBytes(Encoding.ASCII.GetBytes(value));
        }
        public void WriteCString(string value)
        {
            if (value == null || value.Length > 31) throw new ArgumentException("MAG name exceeds CString capacity.");
            foreach (char c in value) if (c == 0 || c > 0x7f) throw new ArgumentException("Unsupported MAG name encoding.");
            WriteByte((byte)value.Length); WriteBytes(Encoding.ASCII.GetBytes(value));
        }
        public byte[] ToArray() => _bytes.ToArray();
    }
}
