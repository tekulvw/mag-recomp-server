namespace RT.Models.MAG
{
    // Retail launcher messages have no RSA/transaction header (native8ABEC/8A8BC).
    public sealed class MagRetailPatchRequest : MagMessage
    {
        public override ushort MessageType => 0x01f5;
        public int HighestVersion { get; set; }
        public int InitialVersion { get; set; }
        public string BuildString { get; set; } = string.Empty;
        public string Language { get; set; } = string.Empty;
    }

    // Only the evidenced empty-list variant is modeled; defaults come from native8AA38.
    public sealed class MagRetailNoPatchResponse : MagMessage
    {
        public override ushort MessageType => 0x01f6;
        public int MaxDownloadRate { get; set; } = -1;
        public int MaxCDNDownloadRate { get; set; } = -1;
        public int MaxPeers { get; set; } = -1;
        public uint WindowSize { get; set; } = 8192;
        public byte EncryptP2P { get; set; }
        public int MaxUploadRate { get; set; } = -1;
        public int MaxUploadSlots { get; set; } = -1;
        public int MaxDownloadSlots { get; set; } = -1;
        public int MinDownloadRate { get; set; } = -1;
        public int MaxPeerDownloadRate { get; set; } = -1;
        public int TargetCDNOffloadRate { get; set; } = -1;
    }
}
