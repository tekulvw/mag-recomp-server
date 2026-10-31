using Microsoft.Extensions.Logging;
using Org.BouncyCastle.Math;
using RT.Common;
using RT.Cryptography;
using Server.Common.Logging;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.Serialization;

namespace Server.Dme.Config
{
    public class ServerSettings
    {
        /// <summary>
        /// MUM connection information
        /// </summary>
        public MPSSettings MPS { get; set; } = new MPSSettings();

        /// <summary>
        /// Key used to authenticate clients.
        /// </summary>
        public RsaKeyPair DefaultKey { get; set; } = DevelopmentRsaKeys.ForProcess;



        /// <summary>
        /// Path to the plugins directory.
        /// </summary>
        public string PluginsPath { get; set; } = "plugins/";

        /// <summary>
        /// How many milliseconds before refreshing the config.
        /// </summary>
        public int RefreshConfigInterval = 5000;

        /// <summary>
        /// Application id.
        /// </summary>
        public List<int> ApplicationIds { get; set; } = new List<int>();

        /// <summary>
        /// By default the server will grab its local ip.
        /// If this is set, it will use its public ip instead.
        /// </summary>
        public bool UsePublicIp { get; set; } = false;

        /// <summary>
        /// If UsePublicIp is set to true, allow overriding and skipping using dyndns's dynamic
        /// ip address finder, since it goes down often enough to throw exceptions
        /// </summary>
        public string PublicIpOverride { get; set; } = string.Empty;

        /// <summary>
        /// Id of the location of this dme server.
        /// </summary>
        public int Location { get; set; } = 0;

        /// <summary>
        /// Seconds between disconnects before the client attempts to reconnect to the proxy server.
        /// </summary>
        public int MPSReconnectInterval { get; set; } = 15;

        /// <summary>
        /// Number of milliseconds for main loop thread to sleep.
        /// </summary>
        public int MainLoopSleepMs { get; set; } = 5;

        /// <summary>
        /// Milliseconds between plugin ticks.
        /// </summary>
        public int PluginTickIntervalMs { get; set; } = 50;

        /// <summary>
        /// Port of the TCP server.
        /// </summary>
        public int TCPPort { get; set; } = 10073;

        /// <summary>
        /// Port to bind the udp server to.
        /// </summary>
        public int UDPPort { get; set; } = 50000;

        /// <summary>
        /// Logging settings.
        /// </summary>
        public LogSettings Logging { get; set; } = new LogSettings();
    }

    public class MPSSettings
    {
        /// <summary>
        /// Ip of the Medius Authentication server.
        /// </summary>
        public string Ip { get; set; } = "127.0.0.1";

        /// <summary>
        /// The port that the Authentication server is bound to.
        /// </summary>
        public int Port { get; set; } = 10077;

        /// <summary>
        /// Key used to establish initial handshake with MPS.
        /// </summary>
        public RsaKeyPair Key { get; set; } = DevelopmentRsaKeys.ForProcess;
    }
}
