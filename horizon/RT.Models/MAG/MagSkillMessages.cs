namespace RT.Models.MAG
{
    // Native 0250 is a NetGameMessage: signed-byte skill ordinal, no MAS header.
    public sealed class MagPurchaseSkillRequest : MagMessage
    {
        public override ushort MessageType => 0x0250;
        public sbyte SkillId { get; set; } = -1;
    }
    // Native 0251 always carries the full record, including on failure.
    public sealed class MagPurchaseSkillResponse : MagMessage
    {
        public override ushort MessageType => 0x0251;
        public bool Success { get; set; }
        public MagCharacterRecord Record { get; set; } = new MagCharacterRecord();
    }
}
