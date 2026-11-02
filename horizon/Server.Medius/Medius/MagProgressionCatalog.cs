using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;

namespace Server.Medius
{
    /// <summary>Authored cumulative XP thresholds and level grants. Values come from
    /// the selected application disk catalog; conversion mirrors native single-precision math.</summary>
    public sealed class MagProgressionCatalog
    {
        private sealed class LevelRow { public int Threshold; public byte Points; }
        private readonly SortedDictionary<byte, LevelRow> _levels = new SortedDictionary<byte, LevelRow>();
        private readonly Dictionary<string, float> _objectiveAwards = new Dictionary<string, float>(StringComparer.Ordinal);
        private readonly Dictionary<string, float> _optionalActionAwards = new Dictionary<string, float>(StringComparer.Ordinal);
        private readonly Dictionary<string, float> _victoryModifiers = new Dictionary<string, float>(StringComparer.Ordinal);
        private static readonly Dictionary<string, string> ObjectiveAwardKeys = new Dictionary<string, string>(StringComparer.Ordinal) {
            { "primary_objective_activated", "PrimaryObjectiveActivated" }, { "primary_objective_secured", "PrimaryObjectiveSecured" },
            { "primary_objective_completed", "PrimaryObjective" }, { "secondary_objective_completed", "SecondaryObjective" },
            { "charge_planted", "ChargePlanted" }, { "charge_defused", "ChargeDefused" } };
        private static readonly Dictionary<string, string> OptionalActionAwardKeys = new Dictionary<string, string>(StringComparer.Ordinal) {
            { "enemy_finished", "FinishingMoves" }, { "teammate_resuscitated", "Resuscitations" } };
        private static readonly Dictionary<string, string> VictoryModifierKeys = new Dictionary<string, string>(StringComparer.Ordinal) {
            { "match_major_victory", "MajorVictory" }, { "match_minor_victory", "MinorVictory" },
            { "match_major_defeat", "MajorDefeat" }, { "match_minor_defeat", "MinorDefeat" } };
        private int _enemyKilled;
        private float _healOther;
        public byte MaximumLevel => _levels.Keys.Last();
        public byte InitialLevel { get; private set; }
        public int InitialExperience => _levels[InitialLevel].Threshold;
        public byte InitialSkillPoints { get; private set; }
        private MagProgressionCatalog() { }
        public static MagProgressionCatalog Load(string path, int applicationId = 20230)
        {
            try
            {
                if (!Path.IsPathFullyQualified(path)) throw new InvalidDataException("An absolute progression catalog path is required.");
                if (new FileInfo(path).Length > 2097152) throw new InvalidDataException("Progression catalog exceeds local bound.");
                using var document = JsonDocument.Parse(File.ReadAllBytes(path));
                var root = document.RootElement;
                if (root.GetProperty("schema_version").GetInt32() != 1 || (applicationId != 20230 && applicationId != 20231) || root.GetProperty("application_id").GetInt32() != applicationId)
                    throw new InvalidDataException("Unsupported progression catalog schema/application.");
                var table = root.GetProperty("table");
                float scale = table.GetProperty("ExperienceRequiredScale").GetSingle();
                if (!float.IsFinite(scale) || scale <= 0) throw new InvalidDataException("Invalid experience threshold scale.");
                var result = new MagProgressionCatalog();
                var ordinals = new HashSet<int>();
                foreach (var entry in table.GetProperty("Levels").EnumerateObject())
                {
                    const string prefix = "ExperienceLevel";
                    if (!entry.Name.StartsWith(prefix, StringComparison.Ordinal) || !int.TryParse(entry.Name.Substring(prefix.Length), out int level) ||
                        level < 0 || level > byte.MaxValue || !ordinals.Add(level)) throw new InvalidDataException("Invalid authored level ordinal.");
                    var data = entry.Value.GetProperty("ExperienceLevel");
                    if (data.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Invalid level definition.");
                    if (!data.EnumerateObject().Any())
                    { if (level != 0) throw new InvalidDataException("Missing defined level row."); continue; }
                    if (level == 0) throw new InvalidDataException("Unused level zero must remain empty.");
                    int experience = data.GetProperty("Experience").GetInt32();
                    byte points = data.GetProperty("SkillPointsGained").GetByte();
                    if (experience < 0 || data.GetProperty("LoadoutBudget").GetInt32() < 0 || data.GetProperty("Rank").GetInt32() < 0)
                        throw new InvalidDataException("Invalid authored level fields.");
                    float scaled = (float)experience * scale;
                    if (!float.IsFinite(scaled) || (double)scaled > int.MaxValue) throw new InvalidDataException("Experience threshold exceeds native signed range.");
                    result._levels.Add((byte)level, new LevelRow { Threshold = checked((int)MathF.Truncate(scaled)), Points = points });
                }
                if (result._levels.Count == 0 || !result._levels.ContainsKey(1) || result._levels[1].Threshold != 0 ||
                    result._levels.Keys.Select((level, index) => level != index + 1).Any(value => value))
                    throw new InvalidDataException("Progression levels must be contiguous from level one at zero XP.");
                int previous = -1;
                foreach (var row in result._levels.Values)
                { if (row.Threshold <= previous) throw new InvalidDataException("Progression thresholds must increase."); previous = row.Threshold; }
                var creation = root.GetProperty("creation_settings").GetProperty("root_defaults");
                result.InitialLevel = creation.GetProperty("StartingLevel").GetByte();
                int additional = creation.GetProperty("AdditionalSkillPoints").GetInt32();
                if (result.InitialLevel < 1 || result.InitialLevel > result.MaximumLevel || additional < 0 || additional > byte.MaxValue)
                    throw new InvalidDataException("Invalid authored character progression defaults.");
                result.InitialSkillPoints = (byte)Math.Min(byte.MaxValue, additional + result._levels.Where(entry => entry.Key <= result.InitialLevel).Sum(entry => (int)entry.Value.Points));
                var awards = table.GetProperty("Awards");
                ValidateAwards(awards);
                result._enemyKilled = awards.GetProperty("EnemyKilled").GetInt32();
                result._healOther = awards.GetProperty("ExperienceToHealOtherRatio").GetSingle();
                if (result._enemyKilled < 0 || !float.IsFinite(result._healOther) || result._healOther < 0)
                    throw new InvalidDataException("Invalid supported reward fields.");
                foreach (var entry in ObjectiveAwardKeys)
                {
                    float value = awards.GetProperty(entry.Value).GetSingle();
                    if (!float.IsFinite(value) || value < 0 || (double)value > int.MaxValue) throw new InvalidDataException("Invalid objective reward field.");
                    result._objectiveAwards.Add(entry.Key, value);
                }
                foreach (var entry in OptionalActionAwardKeys)
                {
                    if (!awards.TryGetProperty(entry.Value, out var authored)) continue;
                    float value = authored.GetSingle();
                    if (!float.IsFinite(value) || value < 0 || (double)value > int.MaxValue) throw new InvalidDataException("Invalid optional action reward field.");
                    result._optionalActionAwards.Add(entry.Key, value);
                }
                var victory = awards.GetProperty("VictoryModifier");
                foreach (var entry in VictoryModifierKeys)
                {
                    float value = victory.GetProperty(entry.Value).GetSingle();
                    if (!float.IsFinite(value) || value < 0) throw new InvalidDataException("Invalid victory modifier.");
                    result._victoryModifiers.Add(entry.Key, value);
                }
                return result;
            }
            catch (Exception e) when (e is JsonException || e is KeyNotFoundException || e is InvalidOperationException || e is FormatException || e is OverflowException || e is ArgumentException)
            { throw new InvalidDataException("Invalid progression catalog JSON/schema.", e); }
        }
        private static void ValidateAwards(JsonElement data)
        {
            if (data.ValueKind != JsonValueKind.Object || !data.EnumerateObject().Any()) throw new InvalidDataException("Invalid award table.");
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var entry in data.EnumerateObject())
            {
                if (!names.Add(entry.Name)) throw new InvalidDataException("Duplicate award field.");
                if (entry.Value.ValueKind == JsonValueKind.Object) ValidateAwards(entry.Value);
                else if (entry.Value.ValueKind != JsonValueKind.Number || !double.IsFinite(entry.Value.GetDouble())) throw new InvalidDataException("Non-numeric award field.");
            }
        }
        public int ThresholdForLevel(byte level) => _levels.TryGetValue(level, out var row) ? row.Threshold : throw new ArgumentOutOfRangeException(nameof(level));
        public byte LevelForExperience(int experience)
        {
            if (experience < 0) throw new ArgumentOutOfRangeException(nameof(experience));
            byte level = 1;
            foreach (var entry in _levels) { if (experience < entry.Value.Threshold) break; level = entry.Key; }
            return level;
        }
        public byte PointsAfterGain(byte points, byte previousLevel, byte level)
        {
            if (previousLevel < 1 || level < previousLevel || level > MaximumLevel) throw new ArgumentOutOfRangeException(nameof(level));
            int gained = 0;
            foreach (var entry in _levels) if (entry.Key > previousLevel && entry.Key <= level) gained += entry.Value.Points;
            return (byte)Math.Min(byte.MaxValue, points + gained);
        }
        internal static bool IsFinalRewardKind(string kind) => kind != null && VictoryModifierKeys.ContainsKey(kind);
        internal static bool ValidKindUnits(string kind, int units) => units > 0 && units <= ushort.MaxValue &&
            (kind == "teammate_healed" || units == 1 && (kind == "enemy_killed" || kind != null && (ObjectiveAwardKeys.ContainsKey(kind) || OptionalActionAwardKeys.ContainsKey(kind) || IsFinalRewardKind(kind))));
        internal bool TryFinalAward(string kind, int units, int matchExperience, out int experience)
        {
            experience = 0;
            if (!ValidKindUnits(kind, units) || matchExperience < 0 || !_victoryModifiers.TryGetValue(kind, out float modifier)) return false;
            // Retail AwardExperience converts the original base to f32, multiplies, then truncates.
            // This ledger currently contains action/objective XP only: ribbon, directive, shadow-war
            // and veteran inputs are unavailable and are not invented here.
            float total = (float)matchExperience * modifier;
            if (!float.IsFinite(total) || total < 0 || (double)total > int.MaxValue) return false;
            experience = checked((int)MathF.Truncate(total));
            return true;
        }
        internal bool TryAward(string kind, int units, float matchFraction, out int experience, out float fraction)
        {
            experience = 0; fraction = matchFraction;
            if (!float.IsFinite(matchFraction) || matchFraction < 0 || matchFraction >= 1 || !ValidKindUnits(kind, units)) return false;
            float reward;
            if (kind == "enemy_killed") { if (units != 1) return false; reward = (float)_enemyKilled; }
            else if (kind == "teammate_healed") reward = _healOther;
            else if (!_objectiveAwards.TryGetValue(kind, out reward) && !_optionalActionAwards.TryGetValue(kind, out reward))
            {
                // A recognized optional action still needs a durable receipt when
                // its catalog has no authored XP entry. Settle it without credit
                // so delivery retries do not leave the gameplay outbox pending.
                // This absence policy supplies no tuning value for the event.
                if (!OptionalActionAwardKeys.ContainsKey(kind)) return false;
                reward = 0;
            }
            // Native Increment shares fractional carry for integer kills and healing.
            float total = MathF.FusedMultiplyAdd((float)units, reward, matchFraction);
            if (!float.IsFinite(total) || total < 0 || (double)total > int.MaxValue) return false;
            experience = checked((int)MathF.Truncate(total)); fraction = total - experience;
            return float.IsFinite(fraction) && fraction >= 0 && fraction < 1;
        }
    }
}
