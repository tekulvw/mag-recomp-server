using RT.Common;
namespace RT.Models
{
    [ScertMessage(RT_MSG_TYPE.RT_MSG_CLIENT_APP_TO_PLUGIN)]
    public sealed class RT_MSG_CLIENT_APP_TO_PLUGIN : BasePluginScertMessage
    {
        public int ApplicationMessageType => Payload != null && Payload.Length >= 2 ? (Payload[0] << 8) | Payload[1] : -1;
        public override RT_MSG_TYPE Id => RT_MSG_TYPE.RT_MSG_CLIENT_APP_TO_PLUGIN;
    }
}
