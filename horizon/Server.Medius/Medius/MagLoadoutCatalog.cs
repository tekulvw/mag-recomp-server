using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Immutable authored item bindings, ownership indices and faction starter defaults.</summary>
    public sealed class MagCharacterGrants
    {
        public uint[] AvailableItems { get; }
        public uint[] Skills { get; }
        internal MagCharacterGrants(uint[] availableItems, uint[] skills) { AvailableItems = availableItems; Skills = skills; }
        internal MagCharacterGrants Clone() => new MagCharacterGrants((uint[])AvailableItems.Clone(), (uint[])Skills.Clone());
    }
    public sealed class MagLoadoutCatalog
    {
        private readonly HashSet<short> _assigned = new HashSet<short>();
        private readonly HashSet<short> _primaryWeapons = new HashSet<short>();
        private readonly Dictionary<byte, MagLoadout[]> _defaults = new Dictionary<byte, MagLoadout[]>();
        private readonly Dictionary<byte, MagCharacterGrants> _grants = new Dictionary<byte, MagCharacterGrants>();
        private readonly Dictionary<short, int> _ownershipOrdinals = new Dictionary<short, int>();
        private readonly Dictionary<short, HashSet<byte>> _itemFactions = new Dictionary<short, HashSet<byte>>();
        public int ApplicationId { get; private set; } = 20230;
        private MagLoadoutCatalog() { }
        public static MagLoadoutCatalog LoadRetail(string path)
        {
            try
            {
                using var document = JsonDocument.Parse(File.ReadAllBytes(path));
                var root = document.RootElement;
                if (root.GetProperty("schema_version").GetInt32() != 1 || root.GetProperty("application_id").GetInt32() != 20231)
                    throw new InvalidDataException("Unsupported retail Armory schema/application.");
                var sources = root.GetProperty("source_sha256");
                if (sources.ValueKind != JsonValueKind.Object || !sources.EnumerateObject().Any() || sources.EnumerateObject().Any(entry =>
                    string.IsNullOrEmpty(entry.Name) || entry.Value.GetString()?.Length != 64 || entry.Value.GetString().Any(c => !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')))))
                    throw new InvalidDataException("Invalid retail Armory provenance.");
                var result = new MagLoadoutCatalog { ApplicationId = 20231 };
                var references = new HashSet<string>(StringComparer.Ordinal);
                foreach (var item in root.GetProperty("items").EnumerateArray())
                {
                    short id = item.GetProperty("wire_id").GetInt16();
                    int ordinal = item.GetProperty("available_bit").GetInt32();
                    string reference = item.GetProperty("authored_ref").GetString();
                    var factions = item.GetProperty("factions").EnumerateArray().Select(value => value.GetByte()).ToArray();
                    if (id < 0 || id >= 4096 || ordinal < 0 || ordinal >= 768 || ordinal != (id & 1023) ||
                        string.IsNullOrEmpty(item.GetProperty("name").GetString()) || string.IsNullOrEmpty(reference) || !references.Add(reference) ||
                        factions.Length == 0 || factions.Any(faction => faction > 2) || factions.Distinct().Count() != factions.Length || !result._assigned.Add(id))
                        throw new InvalidDataException("Invalid retail Armory item binding.");
                    result._ownershipOrdinals.Add(id, ordinal);
                    result._itemFactions.Add(id, new HashSet<byte>(factions));
                    var ammo = item.GetProperty("primary_ammo");
                    if (ammo.ValueKind != JsonValueKind.Null)
                    {
                        int capacity = ammo.GetInt32();
                        if (capacity < 1 || capacity > 255) throw new InvalidDataException("Invalid retail primary magazine capacity.");
                        result._primaryWeapons.Add(id);
                    }
                }
                if (result._assigned.Count == 0) throw new InvalidDataException("Empty retail Armory item bindings.");
                foreach (var faction in root.GetProperty("factions").EnumerateArray())
                {
                    byte id = faction.GetProperty("faction").GetByte();
                    if (id > 2 || result._defaults.ContainsKey(id)) throw new InvalidDataException("Invalid retail Armory faction.");
                    var rows = faction.GetProperty("loadouts").EnumerateArray().Select(row => new MagLoadout {
                        UpdateId = row.GetProperty("update_id").GetByte(),
                        Items = row.GetProperty("items").EnumerateArray().Select(value => value.ValueKind == JsonValueKind.Null ? (short)-1 : value.GetInt16()).ToArray()
                    }).ToArray();
                    if (rows.Length != 5 || rows.Any(row => !result.ValidForFaction(row, id)) || !result.HasValidPrimary(rows[0]))
                        throw new InvalidDataException("Invalid retail Armory starter loadouts.");
                    var available = new uint[24]; var skills = new uint[5];
                    var granted = new HashSet<int>();
                    foreach (var value in faction.GetProperty("available_item_bits").EnumerateArray())
                    {
                        int bit = value.GetInt32();
                        if (bit < 0 || bit >= 768 || !granted.Add(bit) || !result._ownershipOrdinals.Any(pair => pair.Value == bit && result._itemFactions[pair.Key].Contains(id)))
                            throw new InvalidDataException("Invalid retail starter item grant.");
                        available[bit >> 5] |= 1u << (bit & 31);
                    }
                    var skillIds = new HashSet<int>();
                    foreach (var value in faction.GetProperty("skill_ids").EnumerateArray())
                    {
                        int bit = value.GetInt32();
                        if (bit < 0 || bit >= 160 || !skillIds.Add(bit)) throw new InvalidDataException("Invalid retail starter skill grant.");
                        skills[bit >> 5] |= 1u << (bit & 31);
                    }
                    result._defaults.Add(id, rows);
                    result._grants.Add(id, new MagCharacterGrants(available, skills));
                }
                if (result._defaults.Count != 3) throw new InvalidDataException("Retail Armory requires all three factions.");
                return result;
            }
            catch (Exception e) when (e is JsonException || e is KeyNotFoundException || e is InvalidOperationException || e is FormatException || e is OverflowException)
            { throw new InvalidDataException("Invalid retail Armory JSON/schema.", e); }
        }
        public static MagLoadoutCatalog Load(string itemCatalogPath, string defaultsPath)
        {
            using var items = JsonDocument.Parse(File.ReadAllBytes(itemCatalogPath));
            using var defaults = JsonDocument.Parse(File.ReadAllBytes(defaultsPath));
            var result = new MagLoadoutCatalog();
            var root = items.RootElement;
            if (root.GetProperty("schema_version").GetInt32() != 1 || root.GetProperty("application_id").GetInt32() != 20230 ||
                defaults.RootElement.GetProperty("application_id").GetInt32() != 20230)
                throw new InvalidDataException("Unsupported loadout catalog schema/application.");
            var definitions = root.GetProperty("definitions");
            foreach (var entry in root.GetProperty("items").EnumerateObject())
            {
                if (!short.TryParse(entry.Name, out var id) || id < 0 || id > 1023 || entry.Value.GetProperty("item_id").GetInt32() != id)
                    throw new InvalidDataException("Invalid authored item ordinal.");
                var binding = entry.Value.GetProperty("definition");
                if (binding.ValueKind == JsonValueKind.Null) continue; // Reserved authored ordinal, never equipment.
                var group = binding.GetProperty("group").GetString(); var name = binding.GetProperty("name").GetString();
                if (entry.Value.GetProperty("name").GetString() != name || string.IsNullOrEmpty(entry.Value.GetProperty("item_type").GetString()) ||
                    !definitions.TryGetProperty(group, out var table) || !table.TryGetProperty(name, out var definition))
                    throw new InvalidDataException("Unresolved assigned authored item.");
                result._assigned.Add(id);
                if (group == "weapons" && entry.Value.GetProperty("item_type").GetString() == "kWeapon" &&
                    definition.TryGetProperty("parameters", out var parameters) && parameters.TryGetProperty("magazine_capacity", out var capacity) &&
                    capacity.TryGetInt32(out int magazineCapacity) && magazineCapacity > 0 && magazineCapacity <= 255)
                    result._primaryWeapons.Add(id);
            }
            foreach (var faction in defaults.RootElement.GetProperty("factions").EnumerateArray())
            {
                byte id = faction.GetProperty("faction").GetByte();
                if (id > 2 || result._defaults.ContainsKey(id)) throw new InvalidDataException("Invalid duplicate/default faction.");
                var rows = faction.GetProperty("loadouts").EnumerateArray().Select(row => new MagLoadout {
                    UpdateId = row.GetProperty("update_id").GetByte(),
                    Items = row.GetProperty("items").EnumerateArray().Select(value => value.GetInt16()).ToArray()
                }).ToArray();
                if (rows.Length < 1 || rows.Length > 5 || rows.Any(row => !result.Valid(row)))
                    throw new InvalidDataException("Invalid authored default loadouts.");
                result._defaults.Add(id, Enumerable.Range(0, 5).Select(index => index < rows.Length ? Clone(rows[index]) : new MagLoadout()).ToArray());
            }
            if (result._defaults.Count == 0) throw new InvalidDataException("No authored loadout defaults.");
            string grantsPath = Path.Combine(Path.GetDirectoryName(Path.GetFullPath(defaultsPath)), "prototype-character-grants.json");
            if (File.Exists(grantsPath)) result.LoadGrants(grantsPath);
            return result;
        }
        private void LoadGrants(string path)
        {
            using var document = JsonDocument.Parse(File.ReadAllBytes(path));
            var root = document.RootElement;
            if (root.GetProperty("schema_version").GetInt32() != 1 || root.GetProperty("application_id").GetInt32() != 20230)
                throw new InvalidDataException("Unsupported character grant schema/application.");
            foreach (var faction in root.GetProperty("factions").EnumerateArray())
            {
                byte id = faction.GetProperty("faction").GetByte();
                if (!_defaults.ContainsKey(id) || _grants.ContainsKey(id)) throw new InvalidDataException("Invalid character grant faction.");
                var available = new uint[24]; var skills = new uint[5];
                var itemIds = new HashSet<int>(); var skillIds = new HashSet<int>();
                foreach (var item in faction.GetProperty("available_item_ids").EnumerateArray())
                {
                    int ordinal = item.GetInt32();
                    if (ordinal < 0 || ordinal >= 768 || !_assigned.Contains((short)ordinal) || !itemIds.Add(ordinal))
                        throw new InvalidDataException("Invalid/reserved/duplicate starter item grant.");
                    available[ordinal >> 5] |= 1u << (ordinal & 31);
                }
                foreach (var skill in faction.GetProperty("skill_ids").EnumerateArray())
                {
                    int ordinal = skill.GetInt32();
                    if (ordinal < 0 || ordinal >= 160 || !skillIds.Add(ordinal)) throw new InvalidDataException("Invalid/duplicate starter skill grant.");
                    skills[ordinal >> 5] |= 1u << (ordinal & 31);
                }
                _grants.Add(id, new MagCharacterGrants(available, skills));
            }
        }
        public MagCharacterGrants GrantsForFaction(byte faction) => _grants.TryGetValue(faction, out var grants) ? grants.Clone() : null;
        public bool Valid(MagLoadout loadout)
        {
            if (loadout?.Items == null || loadout.Items.Length != 22 || loadout.Items.Any(id => id < -1 || id >= (ApplicationId == 20231 ? 4096 : 1024) || (id >= 0 && !_assigned.Contains(id)))) return false;
            return loadout.Items.All(id => id == -1) || _primaryWeapons.Contains(loadout.Items[0]);
        }
        public bool ValidForFaction(MagLoadout loadout, byte faction) => Valid(loadout) && faction <= 2 &&
            (ApplicationId != 20231 || loadout.Items.All(id => id == -1 || _itemFactions[id].Contains(faction)));
        public bool TryOwnershipOrdinal(short itemId, out int ordinal)
        {
            if (ApplicationId == 20231) return _ownershipOrdinals.TryGetValue(itemId, out ordinal);
            ordinal = itemId;
            return itemId >= 0 && itemId < 768 && _assigned.Contains(itemId);
        }
        public bool IsStarterDefaultItem(byte faction, int slot, short itemId) => itemId >= 0 && slot >= 0 && slot < 22 &&
            _defaults.TryGetValue(faction, out var rows) && rows.Any(row => row.Items[slot] == itemId);
        public bool HasValidPrimary(MagLoadout loadout) => Valid(loadout) && _primaryWeapons.Contains(loadout.Items[0]);
        public MagLoadout[] DefaultsForFaction(byte faction) => _defaults.TryGetValue(faction, out var defaults) ? defaults.Select(Clone).ToArray() : null;
        private static MagLoadout Clone(MagLoadout row) => new MagLoadout { UpdateId = row.UpdateId, Items = (short[])row.Items.Clone() };
    }
}
