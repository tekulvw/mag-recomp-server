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
        /// <summary>Portal ticket sign-in (MAG_PORTAL_TICKETS=required); null keeps the development identity policy.</summary>
        public static MagPortalTicketPolicy PortalTickets { get; private set; }
        public static LocalDmeHost LocalDme;
        public static MagWorldRegistry WorldRegistry;
        /// <summary>DME humans across every world (MAG_DME_UDP_PORT_END, MAG_DME_HUMAN_CAPACITY).</summary>
        public static MagDmePortBudget DmePortBudget { get; private set; } = MagDmePortBudget.Unbounded();
        public static IMagGameplayLobby GameplayLobby => (IMagGameplayLobby)WorldRegistry ?? LocalDme;
        public static MagLoadoutCatalog LoadoutCatalog { get; private set; }
        public static MagSkillCatalog SkillCatalog { get; private set; }
        public static MagProgressionCatalog ProgressionCatalog { get; private set; }
        public static MagTrainingCompletionCatalog TrainingCompletionCatalog { get; private set; }
        public static byte InitialSkillPoints { get; private set; }
        /// <summary>Optional operator starting level for new characters (MAG_STARTING_LEVEL).</summary>
        public static byte? StartingLevel { get; private set; }
        /// <summary>Optional operator minimum level reached on completing Training (MAG_TRAINING_LEVEL).</summary>
        public static byte? TrainingLevel { get; private set; }
        /// <summary>Optional operator floor for every character's respec points (MAG_RESPEC_POINTS_FLOOR).</summary>
        public static uint? RespecPointsFloor { get; private set; }
        /// <summary>Optional operator cap on every respec's fee (MAG_RESPEC_FEE_CAP).</summary>
        public static uint? RespecFeeCap { get; private set; }
        /// <summary>Hours an open match reward row may go without a reward before it can be compacted
        /// (MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS).</summary>
        public static TimeSpan OpenMatchRowTtl { get; private set; } = LocalCharacterStore.DefaultOpenMatchRowTtl;
        /// <summary>PostgreSQL behind the character store (MAG_CHARACTER_STORE=postgres); null keeps the JSON file.</summary>
        public static PostgresCharacterDatabase CharacterDatabase { get; private set; }
        /// <summary>How long PostgreSQL keeps settled reward receipts and match rows (MAG_REWARD_LEDGER_RETENTION_DAYS).</summary>
        public static TimeSpan LedgerRetention { get; private set; } = PostgresCharacterDatabase.DefaultLedgerRetention;
        public static LocalCharacterStore CreateCharacterStore() => ConfigureCharacterStore(CharacterDatabase != null
            ? LocalCharacterStore.ForPostgres(CharacterDatabase) : new LocalCharacterStore(Settings.MagCharacterStorePath));
        /// <summary>A JSON store at <paramref name="path"/> with the server's catalogs and policy (import and export).</summary>
        public static LocalCharacterStore CreateJsonCharacterStore(string path) => ConfigureCharacterStore(new LocalCharacterStore(path));
        static LocalCharacterStore ConfigureCharacterStore(LocalCharacterStore store)
        {
            store.TrainingCompletionCatalog = Settings.MagApplicationId == 20231 ? TrainingCompletionCatalog : null;
            store.LoadoutCatalog = Settings.EnableMagCharacterCatalogs || (Settings.MagApplicationId == 20231 && LoadoutCatalog?.ApplicationId == 20231) ? LoadoutCatalog : null;
            store.SkillCatalog = Settings.EnableMagCharacterCatalogs || (Settings.MagApplicationId == 20231 && SkillCatalog?.ApplicationId == 20231) ? SkillCatalog : null;
            store.ProgressionCatalog = Settings.MagApplicationId == 20231 || Settings.EnableMagCharacterCatalogs ? ProgressionCatalog : null;
            store.InitialSkillPoints = Settings.EnableMagCharacterCatalogs ? InitialSkillPoints : (byte)0;
            store.StartingLevel = StartingLevel; store.TrainingLevel = TrainingLevel; store.OpenMatchRowTtl = OpenMatchRowTtl;
            store.RespecPointsFloor = RespecPointsFloor; store.RespecFeeCap = RespecFeeCap;
            store.LedgerMaintenance = message => Logger.Info(message);
            return store;
        }
        public enum CharacterSchemaPolicy { Migrate, Require, None }
        /// <summary>
        /// Selects the character store from MAG_CHARACTER_STORE: json (default, the MagCharacterStorePath
        /// file) or postgres (MAG_CHARACTER_DB_SECRET_DIR, optional MAG_CHARACTER_DB_CA_FILE and
        /// MAG_CHARACTER_DB_READ_ONLY=1 for MAPS). Any other value, or postgres without its settings, fails.
        /// </summary>
        internal static void ConfigureCharacterBackend(CharacterSchemaPolicy policy)
        {
            string backend = Environment.GetEnvironmentVariable("MAG_CHARACTER_STORE");
            backend = string.IsNullOrWhiteSpace(backend) ? "json" : backend.Trim();
            if (backend == "json") { CharacterDatabase = null; return; }
            if (backend != "postgres") throw new InvalidDataException("MAG_CHARACTER_STORE must be json or postgres.");
            LedgerRetention = PostgresCharacterDatabase.ParseLedgerRetention(Environment.GetEnvironmentVariable("MAG_REWARD_LEDGER_RETENTION_DAYS"));
            string readOnlyValue = Environment.GetEnvironmentVariable("MAG_CHARACTER_DB_READ_ONLY");
            if (!string.IsNullOrEmpty(readOnlyValue) && readOnlyValue != "0" && readOnlyValue != "1")
                throw new InvalidDataException("MAG_CHARACTER_DB_READ_ONLY must be 0 or 1.");
            var database = new PostgresCharacterDatabase(PostgresCharacterDatabase.ConnectionStringFromSecretDirectory(
                Environment.GetEnvironmentVariable("MAG_CHARACTER_DB_SECRET_DIR"), Environment.GetEnvironmentVariable("MAG_CHARACTER_DB_CA_FILE")),
                readOnlyValue == "1");
            if (policy == CharacterSchemaPolicy.Migrate && !database.ReadOnly)
            {
                int applied = database.Migrate();
                if (applied > 0) Logger.Info($"Character database migrated: {applied} migration(s) applied.");
            }
            if (policy != CharacterSchemaPolicy.None) database.RequireCurrentSchema();
            CharacterDatabase = database;
        }
        static System.Threading.Timer _ledgerRetentionTimer;
        static int _ledgerPruning;
        /// <summary>Hourly PostgreSQL ledger retention in main Medius, off the reward path.</summary>
        static void StartLedgerRetention()
        {
            if (CharacterDatabase == null || CharacterDatabase.ReadOnly) return;
            _ledgerRetentionTimer = new System.Threading.Timer(_ => PruneLedgerOnce(), null, TimeSpan.FromMinutes(1), TimeSpan.FromHours(1));
        }
        static void PruneLedgerOnce()
        {
            if (Interlocked.Exchange(ref _ledgerPruning, 1) == 1) return;
            try
            {
                var result = CharacterDatabase.PruneLedger(LedgerRetention);
                if (result.Skipped || result.Rows > 0) Logger.Info(result.Describe());
            }
            catch (Exception e) when (e is IOException || e is InvalidDataException)
            { Logger.Warn($"Character store ledger prune deferred: {e.Message}"); }
            finally { Interlocked.Exchange(ref _ledgerPruning, 0); }
        }
        /// <summary>Fail fast on an open match row TTL outside whole hours from one hour to thirty days.</summary>
        public static TimeSpan ParseOpenMatchRowTtl(string value)
        {
            if (string.IsNullOrWhiteSpace(value)) return LocalCharacterStore.DefaultOpenMatchRowTtl;
            int minimum = (int)LocalCharacterStore.MinimumOpenMatchRowTtl.TotalHours, maximum = (int)LocalCharacterStore.MaximumOpenMatchRowTtl.TotalHours;
            if (!int.TryParse(value.Trim(), System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out int hours) ||
                hours < minimum || hours > maximum)
                throw new InvalidDataException($"MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS must be a whole number of hours between {minimum} and {maximum}.");
            return TimeSpan.FromHours(hours);
        }
        /// <summary>Fail fast on a respec point floor that is not a whole number of points from 0 to 4294967295.
        /// Unset or empty is null: balances stay as stored.</summary>
        public static uint? ParseRespecPointsFloor(string value) => ParseRespecPoints("MAG_RESPEC_POINTS_FLOOR", value);
        /// <summary>Fail fast on a respec fee cap that is not a whole number of points from 0 to 4294967295.
        /// Unset or empty is null: the authored fees apply.</summary>
        public static uint? ParseRespecFeeCap(string value) => ParseRespecPoints("MAG_RESPEC_FEE_CAP", value);
        static uint? ParseRespecPoints(string variable, string value)
        {
            if (string.IsNullOrWhiteSpace(value)) return null;
            if (!uint.TryParse(value.Trim(), System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out uint points))
                throw new InvalidDataException($"{variable} must be a whole number of respec points from 0 to 4294967295.");
            return points;
        }
        /// <summary>Fail fast on a starting level the progression catalog cannot represent.</summary>
        public static byte? ParseStartingLevel(string value, MagProgressionCatalog catalog) =>
            ParseOperatorLevel("MAG_STARTING_LEVEL", value, catalog, catalog?.InitialLevel ?? 1);
        /// <summary>
        /// Fail fast on a Training level that either progression table cannot represent: the setting
        /// is applied with the Training-completion catalog's table, so that catalog is required too.
        /// </summary>
        public static byte? ParseTrainingLevel(string value, MagProgressionCatalog catalog, MagTrainingCompletionCatalog training)
        {
            byte? level = ParseOperatorLevel("MAG_TRAINING_LEVEL", value, catalog, 1);
            if (level == null) return null;
            if (training == null) throw new InvalidDataException("MAG_TRAINING_LEVEL requires a Training-completion catalog.");
            if (level > training.Progression.MaximumLevel)
                throw new InvalidDataException($"MAG_TRAINING_LEVEL must be between 1 and {training.Progression.MaximumLevel}.");
            return level;
        }
        static byte? ParseOperatorLevel(string variable, string value, MagProgressionCatalog catalog, byte minimum)
        {
            if (string.IsNullOrWhiteSpace(value)) return null;
            if (!byte.TryParse(value.Trim(), out byte level)) throw new InvalidDataException($"{variable} must be a level number.");
            if (catalog == null) throw new InvalidDataException($"{variable} requires a progression catalog.");
            if (level < minimum || level > catalog.MaximumLevel)
                throw new InvalidDataException($"{variable} must be between {minimum} and {catalog.MaximumLevel}.");
            return level;
        }
        internal static void LoadCharacterCatalog()
        {
            if (Settings.MagProtocolVersion == 1793 ||
                !string.IsNullOrEmpty(Environment.GetEnvironmentVariable(MagRetail212BootstrapProfile.ProfileVariable)))
            {
                var bootstrap = MagRetail212BootstrapProfile.Load(Settings.MagApplicationId,
                    Settings.MagProtocolVersion, Environment.GetEnvironmentVariable);
                LoadoutCatalog = bootstrap.Armory;
                ProgressionCatalog = bootstrap.Progression;
                InitialSkillPoints = ProgressionCatalog.InitialSkillPoints;
                StartingLevel = 1;
                SkillCatalog = null;
                TrainingCompletionCatalog = null;
                TrainingLevel = null;
                RespecPointsFloor = null;
                RespecFeeCap = null;
                OpenMatchRowTtl = ParseOpenMatchRowTtl(Environment.GetEnvironmentVariable("MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS"));
                return;
            }
            string trainingCatalog = Environment.GetEnvironmentVariable("MAG_TRAINING_COMPLETION_CATALOG");
            TrainingCompletionCatalog = string.IsNullOrEmpty(trainingCatalog) ? null : MagTrainingCompletionCatalog.Load(trainingCatalog);
            string progressionCatalog = Environment.GetEnvironmentVariable("MAG_PROGRESSION_CATALOG");
            if (!string.IsNullOrEmpty(progressionCatalog) && !Path.IsPathFullyQualified(progressionCatalog)) throw new InvalidDataException("MAG progression catalog path must be absolute.");
            ProgressionCatalog = string.IsNullOrEmpty(progressionCatalog) ? null : MagProgressionCatalog.Load(progressionCatalog, Settings.MagApplicationId);
            StartingLevel = ParseStartingLevel(Environment.GetEnvironmentVariable("MAG_STARTING_LEVEL"), ProgressionCatalog);
            TrainingLevel = ParseTrainingLevel(Environment.GetEnvironmentVariable("MAG_TRAINING_LEVEL"), ProgressionCatalog, TrainingCompletionCatalog);
            OpenMatchRowTtl = ParseOpenMatchRowTtl(Environment.GetEnvironmentVariable("MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS"));
            RespecPointsFloor = ParseRespecPointsFloor(Environment.GetEnvironmentVariable("MAG_RESPEC_POINTS_FLOOR"));
            RespecFeeCap = ParseRespecFeeCap(Environment.GetEnvironmentVariable("MAG_RESPEC_FEE_CAP"));
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
                    if (WorldRegistry != null) await WorldRegistry.TickAsync();
                    else if (LocalDme != null) await LocalDme.TickAsync();

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

                if (WorldRegistry != null) await WorldRegistry.DisposeAsync();
                else if (LocalDme != null) await LocalDme.DisposeAsync();
                await AuthenticationServer.Stop();
                await LobbyServer.Stop();
                await ProxyServer.Stop();
            }
        }

        // Each medius container (main/maps) publishes its account leases for mag-admin in the same Pod.
        static void PublishActiveAccounts()
        {
            string directory = Environment.GetEnvironmentVariable("MAG_ACTIVE_ACCOUNTS_DIR") ?? "/tmp";
            if (!Path.IsPathFullyQualified(directory) || !Directory.Exists(directory)) return;
            string role = new DirectoryInfo(CONFIG_DIRECTIORY).Name.ToLowerInvariant();
            if (role.Length == 0 || role.Length > 32 || !role.All(c => (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '-')) role = "default";
            MagLoginTickets.PublishActiveAccountsTo(Path.Combine(directory, MagActiveAccounts.FilePrefix + role + ".json"), DateTime.UtcNow);
        }

        /// <summary>Internal admin API (docs/admin-cli.md); off unless both MAG_ADMIN_API_* variables are set.</summary>
        public static MagAdminApi AdminApi { get; private set; }

        /// <summary>The hosts an operator kill searches: every configured world, or the one local world.</summary>
        static IReadOnlyList<MagAdminGameplayHost> AdminGameplayHosts() =>
            WorldRegistry?.AdminGameplayHosts() ??
            (LocalDme != null ? new[] { new MagAdminGameplayHost("gameplay", LocalDme) } : Array.Empty<MagAdminGameplayHost>());

        /// <summary>The in-memory voice mutes and bans the DME relay checks per talker; only main Medius
        /// with the local DME and MAG_VOICE_RELAY=1 loads them.</summary>
        static void StartVoiceSanctions()
        {
            if (Settings.EnableMagPluginAdapter || Environment.GetEnvironmentVariable("MAG_LOCAL_DME") != "1" ||
                !Server.Dme.VoiceRelayOptions.Current.Enabled) return;
            var cache = new VoiceSanctionCache(CreateCharacterStore(), log: message => Logger.Info(message));
            cache.Start();
            VoiceSanctionCache.Current = cache;
        }

        static void StartAdminApi()
        {
            var settings = MagAdminApiSettings.FromEnvironment();
            if (settings == null) return;
            MagAdminApiSettings.RequireMainRole(Settings);
            string storePath = Settings.MagCharacterStorePath, markers = MagAdminOperations.MarkerDirectory();
            // The status reads the registry or local world when asked, so it follows their start.
            AdminApi = new MagAdminApi(settings.Listen, settings.Secret, CreateCharacterStore(),
                MagAdminOperations.BackupDirectory(storePath), markers, () => DateTime.UtcNow, message => Logger.Info(message),
                DmePortBudget, now => MagServerStatus.Capture(WorldRegistry, LocalDme, DmePortBudget, markers, now),
                AdminGameplayHosts, () => VoiceSanctionCache.Current?.RequestRefresh());
            AdminApi.Start();
            Logger.Info($"Admin API listening on {AdminApi.LocalEndPoint}.");
        }

        static async Task StartServerAsync()
        {
            int waitMs = sleepMS;
            LoadCharacterCatalog();
            ConfigureCharacterBackend(CharacterSchemaPolicy.Migrate);
            if (CharacterDatabase != null)
            {
                Logger.Info($"Character store: postgres {CharacterDatabase.Target} (schema {CharacterDatabase.LatestSchemaVersion}" +
                    (CharacterDatabase.ReadOnly ? ", read-only)" : $", ledger retention {LedgerRetention.TotalDays:0} days)"));
                if (!string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable("MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS")))
                    Logger.Info("MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS applies only to the JSON character store; ignored.");
                StartLedgerRetention();
            }
            PortalTickets = MagPortalTicketPolicy.FromSettings(MagPortalTicketSettings.FromEnvironment());
            if (PortalTickets != null)
                Logger.Info($"Portal tickets required: audience={PortalTickets.Settings.Audience} keys={string.Join(",", PortalTickets.Settings.Keys.Keys.OrderBy(id => id))}");
            PublishActiveAccounts();
            DmePortBudget = MagDmePortBudget.FromEnvironment();
            StartVoiceSanctions();
            StartAdminApi();

            string registryPath=Environment.GetEnvironmentVariable("MAG_WORLD_REGISTRY");
            if (registryPath!=null && Environment.GetEnvironmentVariable("MAG_LOCAL_DME")!="1")
                throw new InvalidDataException("MAG_WORLD_REGISTRY requires MAG_LOCAL_DME=1.");
            if (Environment.GetEnvironmentVariable("MAG_LOCAL_DME") == "1")
            {
                if(registryPath!=null) {
                    var registryConfig=MagWorldRegistryConfig.Load(registryPath);
                    if(Settings.MagApplicationId!=registryConfig.ApplicationId) throw new InvalidDataException("Registry application must match the configured MAG lobby application.");
                    WorldRegistry=new MagWorldRegistry(registryConfig,CreateCharacterStore(),registryPath,DmePortBudget);
                    await WorldRegistry.StartAsync();
                } else {
                    LocalDme = new LocalDmeHost(new GameplayBridge(Environment.GetEnvironmentVariable("MAG_GAMEPLAY_HOST") ?? "mag-gameplay",
                        int.Parse(Environment.GetEnvironmentVariable("MAG_GAMEPLAY_PORT") ?? "31000")), CreateCharacterStore(), DmePortBudget);
                    await LocalDme.StartAsync();
                }
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
            // Operator maintenance runs offline against the store; it starts no listeners.
            if (args.Length > 0 && args[0] == "admin")
            {
                Environment.ExitCode = MagAdminCli.RunCommandLine(args.Skip(1).ToArray());
                return;
            }

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
