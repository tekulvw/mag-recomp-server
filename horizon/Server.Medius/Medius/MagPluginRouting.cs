using RT.Models;
using RT.Models.MAG;

namespace Server.Medius
{
    public enum MagPluginService { Disabled, Maps, Mas }

    public static class MagPluginRouting
    {
        public static bool IsKnownMagApplication(int applicationId) => applicationId == 20230 || applicationId == 20231;
        public static bool IsSelectedApplication(int applicationId) => applicationId == Program.Settings.MagApplicationId;

        // Ambiguous dual-mode configuration must never guess which hello response to send.
        public static MagPluginService Resolve(bool mapsEnabled, bool masEnabled) =>
            mapsEnabled == masEnabled ? MagPluginService.Disabled :
            mapsEnabled ? MagPluginService.Maps : MagPluginService.Mas;

        // The retail client has no plugin id of its own on a Medius connection. It remembers the
        // plugin id of the last Medius plugin message it received and sends its plugin messages,
        // including its clan library's requests, with that id. The clan library refuses to send
        // while the id is 0, so retail MLS replies carry a non-zero id: the one the gameplay DME
        // already gives the retail game plugin. The prototype keeps 0.
        public const byte RetailMlsPluginId = 1;

        public static byte MlsPluginId(int applicationId) => applicationId == 20231 ? RetailMlsPluginId : (byte)0;

        // A client's first message precedes any reply and carries 0; afterwards it echoes our id.
        // MAS accepts the same pair, since the client keeps the id across its Medius connections.
        public static bool IsClientPluginId(int applicationId, byte pluginId) =>
            pluginId == 0 || pluginId == MlsPluginId(applicationId);

        // One server instance is configured for one wire protocol. Sessions pin
        // it before their initial reply so a config reload cannot reinterpret IDs.
        public static bool TryParseClient(byte[] payload, int applicationId, uint protocol, out MagMessage message) =>
            protocol == 1793
                ? MagMessageCodec.TryParseClientForProtocol(payload, applicationId, protocol, out message)
                : MagMessageCodec.TryParseClientForApplication(payload, applicationId, out message);

        public static RT_MSG_SERVER_PLUGIN_TO_APP ToClient(MagMessage message, int applicationId, uint protocol) =>
            new RT_MSG_SERVER_PLUGIN_TO_APP {PluginId = 0, Payload = protocol == 1793
                ? MagMessageCodec.SerializeForProtocol(message, applicationId, protocol)
                : MagMessageCodec.SerializeForApplication(message, applicationId)};

        public static RT_MSG_SERVER_PLUGIN_TO_APP ToMlsClient(MagMessage message, int applicationId, uint? protocol = null)
        {
            var envelope = ToClient(message, applicationId, protocol ?? Program.Settings.MagProtocolVersion);
            envelope.PluginId = MlsPluginId(applicationId);
            return envelope;
        }

        // For payloads outside the MAG message codec, such as retail clan messages.
        public static RT_MSG_SERVER_PLUGIN_TO_APP ToMlsClient(byte[] payload, int applicationId) =>
            new RT_MSG_SERVER_PLUGIN_TO_APP { PluginId = MlsPluginId(applicationId), Payload = payload };
    }
}
