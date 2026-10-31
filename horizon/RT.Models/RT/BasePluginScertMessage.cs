using System;
using Server.Common.Stream;

namespace RT.Models
{
    /// <summary>MAG SDK plugin envelope: u16 LE app length, u8 plugin ID, exact app bytes.</summary>
    public abstract class BasePluginScertMessage : BaseScertMessage
    {
        public byte PluginId { get; set; }
        public byte[] Payload { get; set; } = Array.Empty<byte>();
        public bool IsValid { get; private set; }
        public string ValidationError { get; private set; }

        public override void Deserialize(MessageReader reader)
        {
            // BaseScertMessage catches deserialization exceptions, so never expose a
            // partially read message as valid. The application handler must check this.
            IsValid = false;
            Payload = Array.Empty<byte>();
            ValidationError = "truncated plugin header";
            if (reader.BaseStream.Length - reader.BaseStream.Position < 3)
                return;
            int declaredLength = reader.ReadUInt16();
            PluginId = reader.ReadByte();
            if ((declaredLength & 0xc000) != 0)
            {
                ValidationError = "plugin fragmentation flags are unsupported";
                return;
            }
            if (declaredLength != reader.BaseStream.Length - reader.BaseStream.Position)
            {
                ValidationError = "plugin length differs from remaining SCERT payload";
                return;
            }
            var payload = reader.ReadBytes(declaredLength);
            if (payload.Length != declaredLength)
            {
                ValidationError = "truncated plugin application payload";
                return;
            }
            Payload = payload;
            ValidationError = null;
            IsValid = true;
        }

        public override void Serialize(MessageWriter writer)
        {
            if (Payload == null || Payload.Length > 0x3fff || Payload.Length > 10 * 1024 - 3)
                throw new ArgumentException("Plugin payload exceeds the unfragmented envelope or upstream SCERT buffer limit.");
            writer.Write((ushort)Payload.Length);
            writer.Write(PluginId);
            writer.Write(Payload);
        }

        public override string ToString() => base.ToString() +
            $" PluginId:{PluginId} PayloadLength:{Payload.Length} Valid:{IsValid} Error:{ValidationError}";
    }
}
