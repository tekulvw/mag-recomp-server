using RT.Common;
namespace RT.Models
{
    [ScertMessage(RT_MSG_TYPE.RT_MSG_SERVER_PLUGIN_TO_APP)]
    public sealed class RT_MSG_SERVER_PLUGIN_TO_APP : BasePluginScertMessage
    {
        public override RT_MSG_TYPE Id => RT_MSG_TYPE.RT_MSG_SERVER_PLUGIN_TO_APP;
    }
}
