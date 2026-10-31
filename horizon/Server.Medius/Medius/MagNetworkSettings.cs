using System;
using System.Globalization;
using System.Net;
using System.Net.Sockets;

namespace Server.Medius
{
    /// <summary>Client-visible endpoints; listener ports are configured separately.</summary>
    public sealed class MagNetworkSettings
    {
        public IPAddress Address { get; private set; }
        public string AuthDns { get; private set; }
        public ushort AuthPort { get; private set; }
        public ushort LobbyPort { get; private set; }
        public ushort GameTcpPort { get; private set; }

        public static MagNetworkSettings FromEnvironment() => Load(Environment.GetEnvironmentVariable);

        public static MagNetworkSettings Load(Func<string, string> read)
        {
            string raw = read("MAG_ADVERTISE_IPV4") ?? "127.0.0.1";
            // Require dotted decimal: IPAddress.TryParse alone accepts abbreviated IPv4.
            string[] octets = raw.Split('.');
            if (octets.Length != 4 || Array.Exists(octets, part => part.Length == 0 ||
                part.Length > 3 || (part.Length > 1 && part[0] == '0') ||
                !byte.TryParse(part, NumberStyles.None, CultureInfo.InvariantCulture, out _)) ||
                !IPAddress.TryParse(raw, out var address) || address.AddressFamily != AddressFamily.InterNetwork ||
                raw == "0.0.0.0" || raw == "255.255.255.255" || address.GetAddressBytes()[0] >= 224)
                throw new ArgumentException("MAG_ADVERTISE_IPV4 requires a reachable unicast dotted IPv4 address.");
            string dns = read("MAG_AUTH_DNS") ?? "mag.test";
            if (dns.Length > 253 || Uri.CheckHostName(dns) != UriHostNameType.Dns)
                throw new ArgumentException("MAG_AUTH_DNS requires a DNS hostname without a scheme or path.");
            return new MagNetworkSettings
            {
                Address = address, AuthDns = dns,
                AuthPort = Port(read, "MAG_AUTH_PORT", 10475),
                LobbyPort = Port(read, "MAG_LOBBY_PORT", 10478),
                GameTcpPort = Port(read, "MAG_GAME_TCP_PORT", 10473)
            };
        }

        private static ushort Port(Func<string, string> read, string name, ushort fallback)
        {
            string raw = read(name);
            if (raw == null) return fallback;
            if (!ushort.TryParse(raw, NumberStyles.None, CultureInfo.InvariantCulture, out var value) || value == 0)
                throw new ArgumentException(name + " requires a port in 1..65535.");
            return value;
        }
    }
}
