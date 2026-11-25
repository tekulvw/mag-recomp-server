using System;
using System.Collections.Generic;
using System.Linq;

namespace Server.Medius
{
    public sealed record MagWorldPlacementState(string WorkerId, bool Ready, bool Draining,
        int MinimumPlayers, int MaximumPlayers, int[] MinimumPlayersPerTeam, IReadOnlyList<MagMatchProfile> Profiles,
        MagMatchProfile CurrentProfile, IReadOnlyList<int> LiveFactionCounts, IReadOnlyList<int> PendingFactionCounts,
        int LiveCount, int PendingCount, bool CanForm, bool AcceptsLateParties = false);
    public sealed record MagWorldPlacementDecision(string WorkerId, MagMatchProfile Profile,
        IReadOnlyList<MagQueuedHuman> Players, bool NewFormation);

    /// <summary>Pure placement over already leased, owned world snapshots.</summary>
    public static class MagWorldPlacement
    {
        /// <param name="humanRoom">Humans that can still get a DME UDP port across
        /// every world, or null when ports are unbounded.</param>
        public static MagWorldPlacementDecision PlanNext(IReadOnlyList<MagQueuedHuman> humans,
            IReadOnlyList<MagWorldPlacementState> worlds, Func<int, int> random, int? humanRoom = null)
        {
            if (humanRoom is int free)
            {
                if (free <= 0) return null;
                // Ports go out in queue order. The oldest party that does not fit
                // the free ports keeps them for itself: humans queued after it
                // wait, so ports freed one at a time cannot starve the group.
                var blocked = humans.Where(human => human.Size > free).OrderBy(human => human.Order).FirstOrDefault();
                if (blocked != null) humans = humans.Where(human => human.Order < blocked.Order).ToArray();
                worlds = worlds.Select(world => world with { MaximumPlayers = Math.Min(world.MaximumPlayers,
                    world.LiveCount + world.PendingCount + free) }).ToArray();
            }
            // A waiting party joins a running compatible match first.
            var late = PlanLateParty(humans, worlds);
            if (late != null) return late;
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
                int room = Math.Min(world.MaximumPlayers, profile.HumanCapacity) - world.LiveCount - world.PendingCount;
                var counts = Enumerable.Range(0, 3).Select(f => world.LiveFactionCounts[f] + world.PendingFactionCounts[f]).ToArray();
                var selected = new List<MagQueuedHuman>();
                foreach (var human in humans.OrderBy(h => h.Order))
                {
                    if (selected.Count >= room) break;
                    if (human.Size != 1 || human.GameType != profile.GameType || (human.Faction != profile.AttackingFaction && human.Faction != profile.DefendingFaction) ||
                        counts[human.Faction] >= profile.HumanTeamCapacity) continue;
                    selected.Add(human); counts[human.Faction]++;
                }
                if (selected.Count > 0) return new(world.WorkerId, profile, selected.AsReadOnly(), false);
            }
            return fresh ?? PlanNewFormation(humans, worlds, random);
        }

        /// <summary>The oldest waiting party that fits a running world's side, in
        /// the fullest such world. The worker picks its squad and displaces bots.</summary>
        static MagWorldPlacementDecision PlanLateParty(IReadOnlyList<MagQueuedHuman> humans,
            IReadOnlyList<MagWorldPlacementState> worlds)
        {
            var running = worlds.Where(w => w.Ready && !w.Draining && w.AcceptsLateParties && w.CurrentProfile != null)
                .OrderByDescending(w => w.LiveCount + w.PendingCount).ToArray();
            if (running.Length == 0) return null;
            foreach (var party in humans.Where(human => human.Size > 1).OrderBy(human => human.Order))
                foreach (var world in running)
                {
                    var profile = world.CurrentProfile;
                    int room = Math.Min(world.MaximumPlayers, profile.HumanCapacity) - world.LiveCount - world.PendingCount;
                    int side = world.LiveFactionCounts[party.Faction] + world.PendingFactionCounts[party.Faction];
                    if (party.GameType != profile.GameType ||
                        (party.Faction != profile.AttackingFaction && party.Faction != profile.DefendingFaction) ||
                        profile.SquadCapacity is not int squad || party.Size > squad || party.Size > room ||
                        side + party.Size > profile.HumanTeamCapacity) continue;
                    return new(world.WorkerId, profile, new[] { party }, false);
                }
            return null;
        }

        static MagWorldPlacementDecision PlanNewFormation(IReadOnlyList<MagQueuedHuman> humans,
            IReadOnlyList<MagWorldPlacementState> worlds, Func<int, int> random)
        {
            var candidates = new List<MagWorldPlacementDecision>();
            // A port-capped world whose minima no longer fit cannot form.
            foreach (var world in worlds.Where(w => w.Ready && !w.Draining && w.CanForm &&
                w.MaximumPlayers >= w.MinimumPlayers && w.MinimumPlayersPerTeam.Sum() <= w.MaximumPlayers))
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
