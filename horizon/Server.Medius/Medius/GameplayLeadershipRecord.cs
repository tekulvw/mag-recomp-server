using System.Text.Json.Serialization;
namespace Server.Medius
{
    /// <summary>Private admission metadata consumed by gameplay's typed LeadershipRecord.</summary>
    internal sealed class GameplayLeadershipRecord
    {
        [JsonPropertyName("leadership_level")] public byte LeadershipLevel { get; }
        [JsonPropertyName("nominations")] public byte Nominations { get; }
        [JsonPropertyName("matches_since_last_command")] public byte MatchesSinceLastCommand { get; }
        public GameplayLeadershipRecord(StoredLeadershipRecord stored)
        {
            LeadershipLevel = stored.LeadershipLevel;
            Nominations = stored.Nominations;
            MatchesSinceLastCommand = stored.MatchesSinceLastCommand;
        }
    }
}
