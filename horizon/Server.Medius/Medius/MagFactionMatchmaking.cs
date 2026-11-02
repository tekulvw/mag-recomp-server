using System;
using System.Collections.Generic;
using System.Linq;

namespace Server.Medius
{
    public sealed record MagMatchProfile(string ProfileId, int GameType, byte AttackingFaction,
        byte DefendingFaction, int MaximumPlayers, int TeamCapacity);
    public sealed record MagQueuedHuman(string TicketId, int GameType, byte Faction, long Order);
    public sealed record MagFactionSelection(MagMatchProfile Profile, IReadOnlyList<MagQueuedHuman> Players);

    /// <summary>Pure saved-faction matchmaking. Callers supply real human tickets,
    /// persist the selected formation, and commit queue removal themselves.</summary>
    public static class MagFactionMatchmaking
    {
        public static MagFactionSelection Select(IReadOnlyList<MagQueuedHuman> waiting,
            IReadOnlyList<MagMatchProfile> profiles, int minimumPlayers, int maximumPlayers,
            int[] minimumPerTeam, Func<int, int> nextRandom)
        {
            var ordered = ValidateWaiting(waiting);
            ValidateOptions(minimumPlayers, maximumPlayers, minimumPerTeam);
            if (profiles == null) throw new ArgumentNullException(nameof(profiles));
            if (nextRandom == null) throw new ArgumentNullException(nameof(nextRandom));
            var ids = new HashSet<string>(StringComparer.Ordinal);
            foreach (var profile in profiles)
            {
                ValidateProfile(profile);
                if (!ids.Add(profile.ProfileId)) throw new ArgumentException("Duplicate match profile identity.", nameof(profiles));
            }
            // A new opposing formation always needs a human on each side, even
            // when the operator's generic startup policy permits solo matches.
            var minima = new[] { Math.Max(1, minimumPerTeam[0]), Math.Max(1, minimumPerTeam[1]) };
            int minimum = Math.Max(2, minimumPlayers);
            var feasible = profiles.Select(p => Cohort(ordered, p, minimum, maximumPlayers, minima))
                .Where(selection => selection != null).ToArray();
            if (feasible.Length == 0) return null;

            var oldest = feasible.SelectMany(selection => selection.Players)
                .OrderBy(player => player.Order).ThenBy(player => player.TicketId, StringComparer.Ordinal).First();
            var candidates = feasible.Where(selection => selection.Profile.GameType == oldest.GameType &&
                (selection.Profile.AttackingFaction == oldest.Faction || selection.Profile.DefendingFaction == oldest.Faction)).ToArray();
            var opponents = candidates.Select(selection => selection.Profile.AttackingFaction == oldest.Faction
                ? selection.Profile.DefendingFaction : selection.Profile.AttackingFaction).Distinct().OrderBy(f => f).ToArray();
            byte opponent = opponents[Draw(nextRandom, opponents.Length)];
            // Draw roles independently of map/profile count. Multiple maps with
            // the same ordered factions use the first feasible supplied profile.
            var orientations = candidates.Where(selection => selection.Profile.AttackingFaction == opponent ||
                    selection.Profile.DefendingFaction == opponent)
                .GroupBy(selection => selection.Profile.AttackingFaction == oldest.Faction)
                .OrderByDescending(group => group.Key).Select(group => group.First()).ToArray();
            return orientations[Draw(nextRandom, orientations.Length)];
        }

        /// <summary>Select oldest eligible humans for an already fixed pair.
        /// Unlike new formation, one human may late join either configured side.</summary>
        public static MagFactionSelection ForProfile(IReadOnlyList<MagQueuedHuman> waiting,
            MagMatchProfile profile, int minimumPlayers, int maximumPlayers, int[] minimumPerTeam)
        {
            var ordered = ValidateWaiting(waiting);
            ValidateOptions(minimumPlayers, maximumPlayers, minimumPerTeam);
            ValidateProfile(profile);
            return Cohort(ordered, profile, minimumPlayers, maximumPlayers, minimumPerTeam);
        }

