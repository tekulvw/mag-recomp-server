using DotNetty.Common.Internal.Logging;
using RT.Cryptography;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Console;
using Microsoft.Extensions.Options;
using Newtonsoft.Json;
using Org.BouncyCastle.Math;
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using RT.Models;
using Server.Medius.Models;
using Server.Database;
using Server.Medius.Config;
using NReco.Logging.File;
using Server.Common.Logging;
using Server.Plugins;
using System.Net.NetworkInformation;
using Server.Common;
using Haukcode.HighResolutionTimer;
using System.Text.RegularExpressions;

namespace Server.Medius
{
    public class Program
    {
        private static string CONFIG_DIRECTIORY = "./";
        public static string CONFIG_FILE => Path.Combine(CONFIG_DIRECTIORY, "medius.json");
        public static string DB_CONFIG_FILE => Path.Combine(CONFIG_DIRECTIORY, "db.config.json");
        public static string DB_SIM_FILE => Path.Combine(CONFIG_DIRECTIORY, "simulated.db");

        public static RSA_KEY GlobalAuthPublic = null;

        public static ServerSettings Settings = new ServerSettings();
        public static DbController Database = null;

        public static IPAddress SERVER_IP = IPAddress.Loopback;

        public static MediusManager Manager = new MediusManager();
        public static MagLoginRegistry MagLoginTickets = new MagLoginRegistry();
        public static LocalDmeHost LocalDme;
        public static MagLoadoutCatalog LoadoutCatalog { get; private set; }
        public static MagSkillCatalog SkillCatalog { get; private set; }
        public static MagProgressionCatalog ProgressionCatalog { get; private set; }
        public static MagTrainingCompletionCatalog TrainingCompletionCatalog { get; private set; }
        public static byte InitialSkillPoints { get; private set; }
        public static LocalCharacterStore CreateCharacterStore() => new LocalCharacterStore(Settings.MagCharacterStorePath)
        { TrainingCompletionCatalog = Settings.MagApplicationId == 20231 ? TrainingCompletionCatalog : null,
            LoadoutCatalog = Settings.EnableMagCharacterCatalogs || (Settings.MagApplicationId == 20231 && LoadoutCatalog?.ApplicationId == 20231) ? LoadoutCatalog : null,
            SkillCatalog = Settings.EnableMagCharacterCatalogs || (Settings.MagApplicationId == 20231 && SkillCatalog?.ApplicationId == 20231) ? SkillCatalog : null,
            ProgressionCatalog = Settings.MagApplicationId == 20231 || Settings.EnableMagCharacterCatalogs ? ProgressionCatalog : null,
            InitialSkillPoints = Settings.EnableMagCharacterCatalogs ? InitialSkillPoints : (byte)0 };
        private static void LoadCharacterCatalog()
        {
            string trainingCatalog = Environment.GetEnvironmentVariable("MAG_TRAINING_COMPLETION_CATALOG");
            TrainingCompletionCatalog = string.IsNullOrEmpty(trainingCatalog) ? null : MagTrainingCompletionCatalog.Load(trainingCatalog);
            string progressionCatalog = Environment.GetEnvironmentVariable("MAG_PROGRESSION_CATALOG");
            if (!string.IsNullOrEmpty(progressionCatalog) && !Path.IsPathFullyQualified(progressionCatalog)) throw new InvalidDataException("MAG progression catalog path must be absolute.");
            ProgressionCatalog = string.IsNullOrEmpty(progressionCatalog) ? null : MagProgressionCatalog.Load(progressionCatalog, Settings.MagApplicationId);
            // Retail purchases are independent of prototype kit/grant activation.
            string retailSkills = Environment.GetEnvironmentVariable("MAG_RETAIL_SKILL_CATALOG");
            if (!string.IsNullOrEmpty(retailSkills) && (Settings.MagApplicationId != 20231 || !Path.IsPathFullyQualified(retailSkills)))
                throw new InvalidDataException("Retail skill catalog requires retail application and an absolute path.");
            SkillCatalog = string.IsNullOrEmpty(retailSkills) ? null : MagSkillCatalog.LoadForApplication(retailSkills, 20231);
            string retailArmory = Environment.GetEnvironmentVariable("MAG_RETAIL_ARMORY_CATALOG");
            if (!string.IsNullOrEmpty(retailArmory))
            {
                if (Settings.MagApplicationId != 20231 || !Path.IsPathFullyQualified(retailArmory))
                    throw new InvalidDataException("Retail Armory requires retail application and an absolute catalog path.");
                LoadoutCatalog = MagLoadoutCatalog.LoadRetail(retailArmory);
                InitialSkillPoints = 0;
                return;
            }
            string items = Environment.GetEnvironmentVariable("MAG_ITEM_CATALOG") ?? Settings.MagItemCatalogPath;
            string defaults = Environment.GetEnvironmentVariable("MAG_LOADOUT_DEFAULTS") ?? Settings.MagLoadoutDefaultsPath;
            if (string.IsNullOrEmpty(items) && string.IsNullOrEmpty(defaults)) { LoadoutCatalog = null; InitialSkillPoints = 0; return; }
            if (!Path.IsPathFullyQualified(items) || !Path.IsPathFullyQualified(defaults))
                throw new InvalidDataException("MAG item and loadout catalog paths must both be absolute.");
            LoadoutCatalog = MagLoadoutCatalog.Load(items, defaults);
            string skills = Environment.GetEnvironmentVariable("MAG_SKILL_CATALOG") ?? Settings.MagSkillCatalogPath;
            string progression = Environment.GetEnvironmentVariable("MAG_DEVELOPMENT_PROGRESSION") ?? Settings.MagDevelopmentProgressionPath;
            if (!string.IsNullOrEmpty(skills) && !Path.IsPathFullyQualified(skills))
                throw new InvalidDataException("MAG skill catalog path must be absolute.");
            if (Settings.MagApplicationId != 20231) SkillCatalog = string.IsNullOrEmpty(skills) ? null : MagSkillCatalog.Load(skills);
            InitialSkillPoints = 0;
            if (Settings.MagApplicationId != 20231 && !string.IsNullOrEmpty(progression))
            {
                if (SkillCatalog == null || !Path.IsPathFullyQualified(progression))
                    throw new InvalidDataException("Development progression requires a skill catalog and an absolute policy path.");
                using var policy = JsonDocument.Parse(File.ReadAllBytes(progression));
                var root = policy.RootElement;
                if (root.GetProperty("schema_version").GetInt32() != 1)
                    throw new InvalidDataException("Unsupported development progression policy.");
                InitialSkillPoints = root.GetProperty("initial_skill_points").GetByte();
            }
        }

