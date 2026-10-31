namespace RT.Models.MAG
{
    // Retail launcher wire identities are distinct despite some shared scalar bodies.
    public sealed class MagRetailNpAuthenticateRequest : MagNpAuthenticateRequest
    {
        public override ushort MessageType => 0x0034;
    }
    public sealed class MagRetailNpAuthenticateResponse : MagNpAuthenticateResponse
    {
        public override ushort MessageType => 0x0035;
    }
    public sealed class MagRetailMapsHelloResponse : MagMapsHelloResponse
    {
        public override ushort MessageType => 0x0036;
        // Native source names and downstream service semantics remain unproven.
        public string AuxiliaryString { get; set; } = string.Empty;
        public ushort AuxiliaryValue { get; set; }
    }
    public sealed class MagRetailUniverseListRequest : MagUniverseListRequest
    {
        public override ushort MessageType => 0x0037;
    }
    public sealed class MagRetailUniverseListResponse : MagUniverseListResponse
    {
        public override ushort MessageType => 0x0038;
    }
}
