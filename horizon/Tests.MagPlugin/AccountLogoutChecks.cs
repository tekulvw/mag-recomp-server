using System.Text;
using RT.Models;
using RT.Models.MAG;
using Server.Medius;

// Invented accounts, transaction ids and RSA words built in memory; no client data or real stores.
internal static partial class LeadershipProgressionChecks
{
    public static void AccountLogoutWire()
    {
        // The request is the common header alone: type, sixteen RSA words and the transaction id.
        var request = new MagRetailAccountLogoutRequest { TransactionId = 0x01020304 };
        for (int i = 0; i < 16; i++) request.RsaWords[i] = 0x11111111u * (uint)(i % 15 + 1);
        byte[] payload = MagMessageCodec.SerializeForApplication(request, 20231);
        Check(payload.Length == 70 && payload[0] == 0x00 && payload[1] == 0x30 &&
            payload[66] == 0x01 && payload[67] == 0x02 && payload[68] == 0x03 && payload[69] == 0x04,
            "Logout request must be the 70-byte common header with type 0030");
        Check(MagMessageCodec.TryParseClientForApplication(payload, 20231, out var parsed) &&
            parsed is MagRetailAccountLogoutRequest decoded && decoded.TransactionId == 0x01020304 &&
            decoded.RsaWords.SequenceEqual(request.RsaWords), "Retail MLS did not decode the launcher's logout request");
        Check(!MagMessageCodec.TryParseClientForApplication(payload, 20230, out _), "The prototype must not claim retail 0030");
        Check(!MagMessageCodec.TryParseClientForApplication(payload[..69], 20231, out _), "A truncated logout request parsed");
        Check(!MagMessageCodec.TryParseClientForApplication(payload.Concat(new byte[] { 0x01 }).ToArray(), 20231, out _),
            "Trailing logout request bytes parsed");

        // The reply is the header and one success bit, padded to a byte: 71 bytes.
        byte[] reply = MagMessageCodec.SerializeForApplication(new MagRetailAccountLogoutResponse { TransactionId = 7, Success = true }, 20231);
        Check(reply.Length == 71 && reply[0] == 0x00 && reply[1] == 0x31 && reply[69] == 0x07 && reply[70] == 0x80,
            "Logout reply must be type 0031, the header and a leading success bit");
        Check(MagMessageCodec.TryParseForApplication(reply, 20231, out var answer) &&
            answer is MagRetailAccountLogoutResponse { Success: true, TransactionId: 7 }, "Retail logout reply did not round-trip");
        Check(MagMessageCodec.SerializeForApplication(new MagRetailAccountLogoutResponse(), 20231)[70] == 0x00,
            "A failed logout reply must clear the success bit");
        Check(!MagMessageCodec.TryParseClientForApplication(reply, 20231, out _), "A server-only logout reply parsed as a client message");
        // Retail 0031 is the logout reply; on the prototype it remains the NP authentication request.
        Check(!MagMessageCodec.TryParseForApplication(reply, 20230, out var prototype) || prototype is not MagRetailAccountLogoutResponse,
            "The prototype decoded 0031 as a logout reply");
        foreach (MagMessage retailOnly in new MagMessage[] { request, new MagRetailAccountLogoutResponse() })
        {
            bool refused = false;
            try { MagMessageCodec.SerializeForApplication(retailOnly, 20230); }
            catch (ArgumentException) { refused = true; }
            Check(refused, "Logout messages serialized for the prototype");
        }
    }

