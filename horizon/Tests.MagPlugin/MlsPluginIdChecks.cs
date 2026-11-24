using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Medius;

// Invented accounts, transaction ids and payload bytes built in memory; no client data or real stores.
internal static partial class LeadershipProgressionChecks
{
    // The retail client sends its Medius plugin messages with the id it last received, and its
    // clan library stays silent while that id is 0, so every retail MLS reply carries a non-zero id.
    public static void MlsPluginId() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, Catalog()) };
        Create(store, "alpha", "Alpha");
        Check(MagPluginRouting.RetailMlsPluginId != 0 && MagPluginRouting.MlsPluginId(20231) == MagPluginRouting.RetailMlsPluginId &&
            MagPluginRouting.MlsPluginId(20230) == 0, "Retail MLS must use a non-zero plugin id and the prototype must keep 0");
        Check(MagPluginRouting.ToMlsClient(new MagKeepAlive(), 20231).PluginId == MagPluginRouting.RetailMlsPluginId &&
            MagPluginRouting.ToMlsClient(new MagKeepAlive(), 20230).PluginId == 0, "MLS envelope helper used the wrong plugin id");

        WithApplication(20231, () =>
        {
            byte id = MagPluginRouting.RetailMlsPluginId;
            var now = DateTime.UtcNow;
            var registry = new MagLoginRegistry();
            var ticket = registry.IssueForAccount("alpha", 1, 1, now);
            var session = new MagMlsSession { CharacterStore = store };
            Check(session.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20231, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey },
                registry, "synthetic-plugin-id-channel", now) && session.Accept(now) && session.CompleteTransport(now), "Synthetic MLS transport setup failed");

            // The hello precedes any reply, so it carries 0; the protocol reply already carries the MLS id.
            var protocol = session.HandleMany(PluginEnvelope(new MagHello(), 20231, 0), now);
            Check(protocol.Count == 1 && protocol[0].PluginId == id && MagMessageCodec.TryParseForApplication(protocol[0].Payload, 20231, out var info) &&
                info is MagProtocolInfo, "The retail protocol reply did not carry the MLS plugin id");

            // From then on the client echoes the id, and every handled request is answered with it.
            session.HandleMany(PluginEnvelope(new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, 20231, id), now);
            Check(session.State == MagMlsSession.SessionState.Ready && session.LastRefusal == null, "The echoed plugin id did not finish the MLS handshake");
            var keepAlive = session.HandleMany(PluginEnvelope(new MagKeepAlive(), 20231, id), now);
            Check(keepAlive.Count == 1 && keepAlive[0].PluginId == id, "Keepalive with the echoed id was not answered with the MLS id");
            var card = session.HandleMany(PluginEnvelope(new MagCharacterDataRequest { TransactionId = 4, QueryKind = 2, AccountId = 1, RequestId = 1 }, 20231, id), now);
            Check(card.Count == 1 && card[0].PluginId == id && MagMessageCodec.TryParseForApplication(card[0].Payload, 20231, out var data) &&
                data is MagCharacterDataResponse { Success: true }, "Character data with the echoed id was not answered with the MLS id");
            var logout = session.HandleMany(PluginEnvelope(new MagRetailAccountLogoutRequest { TransactionId = 5 }, 20231, id), now);
            Check(logout.Count == 1 && logout[0].PluginId == id, "Logout was not answered with the MLS id");
            // A message still in flight with 0 when the first reply arrived is answered too.
            var late = session.HandleMany(PluginEnvelope(new MagKeepAlive(), 20231, 0), now);
            Check(late.Count == 1 && late[0].PluginId == id, "A late plugin-0 keepalive went unanswered");

            // Any other id is refused visibly, and an unknown type is reported with its first bytes.
            Check(session.HandleMany(PluginEnvelope(new MagKeepAlive(), 20231, (byte)(id + 1)), now).Count == 0 &&
                session.LastRefusal == $"unexpected-plugin-id={id + 1}", "A foreign plugin id was not refused visibly");
            byte[] unknown = { 0x7F, 0xF0, 0x02, 0x00, 0x00, 0x00, 0x09 };
            Check(session.HandleMany(RawEnvelope(unknown, id), now).Count == 0 && session.LastRefusal == "unknown-or-malformed-type",
                "An unknown plugin type was not reported");
            Check(MagMlsSession.PayloadHead(unknown) == "7ff00200000009" &&
                MagMlsSession.PayloadHead(Enumerable.Repeat((byte)0xAB, 70).ToArray()) == string.Concat(Enumerable.Repeat("ab", 64)) + "...",
                "Unhandled payload hex head is wrong");
            Check(session.HandleMany(PluginEnvelope(new MagKeepAlive(), 20231, id), now).Count == 1 && session.LastRefusal == null,
                "The refusal reason outlived its message");

            // MAS takes the same pair, because the client keeps the id across its Medius connections.
            var mas = new MagMasSession { DevelopmentAuthEnabled = true, AccountLoginEnabled = true, CharacterStore = store, LoginRegistry = registry };
            var masReply = mas.HandleMany(true, 20231, PluginEnvelope(new MagHello(), 20231, id), now);
            Check(masReply.Count == 1 && masReply[0].PluginId == 0, "Retail MAS refused the echoed MLS plugin id");
            Check(new MagMasSession { DevelopmentAuthEnabled = true }.HandleMany(true, 20231, PluginEnvelope(new MagHello(), 20231, (byte)(id + 1)), now).Count == 0,
                "Retail MAS accepted a foreign plugin id");
        });

        WithApplication(20230, () =>
        {
            var now = DateTime.UtcNow;
            var registry = new MagLoginRegistry();
            var ticket = registry.IssueForAccount("alpha", 1, 1, now);
            var session = new MagMlsSession { CharacterStore = store };
            Check(session.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = 20230, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey },
                registry, "synthetic-prototype-channel", now) && session.Accept(now) && session.CompleteTransport(now), "Synthetic prototype MLS setup failed");
            Check(session.HandleMany(PluginEnvelope(new MagHello(), 20230, MagPluginRouting.RetailMlsPluginId), now).Count == 0,
                "The prototype accepted the retail MLS plugin id");
            var protocol = session.HandleMany(PluginEnvelope(new MagHello(), 20230, 0), now);
            Check(protocol.Count == 1 && protocol[0].PluginId == 0, "Prototype MLS replies must keep plugin id 0");
            Check(new MagMasSession { DevelopmentAuthEnabled = true }.HandleMany(true, 20230, PluginEnvelope(new MagHello(), 20230, MagPluginRouting.RetailMlsPluginId), now).Count == 0,
                "Prototype MAS accepted the retail MLS plugin id");
        });
    });

    private static RT_MSG_CLIENT_APP_TO_PLUGIN PluginEnvelope(MagMessage message, int applicationId, byte pluginId) =>
        RawEnvelope(MagMessageCodec.SerializeForApplication(message, applicationId), pluginId);

    private static RT_MSG_CLIENT_APP_TO_PLUGIN RawEnvelope(byte[] payload, byte pluginId)
    {
        using var buffer = new MemoryStream();
        using (var writer = new BinaryWriter(buffer, System.Text.Encoding.UTF8, leaveOpen: true))
        { writer.Write((ushort)payload.Length); writer.Write(pluginId); writer.Write(payload); }
        buffer.Position = 0;
        using var reader = new MessageReader(buffer);
        var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN(); envelope.Deserialize(reader);
        Check(envelope.IsValid, "Synthetic plugin envelope rejected");
        return envelope;
    }
}
