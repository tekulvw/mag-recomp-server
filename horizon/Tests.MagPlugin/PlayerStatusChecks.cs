using System.Collections.Concurrent;
using System.Text.Json.Nodes;
using RT.Models;
using RT.Models.MAG;
using Server.Medius;

// Invented accounts and channels built in memory; no client data or real stores.
internal static partial class LeadershipProgressionChecks
{
    // Binds a ready MLS session through a registry the test shares between sessions, the way
    // every MLS channel binds through the one process registry.
    private static MagMlsSession MakeSharedSession(LocalCharacterStore store, MagLoginRegistry registry, string account, string channel)
    {
        var now = DateTime.UtcNow;
        var ticket = registry.IssueForAccount(account, store.ReadForAccount(account).Single().Id, 1, now);
        var session = new MagMlsSession { CharacterStore = store };
        Check(session.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20231, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey },
            registry, channel, now) && session.Accept(now) && session.CompleteTransport(now), "Synthetic shared MLS transport setup failed");
        foreach (var message in new MagMessage[] { new MagHello(), new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion } })
            session.HandleMany(CardEnvelope(message, 20231), now);
        Check(session.State == MagMlsSession.SessionState.Ready, "Synthetic shared MLS handshake failed");
        return session;
    }

    private static byte[] AskStatus(MagMlsSession session, MagMessage request)
    {
        var replies = session.HandleMany(CardEnvelope(request, 20231), DateTime.UtcNow);
        Check(session.LastRefusal == null, $"Player status request {request.MessageType:X4} was refused: {session.LastRefusal}");
        Check(replies.Count == 1, $"A ready MLS session must answer player status request {request.MessageType:X4} exactly once");
        Check(replies[0].PluginId == MagPluginRouting.MlsPluginId(20231), "Player status reply must carry the MLS plugin id");
        return replies[0].Payload;
    }

    private static MagRetailPlayerOnlineResponse AskOnline(MagMlsSession session, uint accountId)
    {
        byte[] payload = AskStatus(session, new MagRetailPlayerOnlineRequest { AccountId = accountId });
        Check(MagMessageCodec.TryParseForApplication(payload, 20231, out var parsed) && parsed is MagRetailPlayerOnlineResponse,
            "Online reply did not parse as a 022E response");
        var online = (MagRetailPlayerOnlineResponse)parsed;
        Check(online.AccountId == accountId, "Online reply must name the account asked about");
        return online;
    }

    private static MagRetailPlayerGameInfoResponse AskGameInfo(MagMlsSession session, uint accountId)
    {
        byte[] payload = AskStatus(session, new MagRetailPlayerGameInfoRequest { AccountId = accountId });
        Check(MagMessageCodec.TryParseForApplication(payload, 20231, out var parsed) && parsed is MagRetailPlayerGameInfoResponse,
            "Game-info reply did not parse as a 022C response");
        var info = (MagRetailPlayerGameInfoResponse)parsed;
        Check(info.AccountId == accountId, "Game-info reply must name the account asked about");
        return info;
    }

    public static void PlayerStatusWire()
    {
        // Requests: type, then the 32-bit account id the card shows (6 bytes, no common header).
        Check(MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("022d00000008"), 20231, out var online) &&
            online is MagRetailPlayerOnlineRequest { AccountId: 8 }, "Online request did not parse");
        Check(MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("022b00000008"), 20231, out var game) &&
            game is MagRetailPlayerGameInfoRequest { AccountId: 8 }, "Game-info request did not parse");
        Check(!MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("022d0000000800"), 20231, out _),
            "An online request with trailing data must be rejected");
        Check(!MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("022d000000"), 20231, out _),
            "A truncated online request must be rejected");
        Check(!MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("022d00000008"), 20230, out _),
            "Player status messages are retail-only");
        // Server replies are not client messages.
        Check(!MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("022e0000000880"), 20231, out _),
            "A server online reply must not be accepted from a client");

        // Online reply: account id, then one online bit padded to the byte.
        Check(Convert.ToHexString(MagMessageCodec.SerializeForApplication(
            new MagRetailPlayerOnlineResponse { AccountId = 8, Online = true }, 20231)) == "022E0000000880", "Online reply wire");
        Check(Convert.ToHexString(MagMessageCodec.SerializeForApplication(
            new MagRetailPlayerOnlineResponse { AccountId = 8, Online = false }, 20231)) == "022E0000000800", "Offline reply wire");
        // Not in a game: account id, game id 0, empty level name, game type 0, two "none" bytes.
        Check(Convert.ToHexString(MagMessageCodec.SerializeForApplication(MagRetailPlayerGameInfoResponse.NotInGame(8), 20231)) ==
            "022C0000000800000000000000FFFF", "Not-in-game reply wire");
        var inGame = new MagRetailPlayerGameInfoResponse { AccountId = 8, GameId = 3, LevelName = "lvl", GameType = 2, TrailingValue1 = 4, TrailingValue2 = 5 };
        Check(MagMessageCodec.TryParseForApplication(MagMessageCodec.SerializeForApplication(inGame, 20231), 20231, out var round) &&
            round is MagRetailPlayerGameInfoResponse { AccountId: 8, GameId: 3, LevelName: "lvl", GameType: 2, TrailingValue1: 4, TrailingValue2: 5 },
            "Game-info reply round trip");
        bool refused = false;
        try { MagMessageCodec.SerializeForApplication(new MagRetailPlayerOnlineResponse { AccountId = 8 }, 20230); }
        catch (ArgumentException) { refused = true; }
        Check(refused, "Player status replies must not serialize for the prototype application");
    }

    public static void FriendCardStatus() => WithDirectory(directory =>
        WithApplication(20231, () => VerifyFriendCard(directory).GetAwaiter().GetResult()));

    private static async Task VerifyFriendCard(string directory)
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, Catalog()) };
        foreach (var account in new[] { "alpha", "bravo", "charlie" })
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = char.ToUpperInvariant(account[0]) + account[1..], Faction = 2,
                UniverseId = 26, NpId = new byte[36] }, account).Result == MagNetResult.Success, "Synthetic friend character creation failed");
        uint bravoId = store.ReadForAccount("bravo").Single().Id, charlieId = store.ReadForAccount("charlie").Single().Id;

        var logins = new MagLoginRegistry();
        var alpha = MakeSharedSession(store, logins, "alpha", "channel-alpha");
        var bravo = MakeSharedSession(store, logins, "bravo", "channel-bravo");

        // Selecting an online friend: character data, then online status, then game info, each answered.
        var card = AskCard(alpha, new MagCharacterDataRequest { TransactionId = 61, QueryKind = 2, AccountId = bravoId, RequestId = 3 });
        Check(card.Success && card.Record.AccountId == bravoId && card.Record.Name == "Bravo" && card.Record.Faction == 2 && card.Record.Level >= 1,
            "An online friend's card must carry their name, level and faction");
        Check(AskOnline(alpha, bravoId).Online, "A friend with a live MLS session must be reported online");
        Check(alpha.StorageStatus == $"player-online=yes/account={bravoId}/session={alpha.AccountId}", "Online status line");
        var info = AskGameInfo(alpha, bravoId);
        Check(info.GameId == 0 && info.LevelName == string.Empty, "A lobby friend must be reported as not in a game");
        Check(AskOnline(alpha, alpha.AccountId).Online, "The session's own account is online");

        // An offline friend gets a definite "offline" answer instead of a timeout.
        var offlineCard = AskCard(alpha, new MagCharacterDataRequest { TransactionId = 62, QueryKind = 2, AccountId = charlieId, RequestId = 4 });
        Check(offlineCard.Success && offlineCard.Record.Name == "Charlie", "An offline friend's card still carries their character");
        Check(!AskOnline(alpha, charlieId).Online, "A friend without a session must be reported offline");
        Check(AskGameInfo(alpha, charlieId).GameId == 0, "An offline friend is not in a game");
        Check(!AskOnline(alpha, 0).Online && !AskOnline(alpha, 999).Online, "Unknown account ids are offline");

        // A pending (not yet bound) login is not online; a released channel goes offline.
        logins.IssueForAccount("charlie", charlieId, 1, DateTime.UtcNow);
        Check(!AskOnline(alpha, charlieId).Online, "An unbound login ticket must not count as online");
        logins.Release("channel-bravo");
        Check(!AskOnline(alpha, bravoId).Online, "A friend whose channel closed must be reported offline");
        bravo = MakeSharedSession(store, logins, "bravo", "channel-bravo-2");
        Check(AskOnline(alpha, bravoId).Online, "A friend who logs back in must be reported online again");

        // A session that has not finished the handshake answers nothing.
        var pendingTicket = logins.IssueForAccount("charlie", charlieId, 1, DateTime.UtcNow);
        var pending = new MagMlsSession { CharacterStore = store };
        Check(pending.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20231, TargetWorldId = 1, AccessToken = pendingTicket.AccessToken,
            SessionKey = pendingTicket.SessionKey }, logins, "channel-charlie", DateTime.UtcNow), "Synthetic pending bind failed");
        Check(pending.HandleMany(CardEnvelope(new MagRetailPlayerOnlineRequest { AccountId = bravoId }, 20231), DateTime.UtcNow).Count == 0,
            "A non-ready MLS session answered an online request");

        // Inviting the online friend from the card reaches them (parties enabled in the registry).
        await using var worker = new RegistryWorker("first") { SupportsParties = true };
        var config = new JsonObject { ["format"] = "mag_world_registry_v1", ["application_id"] = 20231,
            ["client_address"] = "192.0.2.9", ["client_tcp_port"] = 12073, ["party_invite_seconds"] = 30,
            ["workers"] = new JsonArray(new JsonObject { ["worker_id"] = "first", ["worker_host"] = "127.0.0.1",
                ["worker_port"] = worker.Port, ["minimum_players"] = 2, ["maximum_players"] = 4,
                ["minimum_players_per_team"] = new JsonArray(1, 1), ["draining"] = false }) };
        await using var registry = new MagWorldRegistry(MagWorldRegistryConfig.Parse(config.ToJsonString()), store);
        await registry.InitializeAsync();
        var alphaMessages = new ConcurrentQueue<MagMessage>();
        var bravoMessages = new ConcurrentQueue<MagMessage>();
        registry.RegisterPartySession(alpha, () => true, alphaMessages.Enqueue, null);
        registry.RegisterPartySession(bravo, () => true, bravoMessages.Enqueue, null);
        Check(registry.RoutePartyMembership(alpha, new MagRetailPartyInvite { Inviter = alpha.AccountId, Invitee = bravoId }),
            "The invite to an online friend was not routed");
        var delivered = bravoMessages.OfType<MagRetailPartyInvite>().Single();
        Check(delivered.Inviter == alpha.AccountId && delivered.Invitee == bravoId && delivered.PartyId != 0,
            "The online friend must receive the invite naming the inviter and the new party");
        Check(alphaMessages.OfType<MagRetailPartyJoined>().Single().PartyId == delivered.PartyId,
            "The inviter must be placed in the party the invite names");

        // A lease that ran out (no keepalive for two minutes) no longer counts as online.
        Check(logins.IsAccountOnline(alpha.AccountId, DateTime.UtcNow) &&
            !logins.IsAccountOnline(alpha.AccountId, DateTime.UtcNow.AddSeconds(121)), "An expired lease must not count as online");
    }
}
