using Microsoft.Extensions.Logging;
using Org.BouncyCastle.Math;
using RT.Common;
using RT.Cryptography;
using Server.Common.Logging;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.Serialization;

namespace Server.Test.Config
{
    public class ServerSettings
    {
        /// <summary>
        /// MUM connection information
        /// </summary>
        public MediusSettings Medius { get; set; } = new MediusSettings();

        /// <summary>
        /// How many milliseconds before refreshing the config.
        /// </summary>
        public int RefreshConfigInterval = 5000;

        /// <summary>
        /// Application id.
        /// </summary>
        public int ApplicationId { get; set; } = 0;

        /// <summary>
        /// Logging settings.
        /// </summary>
        public LogSettings Logging { get; set; } = new LogSettings();
    }

    public class MediusSettings
    {
        /// <summary>
        /// Ip of the Medius Authentication server.
        /// </summary>
        public string Ip { get; set; } = "127.0.0.1";

        /// <summary>
        /// The port that the Authentication server is bound to.
        /// </summary>
        public short AuthPort { get; set; } = 10075;

        /// <summary>
        /// The port that the Lobby server is bound to.
        /// </summary>
        public short LobbyPort { get; set; } = 10078;

        /// <summary>
        /// The port that the Proxy server is bound to.
        /// </summary>
        public short ProxyPort { get; set; } = 10079;

        /// <summary>
        /// The port that the Dme server is bound to.
        /// </summary>
        public short DmePort { get; set; } = 10072;

        /// <summary>
        /// Key used to establish initial handshake with MPS.
        /// </summary>
        public PS2_RSA Key { get; set; } = DevelopmentRsaKeys.ForProcess.ToPS2();
    }
}
