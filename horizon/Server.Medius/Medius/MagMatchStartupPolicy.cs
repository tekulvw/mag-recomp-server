using System;
using System.Linq;

namespace Server.Medius
{
    /// <summary>Operator startup policy. Gameplay acknowledges team feasibility
    /// against its loaded match catalog before Horizon opens any queues.</summary>
    public sealed class MagMatchStartupPolicy
    {
        readonly int[] _minimumPerTeam;
        public int MinimumPlayers { get; }
        public int MaximumPlayers { get; }
        public MagMatchStartupPolicy(int minimumPlayers, int maximumPlayers, int[] minimumPerTeam)
        {
            _minimumPerTeam = minimumPerTeam == null ? new[] { 0, 0 } : (int[])minimumPerTeam.Clone();
            if (maximumPlayers < 1 || maximumPlayers > 64 || minimumPlayers < 1 || minimumPlayers > maximumPlayers ||
                _minimumPerTeam.Length != 2 || _minimumPerTeam.Any(n => n < 0 || n > maximumPlayers) ||
                (long)_minimumPerTeam[0] + _minimumPerTeam[1] > maximumPlayers)
                throw new ArgumentException("Startup minima require a positive total and two nonnegative team counts within match capacity.");
            MinimumPlayers = Math.Max(minimumPlayers, _minimumPerTeam.Sum());
            MaximumPlayers = maximumPlayers;
        }
        public object ConfigureRequest(string requestId, int applicationId) => new
        {
            v = 2, request_id = requestId, op = "configure_match", application_id = applicationId,
            minimum_players = MinimumPlayers, maximum_players = MaximumPlayers,
            minimum_players_per_team = (int[])_minimumPerTeam.Clone()
        };
    }
}
