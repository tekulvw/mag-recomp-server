using Org.BouncyCastle.Math;
using RT.Cryptography;
using Server.Common.Logging;
using System;
using System.Collections.Generic;
using System.Text;

namespace Server.UnivereInformation.Config
{
    public class ServerSettings
    {
        /// <summary>
        /// How many milliseconds before refreshing the config.
        /// </summary>
        public int RefreshConfigInterval = 5000;

        /// <summary>
        /// Port of the MUIS server.
        /// </summary>
        public int[] Ports { get; set; } = new int[] { 10071 };

        /// <summary>
        /// Key used to authenticate clients.
        /// </summary>
        public RsaKeyPair DefaultKey { get; set; } = DevelopmentRsaKeys.ForProcess;

        /// <summary>
        /// Whether or not to encrypt messages.
        /// </summary>
        public bool EncryptMessages { get; set; } = true;

        /// <summary>
        /// Universes.
        /// </summary>
        public Dictionary<int, UniverseInfo[]> Universes { get; set; } = new Dictionary<int, UniverseInfo[]>();

        /// <summary>
        /// Logging settings.
        /// </summary>
        public LogSettings Logging { get; set; } = new LogSettings();
    }

    public class UniverseInfo
    {
        public bool Enabled { get; set; } = true;
        public string Name { get; set; }
        public string Description { get; set; }
        public string Endpoint { get; set; }
        public string SvoURL { get; set; }
        public string ExtendedInfo { get; set; }
        public int Port { get; set; }
        public uint UniverseId { get; set; }
    }
}
