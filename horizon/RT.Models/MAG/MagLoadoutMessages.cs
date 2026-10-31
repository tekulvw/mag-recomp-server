namespace RT.Models.MAG
{
    // Native NetGameMessage: type16 followed directly by index + one inventory.
    // Unlike character-data requests, this message has no RSA/transaction header.
    public sealed class MagLoadoutUpdateRequest : MagMessage
    {
        public override ushort MessageType => 0x021f;
        public byte LoadoutIndex { get; set; }
        public MagLoadout Loadout { get; set; } = new MagLoadout();
    }
    public sealed class MagLoadoutUpdateResponse : MagMessage
    {
        public override ushort MessageType => 0x0220;
        public bool Success { get; set; }
    }
}
