using System;
using System.IO;
using System.Linq;
using System.Text.Json.Serialization;
using RT.Models.MAG;

namespace Server.Medius
{
    public sealed class GameplayAdmissionLoadouts
    {
        [JsonPropertyName("selected_index")] public byte SelectedIndex { get; }
        [JsonPropertyName("update_ids")] public int[] UpdateIds { get; }
        [JsonPropertyName("inventories")] public ushort?[][] Inventories { get; }
        internal GameplayAdmissionLoadouts(MagCharacterRecord record)
        {
            SelectedIndex = record.LastUsedLoadout;
            UpdateIds = record.Loadouts.Select(row => (int)row.UpdateId).ToArray();
            Inventories = record.Loadouts.Select(row => row.Items.Select(item => item < 0 ? (ushort?)null : (ushort)item).ToArray()).ToArray();
        }
    }

    /// <summary>Admission projects the actual record's saved or configured default
    /// inventories. It does not replace the running actor's equipped kit.</summary>
    public static class GameplayLoadoutAdmission
    {
        public static GameplayAdmissionLoadouts Project(StoredCharacter character, MagCharacterRecord record, MagLoadoutCatalog catalog)
        {
            if (character == null || record == null) throw new ArgumentNullException();
            // With no configured catalog, preserve the previous absent-inventory
            // fallback. A native fallback is not an authored saved-kit catalog.
            if (catalog == null && character.Loadouts == null) return null;
            if (record.LastUsedLoadout >= 5 || record.Loadouts == null || record.Loadouts.Length != 5 ||
                record.Loadouts.Any(row => row?.Items == null || row.Items.Length != 22))
                throw new InvalidDataException("Invalid loadout admission record.");
            if (catalog != null && (record.Loadouts.Any(row => !catalog.ValidForFaction(row, character.Faction)) ||
                !catalog.HasValidPrimary(record.Loadouts[0]) || !catalog.HasValidPrimary(record.Loadouts[record.LastUsedLoadout])))
                throw new InvalidDataException("Invalid authored loadout admission record.");
            return new GameplayAdmissionLoadouts(record);
        }
    }
}
