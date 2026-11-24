namespace RT.Models.MAG
{
    // Retail lobby nomination preference: type16 followed directly by one byte of
    // squad/platoon/OIC volunteer bits (1/2/4), with no RSA/transaction header.
    // Observed on the lobby connection just before a join request; the client does
    // not wait for a reply, and the server sends none.
    public sealed class MagRetailNominationsUpdate : MagMessage
    {
        public const byte KnownBits = 0x07;
        public override ushort MessageType => 0x0243;
        public byte Nominations { get; set; }
    }
}
