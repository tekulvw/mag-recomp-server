namespace RT.Models.MAG
{
    // Player-card status queries the retail client sends when a friend (or another player) is
    // selected: is the account online, and is it in a game. None of the four carries the common
    // RSA/transaction header; the body follows the type directly. The client keeps one such
    // request outstanding at a time and waits for the matching reply type, so an unanswered
    // request leaves the card on "info unavailable" until its timeout.
    public sealed class MagRetailPlayerOnlineRequest : MagMessage
    {
        public override ushort MessageType => 0x022d;
        public uint AccountId { get; set; }
    }

    // Body: the account id asked about, then one online bit (7 bytes with the type).
    public sealed class MagRetailPlayerOnlineResponse : MagMessage
    {
        public override ushort MessageType => 0x022e;
        public uint AccountId { get; set; }
        public bool Online { get; set; }
    }

    public sealed class MagRetailPlayerGameInfoRequest : MagMessage
    {
        public override ushort MessageType => 0x022b;
        public uint AccountId { get; set; }
    }

    // Body: account id, game id, level name (16-bit length + ASCII), game type byte, then two
    // more 8-bit values. A game id of 0 means "not in a game": the client then ignores the rest
    // and resets its own copies, so the remaining fields only matter for a non-zero game id.
    // The meaning of the two trailing values is not established; NotInGame fills them with
    // 0xFF, the "none" value the client itself uses when the game id is 0.
    public sealed class MagRetailPlayerGameInfoResponse : MagMessage
    {
        public override ushort MessageType => 0x022c;
        public uint AccountId { get; set; }
        public uint GameId { get; set; }
        public string LevelName { get; set; } = string.Empty;
        public byte GameType { get; set; }
        public byte TrailingValue1 { get; set; } = 0xff;
        public byte TrailingValue2 { get; set; } = 0xff;

        public static MagRetailPlayerGameInfoResponse NotInGame(uint accountId) =>
            new MagRetailPlayerGameInfoResponse { AccountId = accountId };
    }
}
