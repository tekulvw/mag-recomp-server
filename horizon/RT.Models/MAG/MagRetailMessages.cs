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
    // The launcher logs out of MLS just before handing off to the engine. The request is the
    // common header alone (70 bytes); the reply is the common header and one success bit, the
    // same shape as the retail NP authentication reply.
    public sealed class MagRetailAccountLogoutRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0030;
    }
    public sealed class MagRetailAccountLogoutResponse : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0031;
        public bool Success { get; set; }
    }
}
