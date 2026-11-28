using System.Security.Cryptography;
using System.Text.Json.Nodes;
using System.Reflection;
using RT.Models.MAG;
using Server.Medius;

// Invented catalogs; target identity metadata tests profile binding, not native acceptance.
internal static class Bootstrap212Checks
{
    private static void Check(bool value, string message) {if (!value) throw new Exception(message);}
    private static void Reject(Action action)
    {try {action();} catch (InvalidDataException) {return;} throw new Exception("Invalid bootstrap configuration accepted");}
    private const string Engine = "705a6ec6a25458b2401b869e75e2904597304e2c06d9534a171a1dcd54aca2b3";
    private static JsonObject Sources() => new() {
        ["headers"] = "9845c9c024dddbe035578d0afe09fe3022b2d19a7969a89dd0701f8a8e391326",
        ["classes"] = "b6c722ac6acb53710b470b96e394942a03c223a08c0703c42b2d7024d6562325",
        ["equipment"] = "e187f6989d363aef7f782d7d23ef1e38fd94aa338c449b390836fb2137e9d53d"};
    private static JsonObject Identity() => new() {
        ["profile"] = "retail-v2.12-bootstrap", ["protocol_version"] = 1793,
        ["engine_sha256"] = Engine, ["source_sha256"] = Sources()};

