using RT.Cryptography;
using Microsoft.Extensions.Logging;
using Org.BouncyCastle.Math;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.Serialization;
using System.Text;
using RT.Models;
using RT.Common;
using Server.Common.Logging;

namespace Server.Medius.Config
{
    public class ServerSettings
    {
        /// <summary>
        /// How many milliseconds before refreshing the config.
        /// </summary>
        public int RefreshConfigInterval = 5000;

        /// <summary>Enable the isolated local MAPS adapter for the selected MAG application.</summary>
        public bool EnableMagPluginAdapter { get; set; } = false;

        private int _magApplicationId = 20230;
        /// <summary>Selected MAG compatibility profile. Retail 20231 explicitly reuses existing typed layouts pending validation.</summary>
        public int MagApplicationId
        {
            get => _magApplicationId;
            set
            {
                if (value != 20230 && value != 20231) throw new System.ArgumentOutOfRangeException(nameof(MagApplicationId));
                _magApplicationId = value;
            }
        }

        /// <summary>MAG MAS/MLS version selected from the maintained application profile; defaults to accepted prototype.</summary>
        public uint MagProtocolVersion { get; set; } = 1725;

        private string _magSvoUrl = string.Empty;
        /// <summary>Optional local statistics HTTPS base URL; empty preserves the prior universe reply.</summary>
        public string MagSvoUrl
        {
            get => _magSvoUrl;
            set
            {
                if (value == string.Empty) { _magSvoUrl = value; return; }
                if (value == null || value != value.Trim() || !value.EndsWith("/") ||
                    !Uri.TryCreate(value, UriKind.Absolute, out var uri) || uri.Scheme != Uri.UriSchemeHttps ||
                    !uri.IsLoopback || uri.UserInfo.Length != 0 || uri.Query.Length != 0 || uri.Fragment.Length != 0)
                    throw new ArgumentException("MAG statistics URL must be empty or a loopback HTTPS base URL ending in '/'.", nameof(MagSvoUrl));
                _magSvoUrl = value;
            }
        }


        /// <summary>Attach configured item/skill/progression catalogs to character records. Disable until retail catalogs are validated.</summary>
        public bool EnableMagCharacterCatalogs { get; set; } = true;

        /// <summary>Enable isolated MAS protocol handling; mutually exclusive with MAPS.</summary>
        public bool EnableMagMasAdapter { get; set; } = false;

        /// <summary>Accept only the captured local player development-auth profile.</summary>
        public bool EnableMagDevelopmentAuth { get; set; } = false;

        public bool EnableMagCharacterStorage { get; set; } = false;
        public bool EnableMagAccountLogin { get; set; } = false;
        public bool EnableMagCharacterData { get; set; } = false;
        /// <summary>Game type (retail NetJoinGameType value0) served by the attached gameplay process: retail0 GameType64/Sabotage, 2 GameType256/Domination (default), 3 FactionExercise/Suppression.</summary>
        public int MagGameType { get; set; } = 2;
        public int MagMinimumPlayers { get; set; } = 1;
        /// <summary>Opt-in local capacity: 1..64 for retail Sabotage/Suppression, otherwise 1..8. Rust must acknowledge its matching disk-backed admission policy.</summary>
        public int MagMaximumPlayers { get; set; } = 8;
        /// <summary>Optional served faction values0..2, derived from the attached match profile. Null preserves legacy availability for all three factions.</summary>
        public int[] MagAllowedFactions { get; set; } = null;
        public string MagCharacterStorePath { get; set; } = string.Empty;
        public string MagItemCatalogPath { get; set; } = string.Empty;
        public string MagLoadoutDefaultsPath { get; set; } = string.Empty;
        public string MagSkillCatalogPath { get; set; } = string.Empty;
        public string MagDevelopmentProgressionPath { get; set; } = string.Empty;

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
        /// Number of ticks per second.
        /// </summary>
        public int TickRate { get; set; } = 10;

        /// <summary>
        /// Port of the MAS server.
        /// </summary>
        public int MASPort { get; set; } = 10075;

        /// <summary>
        /// Port of the MLS server.
        /// </summary>
        public int MLSPort { get; set; } = 10078;

        /// <summary>
        /// Port of the MPS server.
        /// </summary>
        public int MPSPort { get; set; } = 10077;

        /// <summary>
        /// Ip address of the NAT server.
        /// </summary>
        public string NATIp { get; set; } = null;

        /// <summary>
        /// Port of the NAT server.
        /// </summary>
        public int NATPort { get; set; } = 10070;

        /// <summary>
        /// Root path of the medius file service directory.
        /// </summary>
        public string MediusFileServerRootPath { get; set; } = "files";

        /// <summary>
        /// Path to the plugins directory.
        /// </summary>
        public string PluginsPath { get; set; } = "plugins/";

        /// <summary>
        /// Key used to authenticate dme servers.
        /// </summary>
        public RsaKeyPair MPSKey { get; set; } = DevelopmentRsaKeys.ForProcess;

        /// <summary>
        /// Key used to authenticate clients.
        /// </summary>
        public RsaKeyPair DefaultKey { get; set; } = DevelopmentRsaKeys.ForProcess;

        /// <summary>
        /// Logging settings.
        /// </summary>
        public LogSettings Logging { get; set; } = new LogSettings();
    }

    public enum TextFilterContext
    {
        DEFAULT,
        ACCOUNT_NAME,
        CLAN_NAME,
        CLAN_MESSAGE,
        CHAT,
        GAME_NAME
    }
}