        public static PluginsManager Plugins = null;

        public static MAS AuthenticationServer = new MAS();
        public static MLS LobbyServer = new MLS();
        public static MPS ProxyServer = new MPS();

        public static int TickMS => 1000 / (Settings?.TickRate ?? 10);

        private static Dictionary<int, AppSettings> _appSettings = new Dictionary<int, AppSettings>();
        private static AppSettings _defaultAppSettings = new AppSettings(0);
        private static FileLoggerProvider _fileLogger = null;
        private static ulong _sessionKeyCounter = 0;
        private static int sleepMS = 0;
        private static readonly object _sessionKeyCounterLock = (object)_sessionKeyCounter;
        private static DateTime? _lastSuccessfulDbAuth = null;
        private static DateTime _lastConfigRefresh = Utils.GetHighPrecisionUtcTime();
        private static DateTime _lastComponentLog = Utils.GetHighPrecisionUtcTime();
        private static bool _hasPurgedAccountStatuses = false;

        private static int _ticks = 0;
        private static Stopwatch _sw = new Stopwatch();
        private static HighResolutionTimer _timer;

        static readonly IInternalLogger Logger = InternalLoggerFactory.GetInstance<Program>();

        static async Task TickAsync()
        {
            try
            {
#if DEBUG || RELEASE
                if (!_sw.IsRunning)
                    _sw.Start();
#endif

#if DEBUG || RELEASE
                ++_ticks;
                if (_sw.Elapsed.TotalSeconds > 5f)
                {
                    // 
                    _sw.Stop();
                    float tps = _ticks / (float)_sw.Elapsed.TotalSeconds;
                    float error = MathF.Abs(Settings.TickRate - tps) / Settings.TickRate;

                    if (error > 0.1f)
                        Logger.Error($"Average TPS: {tps} is {error * 100}% off of target {Settings.TickRate}");

                    //var dt = DateTime.UtcNow - Utils.GetHighPrecisionUtcTime();
                    //if (Math.Abs(dt.TotalMilliseconds) > 50)
                    //    Logger.Error($"System clock and local clock are out of sync! delta ms: {dt.TotalMilliseconds}");

                    _sw.Restart();
                    _ticks = 0;
                }
#endif

                // Attempt to authenticate with the db middleware
                // We do this every 24 hours to get a fresh new token
                if (!await Database.AmIAuthenticated() || (_lastSuccessfulDbAuth == null || (Utils.GetHighPrecisionUtcTime() - _lastSuccessfulDbAuth.Value).TotalHours > 24))
                {
                    if (!await Database.Authenticate())
                    {
                        // Log and exit when unable to authenticate
                        Logger.Error("Unable to authenticate with the db middleware server");
                        await Task.Delay(1000); // delay loop to give time before next authentication request
                        return;
                    }
                    else
                    {
                        _lastSuccessfulDbAuth = Utils.GetHighPrecisionUtcTime();
                        Logger.Info("Successfully authenticated with the db middleware server");

                        // pass to manager
                        await Manager.OnDatabaseAuthenticated();

                        // refresh app settings
                        await RefreshAppSettings();

#if !DEBUG
                        if (!_hasPurgedAccountStatuses)
                        {
                            _hasPurgedAccountStatuses = await Database.ClearAccountStatuses();
                            await Database.ClearActiveGames();
                        }
#endif
                    }
                }

                // Tick
                await Task.WhenAll(Database.Tick(), AuthenticationServer.Tick(), LobbyServer.Tick(), ProxyServer.Tick());

                // Tick manager
                await Manager.Tick();
                    if (LocalDme != null) await LocalDme.TickAsync();

                // Tick plugins
                await Plugins.Tick();

                // 
                if ((Utils.GetHighPrecisionUtcTime() - _lastComponentLog).TotalSeconds > 15f)
                {
                    AuthenticationServer.Log();
                    LobbyServer.Log();
                    ProxyServer.Log();
                    _lastComponentLog = Utils.GetHighPrecisionUtcTime();
                }

                // Reload config
                if ((Utils.GetHighPrecisionUtcTime() - _lastConfigRefresh).TotalMilliseconds > Settings.RefreshConfigInterval)
                {
                    RefreshConfig();
                    _lastConfigRefresh = Utils.GetHighPrecisionUtcTime();
                }
            }
            catch (Exception ex)
            {
                Logger.Error(ex);

                if (LocalDme != null) await LocalDme.DisposeAsync();
                await AuthenticationServer.Stop();
                await LobbyServer.Stop();
                await ProxyServer.Stop();
            }
        }