        static MagFactionSelection Cohort(MagQueuedHuman[] ordered, MagMatchProfile profile,
            int minimumPlayers, int maximumPlayers, int[] minima)
        {
            int maximum = Math.Min(maximumPlayers, profile.MaximumPlayers);
            if (minimumPlayers > maximum || (long)minima[0] + minima[1] > maximum ||
                minima.Any(minimum => minimum > profile.TeamCapacity)) return null;
            var eligible = ordered.Where(player => player.GameType == profile.GameType &&
                (player.Faction == profile.AttackingFaction || player.Faction == profile.DefendingFaction)).ToArray();
            var roles = new[] { profile.AttackingFaction, profile.DefendingFaction };
            var selected = new List<MagQueuedHuman>();
            var ids = new HashSet<string>(StringComparer.Ordinal);
            var counts = new int[2];
            for (int team = 0; team < 2; team++)
            {
                var required = eligible.Where(player => player.Faction == roles[team]).Take(minima[team]).ToArray();
                if (required.Length < minima[team]) return null;
                selected.AddRange(required);
                foreach (var player in required) ids.Add(player.TicketId);
                counts[team] = required.Length;
            }
            foreach (var player in eligible)
            {
                if (selected.Count >= maximum) break;
                int team = player.Faction == profile.AttackingFaction ? 0 : 1;
                if (counts[team] >= profile.TeamCapacity || !ids.Add(player.TicketId)) continue;
                selected.Add(player);
                counts[team]++;
            }
            if (selected.Count < minimumPlayers) return null;
            return new MagFactionSelection(profile, Array.AsReadOnly(selected.OrderBy(player => player.Order)
                .ThenBy(player => player.TicketId, StringComparer.Ordinal).ToArray()));
        }

        static MagQueuedHuman[] ValidateWaiting(IReadOnlyList<MagQueuedHuman> waiting)
        {
            if (waiting == null) throw new ArgumentNullException(nameof(waiting));
            var ids = new HashSet<string>(StringComparer.Ordinal);
            foreach (var player in waiting)
                if (player == null || string.IsNullOrWhiteSpace(player.TicketId) || player.GameType < 0 ||
                    player.GameType > 1 || player.Faction > 2 || !ids.Add(player.TicketId))
                    throw new ArgumentException("Human tickets require unique identities, mode 0/1 and saved faction 0..2.", nameof(waiting));
            return waiting.OrderBy(player => player.Order).ThenBy(player => player.TicketId, StringComparer.Ordinal).ToArray();
        }

        static void ValidateProfile(MagMatchProfile profile)
        {
            if (profile == null) throw new ArgumentNullException(nameof(profile));
            if (string.IsNullOrWhiteSpace(profile.ProfileId) || profile.GameType < 0 || profile.GameType > 1 ||
                profile.AttackingFaction > 2 || profile.DefendingFaction > 2 ||
                profile.AttackingFaction == profile.DefendingFaction || profile.MaximumPlayers < 1 ||
                profile.MaximumPlayers > 64 || profile.TeamCapacity < 1 || profile.TeamCapacity > 32)
                throw new ArgumentException("Profiles require an identity, mode 0/1, opposing factions and bounded capacities.", nameof(profile));
        }

        static void ValidateOptions(int minimumPlayers, int maximumPlayers, int[] minimumPerTeam)
        {
            if (minimumPerTeam == null) throw new ArgumentNullException(nameof(minimumPerTeam));
            if (minimumPlayers < 1 || maximumPlayers < minimumPlayers || maximumPlayers > 64 ||
                minimumPerTeam.Length != 2 || minimumPerTeam.Any(n => n < 0 || n > maximumPlayers) ||
                (long)minimumPerTeam[0] + minimumPerTeam[1] > maximumPlayers)
                throw new ArgumentException("Match minima and maximum must fit the bounded total and two nonnegative role minima.");
        }

        static int Draw(Func<int, int> nextRandom, int count)
        {
            int result = nextRandom(count);
            if (result < 0 || result >= count)
                throw new ArgumentOutOfRangeException(nameof(nextRandom), "Random draw must lie within the requested exclusive bound.");
            return result;
        }
    }
}
