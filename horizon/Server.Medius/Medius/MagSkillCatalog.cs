using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;

namespace Server.Medius
{
    /// <summary>Skill rules loaded from the selected application's retained authored catalog. Purchase eligibility
    /// follows native CanPurchaseSkill: the target category's exact preceding tier, not total spend.</summary>
    public sealed class MagSkillCatalog
    {
        private sealed class Skill
        {
            public byte Cost;
            public int Tier;
            public string Category;
            public int? PrerequisiteId; // Authored metadata; native CanPurchaseSkill does not test it.
            public bool[] Available = new bool[3];
            public bool[] Resolved = new bool[3];
            public int[][] Equipment = new int[3][];
        }
        private static readonly string[] FactionNames = { "Green", "Black", "Red" };
        private readonly Dictionary<int, Skill> _skills = new Dictionary<int, Skill>();
        private readonly Dictionary<string, int[][]> _tiers = new Dictionary<string, int[][]>(StringComparer.Ordinal);
        private readonly HashSet<int> _purchaseEquipment = new HashSet<int>();
        public int ApplicationId { get; private set; }
        private MagSkillCatalog() { }
        public static MagSkillCatalog Load(string path) => LoadForApplication(path, 20230);
        public static MagSkillCatalog LoadForApplication(string path, int applicationId)
        {
            try
            {
                using var document = JsonDocument.Parse(File.ReadAllBytes(path));
                var root = document.RootElement;
                if ((applicationId != 20230 && applicationId != 20231) ||
                    root.GetProperty("schema_version").GetInt32() != (applicationId == 20231 ? 2 : 1) ||
                    root.GetProperty("application_id").GetInt32() != applicationId)
                    throw new InvalidDataException("Unsupported skill catalog schema/application.");
                var result = new MagSkillCatalog { ApplicationId = applicationId };
                // Native retail AwardSkill skips tokens with no item binding; it
                // still deducts the skill cost and records the owned skill.
                var noAward = new HashSet<string>(StringComparer.Ordinal);
                if (root.TryGetProperty("equipment_tokens_without_item_ids", out var skipped))
                    foreach (var token in skipped.EnumerateArray())
                        if (string.IsNullOrWhiteSpace(token.GetString()) || !noAward.Add(token.GetString()))
                            throw new InvalidDataException("Invalid native no-award equipment token.");
                var aliases = new Dictionary<string, int[]>(StringComparer.Ordinal);
                foreach (var entry in root.GetProperty("equipment_token_item_ids").EnumerateObject())
                {
                    var ids = entry.Value.EnumerateArray().Select(value => value.GetInt32()).ToArray();
                    if (noAward.Contains(entry.Name) || string.IsNullOrWhiteSpace(entry.Name) || ids.Length == 0 || ids.Any(id => id < 0 || id >= 768) || ids.Distinct().Count() != ids.Length || !aliases.TryAdd(entry.Name, ids))
                        throw new InvalidDataException("Invalid skill equipment token/item ordinals.");
                }
                var table = root.GetProperty("table");
                foreach (var entry in table.GetProperty("Tiers").EnumerateObject())
                {
                    var data = entry.Value.GetProperty("MagCharacterSkillTier");
                    string category = data.GetProperty("Category").GetString();
                    if (string.IsNullOrWhiteSpace(category) || result._tiers.ContainsKey(category)) throw new InvalidDataException("Invalid skill tier category.");
                    var thresholds = new int[3][];
                    for (int faction = 0; faction < 3; faction++)
                    {
                        thresholds[faction] = new int[6];
                        var factionData = data.GetProperty(FactionNames[faction]);
                        for (int tier = 2; tier <= 5; tier++)
                        {
                            int threshold = factionData.GetProperty("Tier" + tier + "_PrereqPoints").GetInt32();
                            if (threshold < 0 || threshold > 255) throw new InvalidDataException("Invalid skill tier prerequisite points.");
                            thresholds[faction][tier] = threshold;
                        }
                    }
                    result._tiers.Add(category, thresholds);
                }
                var skillOrdinals = new HashSet<int>();
                foreach (var entry in table.GetProperty("Skills").EnumerateObject())
                {
                    const string prefix = "MagCharacterSkill";
                    if (!entry.Name.StartsWith(prefix, StringComparison.Ordinal) || !int.TryParse(entry.Name.Substring(prefix.Length), out int id) || id < 0 || id > 127 || !skillOrdinals.Add(id))
                        throw new InvalidDataException("Invalid native signed skill ordinal.");
                    var data = entry.Value.GetProperty("MagCharacterSkill");
                    if (data.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Invalid skill definition.");
                    if (!data.EnumerateObject().Any()) continue; // Retained reserved skill slots are not purchasable.
                    var skill = new Skill { Cost = data.GetProperty("Cost").GetByte(), Tier = data.GetProperty("Tier").GetInt32(), Category = data.GetProperty("Category").GetString() };
                    if (skill.Tier < 0 || skill.Tier > 5 || string.IsNullOrWhiteSpace(skill.Category) || !result._tiers.ContainsKey(skill.Category))
                        throw new InvalidDataException("Invalid skill tier/category binding.");
                    if (data.TryGetProperty("PrereqID", out var prerequisite))
                    {
                        int prerequisiteId = prerequisite.GetInt32();
                        if (prerequisiteId < 0 || prerequisiteId > 127 || prerequisiteId == id) throw new InvalidDataException("Invalid authored skill prerequisite ordinal.");
                        skill.PrerequisiteId = prerequisiteId;
                    }
                    for (byte faction = 0; faction < 3; faction++)
                    {
                        var factionData = data.GetProperty(FactionNames[faction]);
                        skill.Available[faction] = factionData.GetProperty("Available").GetBoolean();
                        skill.Resolved[faction] = true;
                        var equipmentIds = new HashSet<int>();
                        JsonElement equipment;
                        bool hasEquipment = factionData.TryGetProperty("Equipment", out equipment) ||
                            (data.TryGetProperty("Default", out var defaults) && defaults.TryGetProperty("Equipment", out equipment));
                        if (hasEquipment)
                        {
                            foreach (var reward in equipment.EnumerateObject())
                            {
                                string token = reward.Value.GetProperty("element").GetString();
                                if (string.IsNullOrEmpty(token)) throw new InvalidDataException("Invalid skill equipment reward token.");
                                if (noAward.Contains(token)) continue;
                                if (!aliases.TryGetValue(token, out var ids)) { skill.Resolved[faction] = false; continue; }
                                // GetGlobalItemIndex scans authored ordinals ascending and returns
                                // the first token match; AwardSkill sets only that one item's bit.
                                equipmentIds.Add(ids.Min());
                                if (skill.Cost != 0) foreach (int itemId in ids) result._purchaseEquipment.Add(itemId);
                            }
                        }
                        skill.Equipment[faction] = equipmentIds.OrderBy(value => value).ToArray();
                    }
                    result._skills.Add(id, skill);
                }
                if (result._skills.Count == 0 || result._tiers.Count == 0) throw new InvalidDataException("Empty authored skill catalog.");
                return result;
            }
            catch (Exception e) when (e is JsonException || e is KeyNotFoundException || e is InvalidOperationException || e is FormatException || e is OverflowException)
            { throw new InvalidDataException("Invalid skill catalog JSON/schema.", e); }
        }
        public bool RequiresPurchaseOwnership(int itemId) => _purchaseEquipment.Contains(itemId);
        internal bool TryPurchase(byte faction, int skillId, byte points, uint[] ownedSkills, out byte cost, out int[] equipment, out string reason)
        {
            cost = 0; equipment = Array.Empty<int>(); reason = "invalid-skill";
            if (faction > 2 || skillId < 0 || skillId > 127 || ownedSkills == null || ownedSkills.Length != 5 || !_skills.TryGetValue(skillId, out var skill)) return false;
            if (Has(ownedSkills, skillId)) { reason = "skill-already-owned"; return false; }
            if (!skill.Available[faction]) { reason = "skill-unavailable-for-faction"; return false; }
            if (!skill.Resolved[faction]) { reason = "skill-equipment-unresolved"; return false; }
            if (points < skill.Cost) { reason = "insufficient-skill-points"; return false; }
            int required = _tiers[skill.Category][faction][skill.Tier];
            if (required > 0)
            {
                int spent = _skills.Where(entry => Has(ownedSkills, entry.Key) && entry.Value.Category == skill.Category && entry.Value.Tier == skill.Tier - 1).Sum(entry => (int)entry.Value.Cost);
                if (spent < required) { reason = "skill-tier-prerequisite-not-met"; return false; }
            }
            cost = skill.Cost; equipment = (int[])skill.Equipment[faction].Clone(); reason = "skill-purchase-eligible"; return true;
        }
        private static bool Has(uint[] words, int id) => (words[id >> 5] & (1u << (id & 31))) != 0;
    }
}
