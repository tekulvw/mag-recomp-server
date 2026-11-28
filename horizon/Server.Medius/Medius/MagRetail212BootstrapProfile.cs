using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text.Json;

namespace Server.Medius
{
    /// <summary>Explicit source-bound character inputs for authentication-only protocol 1793 testing.
    /// This profile makes no claim about native acceptance, purchases or gameplay.</summary>
    public sealed class MagRetail212BootstrapProfile
    {
        public const string ProfileVariable = "MAG_CHARACTER_BOOTSTRAP_PROFILE";
        public const string Name = "retail-v2.12-bootstrap";
        private const string Engine = "705a6ec6a25458b2401b869e75e2904597304e2c06d9534a171a1dcd54aca2b3";
        private static readonly IReadOnlyDictionary<string, string> Sources = new Dictionary<string, string> {
            ["headers"] = "9845c9c024dddbe035578d0afe09fe3022b2d19a7969a89dd0701f8a8e391326",
            ["classes"] = "b6c722ac6acb53710b470b96e394942a03c223a08c0703c42b2d7024d6562325",
            ["equipment"] = "e187f6989d363aef7f782d7d23ef1e38fd94aa338c449b390836fb2137e9d53d"};
        public MagLoadoutCatalog Armory { get; private set; }
        public MagProgressionCatalog Progression { get; private set; }
        private MagRetail212BootstrapProfile() { }

        public static MagRetail212BootstrapProfile Load(int applicationId, uint protocol, Func<string, string> read)
        {
            if (applicationId != 20231 || protocol != 1793)
                throw new InvalidDataException("The 2.12 bootstrap profile requires application 20231/protocol 1793.");
            if (read == null) throw new InvalidDataException("Bootstrap configuration reader is missing.");
            string dme = read("MAG_LOCAL_DME");
            if (!string.IsNullOrEmpty(dme) && dme != "0")
                throw new InvalidDataException("The 2.12 bootstrap profile requires MAG_LOCAL_DME=0.");
            foreach (string variable in new[] {"MAG_WORLD_REGISTRY", "MAG_RETAIL_SKILL_CATALOG", "MAG_TRAINING_COMPLETION_CATALOG",
                "MAG_TRAINING_LEVEL", "MAG_RESPEC_POINTS_FLOOR", "MAG_RESPEC_FEE_CAP"})
                if (!string.IsNullOrEmpty(read(variable)))
                    throw new InvalidDataException(variable + " is unqualified for the 2.12 bootstrap profile; omit it.");
            if (!string.IsNullOrEmpty(read("MAG_STARTING_LEVEL")) && read("MAG_STARTING_LEVEL") != "1")
                throw new InvalidDataException("The 2.12 bootstrap profile requires MAG_STARTING_LEVEL omitted or 1.");
            string profilePath = read(ProfileVariable);
            try
            {
                using var manifest = JsonDocument.Parse(ReadBounded(profilePath, 16384, ProfileVariable));
                var root = manifest.RootElement;
                if (root.GetProperty("format").GetString() != "mag-retail-v212-bootstrap-v1")
                    throw new InvalidDataException("Unsupported 2.12 bootstrap manifest format.");
                RequireIdentity(root);
                var policy = root.GetProperty("policy");
                if (policy.GetProperty("initial_level").GetInt32() != 1 || policy.GetProperty("additional_skill_points").GetInt32() != 0 ||
                    policy.GetProperty("additional_leadership_points").GetInt32() != 0 || policy.GetProperty("authored_kits").GetInt32() != 3 ||
                    policy.GetProperty("wire_loadouts").GetInt32() != 5 || policy.GetProperty("update_id").GetInt32() != 1 ||
                    policy.GetProperty("skill_grants").GetString() != "none" || policy.GetProperty("training_rewards").GetBoolean() ||
                    policy.GetProperty("gameplay_worlds").GetBoolean())
                    throw new InvalidDataException("Unsupported 2.12 bootstrap operator policy.");
                byte[] armory = ReadCatalog(root, "armory", profilePath, read("MAG_RETAIL_ARMORY_CATALOG"));
                byte[] progression = ReadCatalog(root, "progression", profilePath, read("MAG_PROGRESSION_CATALOG"));
                using (var data = JsonDocument.Parse(progression))
                {
                    var creation = data.RootElement.GetProperty("creation_settings").GetProperty("root_defaults");
                    if (creation.GetProperty("StartingLevel").GetInt32() != 1 || creation.GetProperty("AdditionalSkillPoints").GetInt32() != 0 ||
                        (creation.TryGetProperty("AdditionalLeadershipPoints", out var leadership) && leadership.GetUInt32() != 0))
                        throw new InvalidDataException("Progression conflicts with the bootstrap creation policy.");
                }
                var result = new MagRetail212BootstrapProfile {
                    Armory = MagLoadoutCatalog.LoadRetailBytes(armory),
                    Progression = MagProgressionCatalog.LoadBytes(progression, 20231)};
                for (byte faction = 0; faction < 3; faction++)
                {
                    var kits = result.Armory.DefaultsForFaction(faction);
                    var grants = result.Armory.GrantsForFaction(faction);
                    if (kits?.Length != 5 || kits.Any(kit => kit.UpdateId != 1) ||
                        kits.Take(3).Any(kit => !result.Armory.HasValidPrimary(kit)) ||
                        kits.Skip(3).Any(kit => kit.Items.Any(id => id != -1)) || grants == null || grants.Skills.Any(word => word != 0))
                        throw new InvalidDataException("Armory conflicts with the bootstrap kit/grant policy.");
                    foreach (short item in kits.Take(3).SelectMany(kit => kit.Items).Where(item => item >= 0))
                    {
                        int bit = item & 1023;
                        if (bit >= 768 || (grants.AvailableItems[bit >> 5] & (1u << (bit & 31))) == 0)
                            throw new InvalidDataException("A bootstrap starter item has no matching ownership bit.");
                    }
                }
                return result;
            }
            catch (Exception error) when (error is JsonException || error is KeyNotFoundException || error is InvalidOperationException ||
                error is FormatException || error is OverflowException)
            { throw new InvalidDataException("Invalid 2.12 bootstrap manifest/catalog schema.", error); }
        }