    public static void Run()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-bootstrap212-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            string armoryPath = Path.Combine(directory, "retail-v212-bootstrap-armory.json");
            string progressionPath = Path.Combine(directory, "retail-v212-bootstrap-progression.json");
            string profilePath = Path.Combine(directory, "retail-v212-bootstrap-profile.json");
            JsonArray Kit(bool populated) {
                var items = new JsonArray();
                for (int i = 0; i < 22; i++) items.Add(i == 0 && populated ? JsonValue.Create(0) : null);
                return items;
            }
            var factions = new JsonArray();
            for (int f = 0; f < 3; f++)
            {
                var kits = new JsonArray();
                for (int i = 0; i < 5; i++) kits.Add(new JsonObject {["update_id"] = 1, ["items"] = Kit(i < 3)});
                factions.Add(new JsonObject {["faction"] = f, ["loadouts"] = kits,
                    ["available_item_bits"] = new JsonArray(0), ["skill_ids"] = new JsonArray()});
            }
            var armory = Identity(); armory["schema_version"] = 1; armory["application_id"] = 20231;
            armory["items"] = new JsonArray(new JsonObject {
                ["wire_id"] = 0, ["available_bit"] = 0, ["authored_ref"] = "Synthetic/Primary",
                ["name"] = "synthetic-primary", ["factions"] = new JsonArray(0,1,2), ["primary_ammo"] = 7});
            armory["factions"] = factions;
            void WriteArmory() => File.WriteAllText(armoryPath, armory.ToJsonString());
            WriteArmory();
            TrainingLevelChecks.Catalog(progressionPath, 3);
            var progression = JsonNode.Parse(File.ReadAllText(progressionPath)).AsObject();
            progression.Remove("training_completion");
            foreach (var entry in Identity()) progression[entry.Key] = entry.Value.DeepClone();
            File.WriteAllText(progressionPath, progression.ToJsonString());
            string Hash(string path) => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))).ToLowerInvariant();
            var manifest = Identity(); manifest["format"] = "mag-retail-v212-bootstrap-v1"; manifest["application_id"] = 20231;
            manifest["policy"] = new JsonObject {["initial_level"] = 1, ["additional_skill_points"] = 0,
                ["additional_leadership_points"] = 0, ["authored_kits"] = 3, ["wire_loadouts"] = 5,
                ["update_id"] = 1, ["skill_grants"] = "none", ["training_rewards"] = false, ["gameplay_worlds"] = false};
            void WriteManifest() {
                manifest["catalogs"] = new JsonObject {
                    ["armory"] = new JsonObject {["file"] = Path.GetFileName(armoryPath), ["sha256"] = Hash(armoryPath)},
                    ["progression"] = new JsonObject {["file"] = Path.GetFileName(progressionPath), ["sha256"] = Hash(progressionPath)}};
                File.WriteAllText(profilePath, manifest.ToJsonString());
            }
            WriteManifest();
            var env = new Dictionary<string, string> {
                ["MAG_CHARACTER_BOOTSTRAP_PROFILE"] = profilePath, ["MAG_RETAIL_ARMORY_CATALOG"] = armoryPath,
                ["MAG_PROGRESSION_CATALOG"] = progressionPath, ["MAG_LOCAL_DME"] = "0"};
            MagRetail212BootstrapProfile Load(int app = 20231, uint protocol = 1793) =>
                MagRetail212BootstrapProfile.Load(app, protocol, name => env.GetValueOrDefault(name));
            var result = Load();
            Check(result.Armory.DefaultsForFaction(0).Take(3).All(result.Armory.HasValidPrimary) &&
                result.Armory.DefaultsForFaction(0).Skip(3).All(kit => kit.Items.All(id => id == -1)), "Bootstrap kit policy changed");
            Check(result.Progression.InitialLevel == 1 && result.Progression.InitialSkillPoints == 1 &&
                result.Progression.ThresholdForLevel(2) == 100, "Bound progression was not loaded");
            Reject(() => Load(20230, 1793)); Reject(() => Load(20231, 1740));
            foreach (var variable in new[] {"MAG_LOCAL_DME", "MAG_WORLD_REGISTRY", "MAG_RETAIL_SKILL_CATALOG",
                "MAG_TRAINING_COMPLETION_CATALOG", "MAG_TRAINING_LEVEL", "MAG_RESPEC_POINTS_FLOOR", "MAG_RESPEC_FEE_CAP", "MAG_STARTING_LEVEL"})
            {
                string previous = env.GetValueOrDefault(variable); env[variable] = variable == "MAG_LOCAL_DME" ? "1" : "unqualified";
                Reject(() => Load());
                if (previous == null) env.Remove(variable); else env[variable] = previous;
            }
            env.Remove("MAG_CHARACTER_BOOTSTRAP_PROFILE"); Reject(() => Load()); env["MAG_CHARACTER_BOOTSTRAP_PROFILE"] = profilePath;
            // A self-consistent outer receipt cannot relabel an older catalog as1793.
            armory["profile"] = "retail-v1.0"; WriteArmory(); WriteManifest(); Reject(() => Load());
            armory["profile"] = "retail-v2.12-bootstrap"; WriteArmory(); WriteManifest();
            // Bytes changed after a receipt must fail before any account/store is opened.
            File.AppendAllText(armoryPath, " "); Reject(() => Load()); WriteArmory(); WriteManifest();
            manifest["engine_sha256"] = new string('a', 64); WriteManifest(); Reject(() => Load()); manifest["engine_sha256"] = Engine;
            manifest["policy"]["skill_grants"] = "legacy"; WriteManifest(); Reject(() => Load()); manifest["policy"]["skill_grants"] = "none";
            manifest["policy"]["gameplay_worlds"] = true; WriteManifest(); Reject(() => Load()); manifest["policy"]["gameplay_worlds"] = false;
            armory["factions"][0]["skill_ids"] = new JsonArray(1); WriteArmory(); WriteManifest(); Reject(() => Load());
            armory["factions"][0]["skill_ids"] = new JsonArray(); WriteArmory(); WriteManifest();
            Check(Load().Armory.GrantsForFaction(0).Skills.All(word => word == 0), "Bootstrap acquired unsupported skill grants");
            StartupWiring(env, directory);
        }
        finally {Directory.Delete(directory, true);}
    }

    private static void StartupWiring(Dictionary<string, string> env, string directory)
    {
        var type = typeof(Server.Medius.Program);
        var names = new[] {"LoadoutCatalog", "ProgressionCatalog", "InitialSkillPoints", "StartingLevel", "SkillCatalog",
            "TrainingCompletionCatalog", "TrainingLevel", "RespecPointsFloor", "RespecFeeCap", "OpenMatchRowTtl"};
        var properties = names.Select(name => type.GetProperty(name, BindingFlags.Static | BindingFlags.Public)).ToArray();
        var previous = properties.Select(property => property.GetValue(null)).ToArray();
        var variables = env.Keys.Concat(new[] {"MAG_WORLD_REGISTRY", "MAG_RETAIL_SKILL_CATALOG", "MAG_TRAINING_COMPLETION_CATALOG",
            "MAG_TRAINING_LEVEL", "MAG_RESPEC_POINTS_FLOOR", "MAG_RESPEC_FEE_CAP", "MAG_STARTING_LEVEL"}).Distinct().ToArray();
        var oldEnvironment = variables.ToDictionary(name => name, Environment.GetEnvironmentVariable);
        var settings = Server.Medius.Program.Settings;
        int oldApp = settings.MagApplicationId; uint oldProtocol = settings.MagProtocolVersion;
        var load = type.GetMethod("LoadCharacterCatalog", BindingFlags.Static | BindingFlags.NonPublic);
        try
        {
            settings.MagApplicationId = 20231; settings.MagProtocolVersion = 1793;
            foreach (string variable in variables) Environment.SetEnvironmentVariable(variable, env.GetValueOrDefault(variable));
            load.Invoke(null, null);
            var store = Server.Medius.Program.CreateJsonCharacterStore(Path.Combine(directory, "unopened.json"));
            Check(store.LoadoutCatalog != null && store.ProgressionCatalog?.InitialLevel == 1 && store.StartingLevel == 1 &&
                store.SkillCatalog == null && store.TrainingCompletionCatalog == null && store.TrainingLevel == null,
                "Program did not configure the authentication-only character store");
            Environment.SetEnvironmentVariable(MagRetail212BootstrapProfile.ProfileVariable, null);
            try {load.Invoke(null, null); throw new Exception("1793 startup fell back to legacy catalogs");}
            catch (TargetInvocationException error) when (error.InnerException is InvalidDataException) { }
            Check(!File.Exists(Path.Combine(directory, "unopened.json")), "Catalog startup unexpectedly opened a character store");
        }
        finally
        {
            settings.MagApplicationId = oldApp; settings.MagProtocolVersion = oldProtocol;
            foreach (var entry in oldEnvironment) Environment.SetEnvironmentVariable(entry.Key, entry.Value);
            for (int index = 0; index < properties.Length; index++) properties[index].SetValue(null, previous[index]);
        }
    }
}
