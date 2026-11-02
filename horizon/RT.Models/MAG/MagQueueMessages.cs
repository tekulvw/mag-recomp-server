namespace RT.Models.MAG
{
    /// <summary>Retail waiting update: common header, bool, signed enum byte,12 unaligned u32 counts.</summary>
    public sealed class MagRetailJoinGameUpdate : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0014;
        public bool Success {get;set;}
        // Retain the raw enum. Its native semantic names are not recovered.
        public sbyte QueuedType {get;set;}
        public uint[] QueueSizes {get;set;} = new uint[12];
    }
    public sealed class MagRetailCancelJoinRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x0015;
        public uint PartyId {get;set;}
    }
}