    public static void AccountLogoutKeepsNextLogin() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, Catalog()) };
        Create(store, "alpha", "Alpha");
        WithApplication(20231, () =>
        {
            var now = DateTime.UtcNow;
            var registry = new MagLoginRegistry();
            var launcherTicket = registry.IssueForAccount("alpha", 1, 1, now);
            var launcher = ReadyMls(store, registry, launcherTicket.AccessToken, launcherTicket.SessionKey, "synthetic-launcher-channel", now);

            // A pending (handshake-incomplete) session stays silent, like every other request.
            var pendingTicket = registry.IssueForAccount("alpha", 1, 1, now);
            var pending = new MagMlsSession { CharacterStore = store };
            Check(pending.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20231, TargetWorldId = 1, AccessToken = pendingTicket.AccessToken, SessionKey = pendingTicket.SessionKey },
                registry, "synthetic-pending-channel", now) && pending.Accept(now) && pending.CompleteTransport(now), "Synthetic pending MLS session setup failed");
            Check(pending.HandleMany(CardEnvelope(new MagRetailAccountLogoutRequest { TransactionId = 3 }, 20231), now).Count == 0,
                "A non-ready MLS session answered a logout request");

            // The launcher's logout gets exactly one 0031 success carrying its transaction id.
            var replies = launcher.HandleMany(CardEnvelope(new MagRetailAccountLogoutRequest { TransactionId = 0x5150 }, 20231), now);
            Check(replies.Count == 1 && replies[0].PluginId == MagPluginRouting.RetailMlsPluginId && replies[0].Payload.Length == 71, "Logout was not answered with one reply");
            Check(MagMessageCodec.TryParseForApplication(replies[0].Payload, 20231, out var parsed) &&
                parsed is MagRetailAccountLogoutResponse { Success: true, TransactionId: 0x5150 }, "Logout reply is not a 0031 success for the request");
            Check(launcher.StorageStatus == "account-logout-acknowledged", "Logout acknowledgement is not visible in the MLS status");

            // Acknowledge-only: the launcher channel keeps its identity and lease until it closes.
            Check(launcher.State == MagMlsSession.SessionState.Ready && launcher.AuthenticatedAccount == "alpha" &&
                launcher.CharacterId == 1 && launcher.SessionKey == launcherTicket.SessionKey, "Logout changed the launcher's MLS session");
            Check(registry.ActiveAccounts(now).ContainsKey("alpha") && registry.Touch("synthetic-launcher-channel", now),
                "Logout released the launcher ticket before its channel closed");
            var keepAlive = launcher.HandleMany(CardEnvelope(new MagKeepAlive(), 20231), now);
            Check(keepAlive.Count == 1 && MagMessageCodec.TryParseForApplication(keepAlive[0].Payload, 20231, out var echoed) && echoed is MagKeepAlive,
                "Keepalive after logout went unanswered");
            Check(store.ReadForAccount("alpha").Single().Name == "Alpha", "Logout changed the stored character");

            // The engine's following login: MAS signs in, issues a fresh ticket from the same registry, and MLS binds it.
            var mas = new MagMasSession { DevelopmentAuthEnabled = true, AccountLoginEnabled = true, CharacterStorageEnabled = true,
                CharacterStore = store, LoginRegistry = registry };
            MasSend(mas, new MagHello(), now);
            MasSend(mas, new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, now);
            var auth = new MagRetailNpAuthenticateRequest { TransactionId = 1, FullTicketSize = 5, TicketChunkSize = 5, LastMessage = true };
            Encoding.ASCII.GetBytes("alpha").CopyTo(auth.TicketChunk, 0);
            Check(MasSend(mas, auth, now).Count == 1 && mas.State == MagMasSession.SessionState.LocalDevelopmentAuthenticated,
                "Synthetic engine MAS sign-in failed");
            var login = MasSend(mas, new MagRetailAccountLoginRequest { TransactionId = 2, VersionMajor = 3, VersionMinor = 3, VersionBuild = 1, AccountName = "Alpha" }, now);
            MagMessage loginReply = null;
            Check(login.Count == 1 && MagMessageCodec.TryParseForApplication(login[0].Payload, 20231, out loginReply) &&
                loginReply is MagRetailAccountLoginResponse { Result: 1, AccountId: 1 },
                "Engine account login after the launcher's logout was refused: " + mas.StorageStatus);
            var granted = (MagRetailAccountLoginResponse)loginReply;
            var engine = ReadyMls(store, registry, Encoding.ASCII.GetString(granted.AccessKey, 0, 16), Encoding.ASCII.GetString(granted.SessionKey, 0, 16),
                "synthetic-engine-channel", now);
            Check(engine.AuthenticatedAccount == "alpha" && engine.CharacterId == 1, "Engine MLS session lost the account after the launcher's logout");
            var card = engine.HandleMany(CardEnvelope(new MagCharacterDataRequest { TransactionId = 9, QueryKind = 2, AccountId = 1, RequestId = 1 }, 20231), now);
            Check(card.Count == 1 && MagMessageCodec.TryParseForApplication(card[0].Payload, 20231, out var data) &&
                data is MagCharacterDataResponse { Success: true } served && served.Record.Name == "Alpha",
                "Engine MLS session could not read its character after the launcher's logout");

            // Closing the launcher channel releases only the launcher ticket.
            registry.Release("synthetic-launcher-channel");
            Check(registry.ActiveAccounts(now).ContainsKey("alpha") && registry.Touch("synthetic-engine-channel", now),
                "Launcher channel close released the engine's session");
        });
    });

    private static MagMlsSession ReadyMls(LocalCharacterStore store, MagLoginRegistry registry, string accessToken, string sessionKey, string channel, DateTime now)
    {
        var session = new MagMlsSession { CharacterStore = store };
        Check(session.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20231, TargetWorldId = 1, AccessToken = accessToken, SessionKey = sessionKey },
            registry, channel, now) && session.Accept(now) && session.CompleteTransport(now), "Synthetic MLS transport setup failed");
        session.HandleMany(CardEnvelope(new MagHello(), 20231), now);
        session.HandleMany(CardEnvelope(new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, 20231), now);
        Check(session.State == MagMlsSession.SessionState.Ready, "Synthetic MLS handshake failed");
        return session;
    }

    private static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> MasSend(MagMasSession mas, MagMessage message, DateTime now) =>
        mas.HandleMany(true, 20231, CardEnvelope(message, 20231), now);
}
