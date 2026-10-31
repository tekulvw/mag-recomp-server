namespace RT.Models.MAG
{
    // Retail original002D is header-only; prototype002D remains logout.
    public sealed class MagTrainingCompletedRequest : MagPluginClientMessage
    {
        public override ushort MessageType => 0x002d;
    }
}
