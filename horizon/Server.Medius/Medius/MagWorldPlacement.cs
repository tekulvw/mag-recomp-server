using System;
using System.Collections.Generic;
using System.Linq;

namespace Server.Medius
{
    public sealed record MagWorldPlacementState(string WorkerId, bool Ready, bool Draining,
        int MinimumPlayers, int MaximumPlayers, int[] MinimumPlayersPerTeam, IReadOnlyList<MagMatchProfile> Profiles,
        MagMatchProfile CurrentProfile, IReadOnlyList<int> LiveFactionCounts, IReadOnlyList<int> PendingFactionCounts,
        int LiveCount, int PendingCount, bool CanForm);
    public sealed record MagWorldPlacementDecision(string WorkerId, MagMatchProfile Profile,
        IReadOnlyList<MagQueuedHuman> Players, bool NewFormation);

    /// <summary>Pure placement over already leased, owned world snapshots.</summary>
    public static class MagWorldPlacement
    {
        public static MagWorldPlacementDecision PlanNext(IReadOnlyList<MagQueuedHuman> humans,
            IReadOnlyList<MagWorldPlacementState> worlds, Func<int, int> random)
        {
            MagWorldPlacementDecision fresh = null;
            if (humans.Any(human => human.Size > 1))
            {
                fresh = PlanNewFormation(humans, worlds, random);
                if (fresh?.Players.Any(human => human.Size > 1) == true) return fresh;
            }

            foreach (var world in worlds.Where(w => w.Ready && !w.Draining && w.CurrentProfile != null)
                .OrderByDescending(w => w.LiveCount + w.PendingCount))
            {
                var profile = world.CurrentProfile;
                int room = Math.Min(world.MaximumPlayers, profile.MaximumPlayers) - world.LiveCount - world.PendingCount;
                var counts = Enumerable.Range(0, 3).Select(f => world.LiveFactionCounts[f] + world.PendingFactionCounts[f]).ToArray();
                var selected = new List<MagQueuedHuman>();
                foreach (var human in humans.OrderBy(h => h.Order))
                {
                    if (selected.Count >= room) break;
                    if (human.Size != 1 || human.GameType != profile.GameType || (human.Faction != profile.AttackingFaction && human.Faction != profile.DefendingFaction) ||
                        counts[human.Faction] >= profile.TeamCapacity) continue;
                    selected.Add(human); counts[human.Faction]++;
                }
                if (selected.Count > 0) return new(world.WorkerId, profile, selected.AsReadOnly(), false);
            }
            return fresh ?? PlanNewFormation(humans, worlds, random);
        }

        static MagWorldPlacementDecision PlanNewFormation(IReadOnlyList<MagQueuedHuman> humans,
            IReadOnlyList<MagWorldPlacementState> worlds, Func<int, int> random)
        {
            var candidates = new List<MagWorldPlacementDecision>();
            foreach (var world in worlds.Where(w => w.Ready && !w.Draining && w.CanForm))
            {
                var selection = MagFactionMatchmaking.Select(humans, world.Profiles, world.MinimumPlayers,
                    world.MaximumPlayers, world.MinimumPlayersPerTeam, random);
                if (selection != null) candidates.Add(new(world.WorkerId, selection.Profile, selection.Players, true));
            }
            // Stable config order breaks equal oldest-faction placement ties.
            return candidates.OrderBy(p => p.Players.Min(h => h.Order)).FirstOrDefault();
        }
    }
}
