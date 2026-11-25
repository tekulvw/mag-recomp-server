using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Sockets;

namespace Server.Medius
{
    public sealed class LocalDmeHostOptions
    {
        public int ApplicationId { get; }
        public int GameType { get; }
        public MagMatchStartupPolicy StartupPolicy { get; }
        public IReadOnlyList<int> MinimumPlayersPerTeam { get; }
        public IReadOnlyList<int> AllowedFactions { get; }
        public IPAddress AdvertisedAddress { get; }
        public ushort GameTcpPort { get; }
        public string WorkerId { get; }
        /// <summary>The largest world this host may form. A profile above 64 Characters is offered
        /// only when it fits; human admission stays within <see cref="StartupPolicy"/>.</summary>
        public int WorldCapacity { get; }

        public LocalDmeHostOptions(int applicationId, int gameType, int minimumPlayers, int maximumPlayers,
            int[] minimumPlayersPerTeam, int[] allowedFactions, IPAddress advertisedAddress, ushort gameTcpPort,
            string workerId = null, int? worldCapacity = null)
        {
            if (workerId != null && (applicationId != 20231 || !GameplayWorkerIdentity.IsValidIdentifier(workerId)))
                throw new ArgumentException("A configured gameplay worker requires a valid identity and retail application 20231.", nameof(workerId));
            if (advertisedAddress?.AddressFamily != AddressFamily.InterNetwork || advertisedAddress.Equals(IPAddress.Any) ||
                advertisedAddress.GetAddressBytes()[0] >= 224 || gameTcpPort == 0)
                throw new ArgumentException("Local gameplay requires a unicast IPv4 address and TCP port.");
            var factions = allowedFactions == null ? new[] { 0, 1, 2 } : (int[])allowedFactions.Clone();
            if (factions.Length == 0 || factions.Any(faction => faction < 0 || faction > 2) || factions.Distinct().Count() != factions.Length)
                throw new ArgumentException("Allowed factions must contain distinct values in 0..2.", nameof(allowedFactions));
            var minima = minimumPlayersPerTeam == null ? null : (int[])minimumPlayersPerTeam.Clone();
            StartupPolicy = new MagMatchStartupPolicy(minimumPlayers, maximumPlayers, minima);
            int supportedMaximum = LocalDmeHost.SupportedMaximumPlayers(applicationId, gameType, hasDominationProfiles: true);
            if (maximumPlayers > supportedMaximum)
                throw new ArgumentException($"Local match limits must be 1..{supportedMaximum}.", nameof(maximumPlayers));
            ApplicationId = applicationId;
            GameType = gameType;
            MinimumPlayersPerTeam = minima == null ? null : Array.AsReadOnly(minima);
            AllowedFactions = Array.AsReadOnly(factions);
            AdvertisedAddress = advertisedAddress;
            GameTcpPort = gameTcpPort;
            WorkerId = workerId;
            WorldCapacity = worldCapacity ?? maximumPlayers;
            if (WorldCapacity < maximumPlayers || WorldCapacity > MagWorldCapacity.MaximumWorldCharacters)
                throw new ArgumentException("World capacity must cover the human maximum and stay within 256.", nameof(worldCapacity));
        }

        internal static LocalDmeHostOptions FromLegacySettings()
        {
            var settings = Program.Settings;
            var network = MagNetworkSettings.FromEnvironment();
            return new LocalDmeHostOptions(settings.MagApplicationId, settings.MagGameType, settings.MagMinimumPlayers,
                settings.MagMaximumPlayers, settings.MagMinimumPlayersPerTeam, settings.MagAllowedFactions,
                network.Address, network.GameTcpPort);
        }
    }
}
