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
        internal const string RepairRewardKind = "repair_units";
        internal const string AssistRewardKind = "enemy_kill_assist";
        private sealed class LevelRow { public int Threshold; public byte Points; }
        private readonly SortedDictionary<byte, LevelRow> _levels = new SortedDictionary<byte, LevelRow>();
        private readonly Dictionary<string, float> _objectiveAwards = new Dictionary<string, float>(StringComparer.Ordinal);
        private readonly Dictionary<string, float> _optionalActionAwards = new Dictionary<string, float>(StringComparer.Ordinal);
        private readonly Dictionary<string, float> _victoryModifiers = new Dictionary<string, float>(StringComparer.Ordinal);
        private readonly SortedDictionary<byte, (int Points, int MinimumLevel)> _leadershipLevels = new();
        private readonly Dictionary<(byte Role, string Kind), uint> _leadershipAwards = new();
        private static readonly Dictionary<string, string> ObjectiveAwardKeys = new Dictionary<string, string>(StringComparer.Ordinal) {
            { "primary_objective_activated", "PrimaryObjectiveActivated" }, { "primary_objective_secured", "PrimaryObjectiveSecured" },
            { "primary_objective_completed", "PrimaryObjective" }, { "secondary_objective_completed", "SecondaryObjective" },
            { "charge_planted", "ChargePlanted" }, { "charge_defused", "ChargeDefused" } };
        private static readonly Dictionary<string, string> ExtendedObjectiveAwardKeys = new Dictionary<string, string>(StringComparer.Ordinal) {
            { "defensive_emplacements_destroyed", "DefensiveEmplacementDestroyed" },
            { "bridges_destroyed", "BridgeDestroyed" },
            { "fortifications_destroyed", "FortificationDestroyed" },
            { "gates_destroyed", "GateDestroyed" },
            { "mortars_destroyed", "MortarDestroyed" },
            { "sensor_networks_destroyed", "SensorNetworkDestroyed" },
            { "anti_aircraft_destroyed", "AntiAircraftDestroyed" },
            { "motor_pools_destroyed", "MotorPoolsDestroyed" },
            { "escort_vehicle_delivered", "EscortVehicleDelivered" } };
        private static readonly Dictionary<string, string> OptionalActionAwardKeys = new Dictionary<string, string>(StringComparer.Ordinal) {
            { "enemy_finished", "FinishingMoves" }, { "teammate_resuscitated", "Resuscitations" } };
        private static readonly Dictionary<string, string> VictoryModifierKeys = new Dictionary<string, string>(StringComparer.Ordinal) {
            { "match_major_victory", "MajorVictory" }, { "match_minor_victory", "MinorVictory" },
            { "match_major_defeat", "MajorDefeat" }, { "match_minor_defeat", "MinorDefeat" } };
        private static readonly Dictionary<byte, string> LeadershipAwardKeys = new() {
            { 1, "LeadershipPoints_Squad" }, { 2, "LeadershipPoints_Platoon" }, { 4, "LeadershipPoints_OIC" } };
        private int _enemyKilled;
        private float _healOther;
        private float? _experiencePerRepairUnit;
        private float? _enemyKillAssist;
        private float? _fragoBonusModifier;
        public byte MaximumLevel => _levels.Keys.Last();
        public byte InitialLevel { get; private set; }
        public int InitialExperience => _levels[InitialLevel].Threshold;
        public byte InitialSkillPoints { get; private set; }
        public uint? InitialLeadershipPoints { get; private set; }
        internal bool FragoBonusEnabled => _fragoBonusModifier.HasValue;
        internal bool FragoActionBonusEnabled => FragoBonusEnabled && _optionalActionAwards.ContainsKey("teammate_resuscitated");
        internal bool RepairRewardsEnabled => _experiencePerRepairUnit.HasValue;
        internal bool AssistRewardsEnabled => _enemyKillAssist.HasValue;
        internal bool ExtendedObjectiveRewardsEnabled => _objectiveAwards.Count == ObjectiveAwardKeys.Count + ExtendedObjectiveAwardKeys.Count;
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
                if (awards.TryGetProperty("ExperiencePerRepairUnit", out var authoredRepairAward))
                {
                    float ratio = authoredRepairAward.GetSingle();
                    if (!float.IsFinite(ratio) || authoredRepairAward.GetDouble() < 0)
                        throw new InvalidDataException("Invalid repair experience ratio.");
                    result._experiencePerRepairUnit = ratio;
                }
                if (awards.TryGetProperty("EnemyKillAssist", out var authoredAssistAward))
                {
                    float award = authoredAssistAward.GetSingle();
                    if (!float.IsFinite(award) || (double)award > int.MaxValue || authoredAssistAward.GetDouble() < 0)
                        throw new InvalidDataException("Invalid assist experience award.");
                    result._enemyKillAssist = award;
                }
                if (awards.TryGetProperty("FRAGOBonusPointModifer", out var authoredFragoBonus))
                {
                    float modifier = authoredFragoBonus.GetSingle();
                    if (!float.IsFinite(modifier) || authoredFragoBonus.GetDouble() < 0) throw new InvalidDataException("Invalid FRAGO bonus modifier.");
                    result._fragoBonusModifier = modifier;
                }
                foreach (var entry in ObjectiveAwardKeys)
                {
                    float value = awards.GetProperty(entry.Value).GetSingle();
                    if (!float.IsFinite(value) || value < 0 || (double)value > int.MaxValue) throw new InvalidDataException("Invalid objective reward field.");
                    result._objectiveAwards.Add(entry.Key, value);
                }
                result.LoadExtendedObjectiveAwards(awards);
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
                result.LoadLeadership(table, awards, creation);
                return result;
            }
            catch (Exception e) when (e is JsonException || e is KeyNotFoundException || e is InvalidOperationException || e is FormatException || e is OverflowException || e is ArgumentException)
            { throw new InvalidDataException("Invalid progression catalog JSON/schema.", e); }
        }
        private void LoadExtendedObjectiveAwards(JsonElement awards)
        {
            var extended = new Dictionary<string, float>(StringComparer.Ordinal);
            foreach (var entry in ExtendedObjectiveAwardKeys)
            {
                if (!awards.TryGetProperty(entry.Value, out var authored)) continue;
                float value = authored.GetSingle();
                if (!float.IsFinite(value) || authored.GetDouble() < 0 || (double)value > int.MaxValue)
                    throw new InvalidDataException($"Invalid extended objective award field {entry.Value}.");
                extended.Add(entry.Key, value);
            }
            // Make the optional group available to settlement as a whole.
            if (extended.Count == ExtendedObjectiveAwardKeys.Count)
                foreach (var entry in extended) _objectiveAwards.Add(entry.Key, entry.Value);
        }
        private void LoadLeadership(JsonElement table, JsonElement awards, JsonElement creation)
        {
            bool hasLevels = table.TryGetProperty("LeadershipLevels", out var levels);
            bool hasCreation = creation.TryGetProperty("AdditionalLeadershipPoints", out var initialPoints);
            if (!hasLevels && !hasCreation && !LeadershipAwardKeys.Values.Any(key => awards.TryGetProperty(key, out _))) return;
            if (!hasLevels || !hasCreation) throw new InvalidDataException("Incomplete leadership progression catalog.");
            InitialLeadershipPoints = initialPoints.GetUInt32();
            bool unusedRow = false;
            foreach (var entry in levels.EnumerateObject())
            {
                const string prefix = "ExperienceLeadershipLevel";
                if (!entry.Name.StartsWith(prefix, StringComparison.Ordinal) || !byte.TryParse(entry.Name.AsSpan(prefix.Length), out byte level) ||
                    entry.Name != prefix + level.ToString(System.Globalization.CultureInfo.InvariantCulture))
                    throw new InvalidDataException("Invalid leadership level ordinal.");
                if (entry.Value.EnumerateObject().Count() != 1) throw new InvalidDataException("Invalid authored leadership level definition.");
                var row = entry.Value.GetProperty(prefix);
                if (level == 0)
                {
                    if (unusedRow || row.EnumerateObject().Any()) throw new InvalidDataException("Unused leadership level zero must remain empty.");
                    unusedRow = true;
                    continue;
                }
                if (row.EnumerateObject().Count() != 2) throw new InvalidDataException("Invalid authored leadership level fields.");
                int threshold = row.GetProperty("LeadershipPoints").GetInt32();
                int minimumLevel = row.GetProperty("MinimumLevel").GetInt32();
                if (threshold < 0 || minimumLevel < 0) throw new InvalidDataException("Negative leadership requirement.");
                _leadershipLevels.Add(level, (threshold, minimumLevel));
            }
            if (!unusedRow || _leadershipLevels.Count == 0 || _leadershipLevels.Keys.Where((level, index) => level != index + 1).Any())
                throw new InvalidDataException("Leadership levels must be contiguous from level one.");
            foreach (var role in LeadershipAwardKeys)
            {
                var row = awards.GetProperty(role.Value);
                if (row.EnumerateObject().Count() != VictoryModifierKeys.Count)
                    throw new InvalidDataException("Invalid leadership award outcome fields.");
                foreach (var outcome in VictoryModifierKeys)
                {
                    float points = row.GetProperty(outcome.Value).GetSingle();
                    if (!float.IsFinite(points) || (double)points > int.MaxValue)
                        throw new InvalidDataException("Invalid leadership award.");
                    _leadershipAwards.Add((role.Key, outcome.Key), (uint)MathF.Truncate(MathF.Max(0, points)));
                }
            }
        }
        internal byte LeadershipLevelForPoints(uint points, byte characterLevel)
        {
            // Native loads the record word as-is and compares it with signed cmpw.
            int signedPoints = unchecked((int)points);
            byte level = 0;
            foreach (var row in _leadershipLevels)
            {
                if (signedPoints < row.Value.Points || characterLevel < row.Value.MinimumLevel) break;
                level = row.Key;
            }
            return level;
        }
        internal bool TryLeadershipAward(byte role, string kind, out uint points)
        {
            points = 0;
            return InitialLeadershipPoints.HasValue && IsFinalRewardKind(kind) &&
                (role == 0 || _leadershipAwards.TryGetValue((role, kind), out points));
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
            (kind is "teammate_healed" or RepairRewardKind ||
                units == 1 && (kind is "enemy_killed" or AssistRewardKind || IsFragoObjectiveKind(kind) ||
                    kind != null && (OptionalActionAwardKeys.ContainsKey(kind) || IsFinalRewardKind(kind))));
        internal static bool IsFragoSupportKind(string kind) => kind is "teammate_healed" or "teammate_resuscitated" or RepairRewardKind;
        internal static bool IsExtendedObjectiveRewardKind(string kind) =>
            kind != null && ExtendedObjectiveAwardKeys.ContainsKey(kind);
        internal static bool IsFragoObjectiveKind(string kind) =>
            kind != null && (ObjectiveAwardKeys.ContainsKey(kind) || IsExtendedObjectiveRewardKind(kind));
        internal static bool ValidFragoContext(string kind, int units, bool? frago) =>
            frago == null || frago == true &&
            (kind is "enemy_killed" or AssistRewardKind || IsFragoSupportKind(kind) || IsFragoObjectiveKind(kind)) && ValidKindUnits(kind, units);
        internal bool CanApplyFragoBonus(string kind) => kind switch {
            "teammate_resuscitated" => FragoActionBonusEnabled,
            RepairRewardKind => FragoBonusEnabled && RepairRewardsEnabled,
            AssistRewardKind => FragoBonusEnabled && AssistRewardsEnabled,
            _ => FragoBonusEnabled
        };
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
            else if (kind == AssistRewardKind)
            {
                if (_enemyKillAssist is not float award) return false;
                reward = award;
            }
            else if (kind == RepairRewardKind)
            {
                if (_experiencePerRepairUnit is not float ratio) return false;
                reward = ratio;
            }
            else if (!_objectiveAwards.TryGetValue(kind, out reward) && !_optionalActionAwards.TryGetValue(kind, out reward))
            {
                // A recognized optional action still needs a durable receipt when
                // its catalog has no authored XP entry. Settle it without credit
                // so delivery retries do not leave the gameplay outbox pending.
                // This absence policy supplies no tuning value for the event.
                if (!OptionalActionAwardKeys.ContainsKey(kind)) return false;
                reward = 0;
            }
            // Native Increment shares fractional XP carry across these award kinds.
            float total = MathF.FusedMultiplyAdd((float)units, reward, matchFraction);
            if (!float.IsFinite(total) || total < 0 || (double)total > int.MaxValue) return false;
            experience = checked((int)MathF.Truncate(total)); fraction = total - experience;
            return float.IsFinite(fraction) && fraction >= 0 && fraction < 1;
        }

        internal bool TryFragoBonus(int baseExperience, out int bonus)
        {
            bonus = 0;
            if (_fragoBonusModifier is not float modifier || baseExperience < 0) return false;
            if (baseExperience == 0) return true;
            // Native multiplies the whole base award after ordinary fractional carry has been saved.
            float product = (float)baseExperience * modifier;
            if (!float.IsFinite(product) || (double)product > int.MaxValue) return false;
            bonus = checked((int)MathF.Truncate(product));
            return true;
        }
    }
}
