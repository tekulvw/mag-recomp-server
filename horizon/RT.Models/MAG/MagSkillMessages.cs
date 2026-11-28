namespace RT.Models.MAG
{
    // Native 0250 is a NetGameMessage: signed-byte skill ordinal, no MAS header.
    public sealed class MagPurchaseSkillRequest : MagMessage
    {
        public override ushort MessageType => 0x0250;
        public sbyte SkillId { get; set; } = -1;
    }
    // Protocol 1793 uses plain NetGameMessage 0268 and a signed 16-bit ordinal. Keep
    // the older signed-byte DTO distinct. This request is offline-codec only
    // until the 1793 catalog and live purchase policy are qualified.
    public sealed class MagPurchaseSkill212Request : MagMessage
    {
        public override ushort MessageType => 0x0250;
        public short SkillId { get; set; } = -1;
    }
    // Native 0251 always carries the full record, including on failure.
    public sealed class MagPurchaseSkillResponse : MagMessage
    {
        public override ushort MessageType => 0x0251;
        public bool Success { get; set; }
        public MagCharacterRecord Record { get; set; } = new MagCharacterRecord();
    }
    // Retail 0254 is the same NetGameMessage family: no MAS header and no payload. The client
    // names no character, fee or transaction; the server answers for the session's character.
    public sealed class MagRetailSkillRespecRequest : MagMessage
    {
        public override ushort MessageType => 0x0254;
    }
    // Retail 0255: a success bit, then the full retail character record only on success.
    public sealed class MagRetailSkillRespecResponse : MagMessage
    {
        public override ushort MessageType => 0x0255;
        public bool Success { get; set; }
        public MagCharacterRecord Record { get; set; } = new MagCharacterRecord();
    }
}