        static async Task StartServerAsync()
        {
            int waitMs = sleepMS;
            LoadCharacterCatalog();

            if (Environment.GetEnvironmentVariable("MAG_LOCAL_DME") == "1")
            {
                LocalDme = new LocalDmeHost(new GameplayBridge(Environment.GetEnvironmentVariable("MAG_GAMEPLAY_HOST") ?? "mag-gameplay",
                    int.Parse(Environment.GetEnvironmentVariable("MAG_GAMEPLAY_PORT") ?? "31000")), CreateCharacterStore());
                await LocalDme.StartAsync();
            }
            Logger.Info("Starting medius components...");

            Logger.Info($"Starting MAS on port {AuthenticationServer.Port}.");
            AuthenticationServer.Start();
            Logger.Info($"MAS started.");

            Logger.Info($"Starting MLS on port {LobbyServer.Port}.");
            LobbyServer.Start();
            Logger.Info($"MLS started.");

            Logger.Info($"Starting MPS on port {ProxyServer.Port}.");
            ProxyServer.Start();
            Logger.Info($"MPS started.");

            // 
            Logger.Info("Started.");

            // start timer
            _timer = new HighResolutionTimer();
            _timer.SetPeriod(waitMs);
            _timer.Start();

            // iterate
            while (true)
            {
                // handle tick rate change
                if (sleepMS != waitMs)
                {
                    waitMs = sleepMS;
                    _timer.Stop();
                    _timer.SetPeriod(waitMs);
                    _timer.Start();
                }

                // tick
                await TickAsync();

                // wait for next tick
                _timer.WaitForTrigger();
            }
        }

