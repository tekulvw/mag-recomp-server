using System.Text.Json.Serialization;
namespace Server.Medius
{
    [JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
    public sealed record GameplayLeadershipBaseline(
        [property: JsonRequired] byte LeadershipLevel,
        [property: JsonRequired] byte Nominations,
        [property: JsonRequired] byte MatchesSinceLastCommand);

    [JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
    public sealed record GameplayLeadershipResult(
        [property: JsonRequired] byte Role,
        [property: JsonRequired] GameplayLeadershipBaseline Baseline)
    {
        internal bool IsValid => Baseline != null && Role is 0 or 1 or 2 or 4;
    }

    /// <summary>Private metadata consumed by gameplay's typed LeadershipRecord.</summary>
    internal sealed class GameplayLeadershipRecord
    {
        [JsonPropertyName("leadership_level")] public byte LeadershipLevel { get; }
        [JsonPropertyName("leadership_points"), JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public uint? LeadershipPoints { get; }
        [JsonPropertyName("nominations")] public byte Nominations { get; }
        [JsonPropertyName("matches_since_last_command")] public byte MatchesSinceLastCommand { get; }
        public GameplayLeadershipRecord(StoredLeadershipRecord stored)
            : this(stored.LeadershipLevel, stored.LeadershipPoints, stored.Nominations, stored.MatchesSinceLastCommand) { }
        public GameplayLeadershipRecord(LeadershipRewardOutcome outcome)
            : this(outcome.LeadershipLevel, outcome.LeadershipPoints, outcome.Nominations, outcome.MatchesSinceLastCommand) { }
        private GameplayLeadershipRecord(byte level, uint? points, byte nominations, byte recency)
        {
            LeadershipLevel = level;
            LeadershipPoints = points;
            Nominations = nominations;
            MatchesSinceLastCommand = recency;
        }
    }
}