        private static void RequireIdentity(JsonElement root)
        {
            if (root.GetProperty("profile").GetString() != Name || root.GetProperty("application_id").GetInt32() != 20231 ||
                root.GetProperty("protocol_version").GetInt32() != 1793 || root.GetProperty("engine_sha256").GetString() != Engine)
                throw new InvalidDataException("Catalog/profile target differs from the supported 2.12 input identity.");
            var sources = root.GetProperty("source_sha256");
            if (sources.EnumerateObject().Count() != Sources.Count ||
                Sources.Any(pair => sources.GetProperty(pair.Key).GetString() != pair.Value))
                throw new InvalidDataException("Unsupported 2.12 bootstrap resource identities.");
        }

        private static byte[] ReadCatalog(JsonElement root, string name, string profilePath, string path)
        {
            var catalog = root.GetProperty("catalogs").GetProperty(name);
            string file = catalog.GetProperty("file").GetString();
            if (string.IsNullOrEmpty(file) || file != Path.GetFileName(file) || file is "." or ".." ||
                string.IsNullOrEmpty(path) || !Path.IsPathFullyQualified(path) ||
                Path.GetFullPath(path) != Path.GetFullPath(Path.Combine(Path.GetDirectoryName(profilePath), file)))
                throw new InvalidDataException("Bootstrap catalog paths must match sibling manifest members.");
            byte[] bytes = ReadBounded(path, 2 * 1024 * 1024, name);
            string digest = Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
            if (digest != catalog.GetProperty("sha256").GetString())
                throw new InvalidDataException("Bootstrap catalog content differs from its manifest SHA256.");
            using var data = JsonDocument.Parse(bytes);
            RequireIdentity(data.RootElement);
            return bytes;
        }

        private static byte[] ReadBounded(string path, int maximum, string variable)
        {
            if (string.IsNullOrEmpty(path) || !Path.IsPathFullyQualified(path))
                throw new InvalidDataException(variable + " requires an absolute file path.");
            try
            {
                using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
                if (stream.Length < 2 || stream.Length > maximum) throw new InvalidDataException("Bootstrap input exceeds its JSON byte bound.");
                var bytes = new byte[(int)stream.Length]; stream.ReadExactly(bytes);
                if (stream.ReadByte() != -1) throw new InvalidDataException("Bootstrap input changed while reading.");
                return bytes;
            }
            catch (IOException error)
            { throw new InvalidDataException("Cannot read bootstrap input " + variable + ".", error); }
        }
    }
}