        public static async Task Main(string[] args)
        {
            // get path to config directory from first argument
            if (args.Length > 0)
                CONFIG_DIRECTIORY = args[0];

            // 
            Database ??= new DbController(DB_CONFIG_FILE, DB_SIM_FILE);

            // 
            Initialize();

            // Add file logger if path is valid
            if (new FileInfo(LogSettings.Singleton.LogPath)?.Directory?.Exists ?? false)
            {
                var loggingOptions = new FileLoggerOptions()
                {
                    Append = false,
                    FileSizeLimitBytes = LogSettings.Singleton.RollingFileSize,
                    MaxRollingFiles = LogSettings.Singleton.RollingFileCount
                };
                InternalLoggerFactory.DefaultFactory.AddProvider(_fileLogger = new FileLoggerProvider(LogSettings.Singleton.LogPath, loggingOptions));
                _fileLogger.MinLevel = Settings.Logging.LogLevel;
            }

            // Optionally add console logger (always enabled when debugging)
#if DEBUG
            InternalLoggerFactory.DefaultFactory = LoggerFactory.Create(builder =>
            {
                builder
                    .AddFilter(level => level >= LogSettings.Singleton.LogLevel)
                    .AddConsole();
            });
            //InternalLoggerFactory.DefaultFactory.AddProvider(new ConsoleLoggerProvider((s, level) => level >= LogSettings.Singleton.LogLevel, true));
#else
            if (Settings.Logging.LogToConsole)
            {
                InternalLoggerFactory.DefaultFactory = LoggerFactory.Create(builder =>
                {
                    builder
                        .AddFilter(level => level >= LogSettings.Singleton.LogLevel)
                        .AddConsole();
                });
                //InternalLoggerFactory.DefaultFactory.AddProvider(new ConsoleLoggerProvider((s, level) => level >= LogSettings.Singleton.LogLevel, true));
            }
#endif

            // Initialize plugins
            Plugins = new PluginsManager(Settings.PluginsPath);

            // 
            await StartServerAsync();
        }

        static void Initialize()
        {
            RefreshServerIp();
            RefreshConfig();
        }


        /// <summary>
        /// 
        /// </summary>
        static void RefreshConfig()
        {
            var usePublicIp = Settings.UsePublicIp;

            // 
            var serializerSettings = new JsonSerializerSettings()
            {
                MissingMemberHandling = MissingMemberHandling.Ignore,
            };

            // Load settings
            if (File.Exists(CONFIG_FILE))
            {
                // Populate existing object
                JsonConvert.PopulateObject(File.ReadAllText(CONFIG_FILE), Settings, serializerSettings);
            }
            else
            {
                // set default nat ip
                if (string.IsNullOrEmpty(Settings.NATIp))
                    Settings.NATIp = GetIpAddress()?.ToString() ?? "localhost";

                // Save defaults
                File.WriteAllText(CONFIG_FILE, JsonConvert.SerializeObject(Settings, Formatting.Indented));
            }

            // Set LogSettings singleton
            LogSettings.Singleton = Settings.Logging;

            // Determine server ip
            if (usePublicIp != Settings.UsePublicIp)
                RefreshServerIp();

            // Update NAT Ip with server ip if null
            if (string.IsNullOrEmpty(Settings.NATIp))
                Settings.NATIp = SERVER_IP.ToString();

            // Update file logger min level
            if (_fileLogger != null)
                _fileLogger.MinLevel = Settings.Logging.LogLevel;

            // Update default rsa key
            Pipeline.Attribute.ScertClientAttribute.DefaultRsaAuthKey = Settings.DefaultKey;

            if (Settings.DefaultKey != null)
                GlobalAuthPublic = new RSA_KEY(Settings.DefaultKey.N.ToByteArrayUnsigned().Reverse().ToArray());

            //
            _ = RefreshAppSettings();

            // Load tick time into sleep ms for main loop
            sleepMS = TickMS;
        }

        static IPAddress GetIpAddress()
        {
            if (!Settings.UsePublicIp)
            {
                return Utils.GetLocalIPAddress();
            }
            else
            {
                if (string.IsNullOrWhiteSpace(Settings.PublicIpOverride))
                    return IPAddress.Parse(Utils.GetPublicIPAddress());
                else
                    return IPAddress.Parse(Settings.PublicIpOverride);
            }
        }

