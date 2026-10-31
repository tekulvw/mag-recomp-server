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
    }
}