        static async Task RefreshAppSettings()
        {
            try
            {
                if (!await Database.AmIAuthenticated())
                    return;

                // get supported app ids
                var appIdGroups = await Database.GetAppIds();
                if (appIdGroups == null)
                    return;

                // get settings
                foreach (var appIdGroup in appIdGroups)
                {
                    foreach (var appId in appIdGroup.AppIds)
                    {
                        var settings = await Database.GetServerSettings(appId);
                        if (settings != null)
                        {
                            if (_appSettings.TryGetValue(appId, out var appSettings))
                            {
                                appSettings.SetSettings(settings);
                            }
                            else
                            {
                                appSettings = new AppSettings(appId);
                                appSettings.SetSettings(settings);
                                _appSettings.Add(appId, appSettings);

                                // we also want to send this back to the server since this is new locally
                                // and there might be new setting fields that aren't yet on the db
                                await Database.SetServerSettings(appId, appSettings.GetSettings());
                            }
                        }
                    }
                }

                // get locations
                var locations = await Database.GetLocations();
                var channels = await Database.GetChannels();

                // 

                // add new channels
                foreach (var channel in channels)
                {
                    if (Manager.GetChannelByChannelId(channel.Id, channel.AppId) == null)
                    {
                        Manager.AddChannel(new Channel()
                        {
                            Id = channel.Id,
                            Name = channel.Name,
                            ApplicationId = channel.AppId,
                            MaxPlayers = channel.MaxPlayers,
                            GenericField1 = (uint)channel.GenericField1,
                            GenericField2 = (uint)channel.GenericField2,
                            GenericField3 = (uint)channel.GenericField3,
                            GenericField4 = (uint)channel.GenericField4,
                            GenericFieldLevel = (RT.Common.MediusWorldGenericFieldLevelType)channel.GenericFieldFilter,
                            Type = ChannelType.Lobby
                        });
                    }
                }
            }
            catch (Exception ex)
            {
                Logger.Error(ex);
            }
        }

        static void RefreshServerIp()
        {
            SERVER_IP = GetIpAddress();
        }

        public static string GenerateSessionKey()
        {
            lock (_sessionKeyCounterLock)
            {
                return (++_sessionKeyCounter).ToString();
            }
        }

        private static string GetTextFilterRegexExpression(int appId, TextFilterContext context)
        {
            var appSettings = GetAppSettingsOrDefault(appId);
            string regex = null;

            switch (context)
            {
                case TextFilterContext.ACCOUNT_NAME: regex = appSettings.TextFilterAccountName; break;
                case TextFilterContext.CHAT: regex = appSettings.TextFilterChat; break;
                case TextFilterContext.CLAN_MESSAGE: regex = appSettings.TextFilterClanMessage; break;
                case TextFilterContext.CLAN_NAME: regex = appSettings.TextFilterClanName; break;
                case TextFilterContext.DEFAULT: regex = appSettings.TextFilterDefault; break;
                case TextFilterContext.GAME_NAME: regex = appSettings.TextFilterGameName; break;
            }

            if (String.IsNullOrEmpty(regex))
                return appSettings.TextFilterDefault;

            return regex;
        }

        public static bool PassTextFilter(int appId, TextFilterContext context, string text)
        {
            var rExp = GetTextFilterRegexExpression(appId, context);
            if (String.IsNullOrEmpty(rExp))
                return true;

            Regex r = new Regex(rExp, RegexOptions.IgnoreCase | RegexOptions.Multiline);
            return !r.IsMatch(text);
        }

        public static string FilterTextFilter(int appId, TextFilterContext context, string text)
        {
            var rExp = GetTextFilterRegexExpression(appId, context);
            if (String.IsNullOrEmpty(rExp))
                return text;

            Regex r = new Regex(rExp, RegexOptions.IgnoreCase | RegexOptions.Multiline);
            return r.Replace(text, "");
        }

        public static string? GetFileSystemPath(int appId, string filename)
        {
            if (!GetAppSettingsOrDefault(appId).EnableMediusFileServices)
                return null;
            if (String.IsNullOrEmpty(Settings.MediusFileServerRootPath))
                return null;
            if (String.IsNullOrEmpty(filename))
                return null;

            var rootPath = Path.GetFullPath(Settings.MediusFileServerRootPath);
            var path = Path.GetFullPath(Path.Combine(Settings.MediusFileServerRootPath, filename, appId.ToString()));

            // prevent filename from moving up directories
            if (!path.StartsWith(rootPath))
                return null;

            return path;
        }

        public static AppSettings GetAppSettingsOrDefault(int appId)
        {
            if (_appSettings.TryGetValue(appId, out var appSettings))
                return appSettings;

            return _defaultAppSettings;
        }
    }
}
