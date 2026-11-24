using System;
using System.Collections.Generic;
using System.Linq;

namespace Server.Medius
{
    /// <param name="BotFillTotal">World population the worker fills with bots when it advertises
    /// <c>bot_fill_v1</c>; such a profile may start without a human on each side.</param>
    public sealed record MagMatchProfile(string ProfileId, int GameType, byte AttackingFaction,
        byte DefendingFaction, int MaximumPlayers, int TeamCapacity, int? SquadCapacity = null, IReadOnlyList<int> SquadCounts = null,
        int? BotFillTotal = null)
    {
        public bool BotFill => BotFillTotal.HasValue;
    }
    public sealed record MagQueuedHuman(string TicketId, int GameType, byte Faction, long Order, int Size = 1);
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
            // A new opposing formation needs a human on each side, even when the
            // operator's generic startup policy permits solo matches, unless the
            // worker fills that profile with bots. A bot-filled profile uses the
            // operator's minima as configured, so one human may start it.
            var feasible = profiles.Select(p => Cohort(ordered, p, FormationMinimum(p, minimumPlayers),
                    maximumPlayers, FormationMinima(p, minimumPerTeam)))
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

        /// <summary>Human total required to form a new match on this profile.</summary>
        public static int FormationMinimum(MagMatchProfile profile, int configured) =>
            profile?.BotFill == true ? Math.Max(1, configured) : Math.Max(2, configured);

        /// <summary>Per-side human minima required to form a new match on this profile.</summary>
        public static int[] FormationMinima(MagMatchProfile profile, IReadOnlyList<int> configured) =>
            profile?.BotFill == true
                ? new[] { Math.Max(0, configured[0]), Math.Max(0, configured[1]) }
                : new[] { Math.Max(1, configured[0]), Math.Max(1, configured[1]) };

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
            if (eligible.Any(player => player.Size != 1))
                return GroupedCohort(eligible, profile, minimumPlayers, maximum, minima);

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

        static MagFactionSelection GroupedCohort(MagQueuedHuman[] eligible, MagMatchProfile profile,
            int minimum, int maximum, int[] minima)
        {
            var units = eligible.Where(player => player.Size == 1 || player.Size <= profile.SquadCapacity.GetValueOrDefault()).ToArray();
            int attackingLimit = profile.SquadCounts?[0] ?? 0;
            int defendingLimit = profile.SquadCounts?[1] ?? 0;
            var attackers = new ulong[units.Length + 1, attackingLimit + 1];
            var defenders = new ulong[units.Length + 1, defendingLimit + 1];
            for (int cap = 0; cap <= attackingLimit; cap++) attackers[units.Length, cap] = 1;
            for (int cap = 0; cap <= defendingLimit; cap++) defenders[units.Length, cap] = 1;
            for (int i = units.Length - 1; i >= 0; i--)
            {
                var unit = units[i];
                bool attacker = unit.Faction == profile.AttackingFaction;
                void AddUnit(ulong[,] counts, int limit, bool belongs)
                {
                    for (int cap = 0; cap <= limit; cap++)
                    {
                        ulong possible = counts[i + 1, cap];
                        int cost = unit.Size == 1 ? 0 : 1;
                        if (belongs && cap >= cost) possible |= counts[i + 1, cap - cost] << unit.Size;
                        counts[i, cap] = possible;
                    }
                }
                AddUnit(attackers, attackingLimit, attacker);
                AddUnit(defenders, defendingLimit, !attacker);
            }

            bool Fits(ulong leftCounts, ulong rightCounts, int a, int d)
            {
                for (int left = Math.Max(0, minima[0] - a); left <= profile.TeamCapacity - a; left++)
                {
                    if ((leftCounts & (1UL << left)) == 0) continue;
                    int low = Math.Max(0, Math.Max(minima[1] - d, minimum - a - d - left));
                    int high = Math.Min(profile.TeamCapacity - d, maximum - a - d - left);
                    if (high < low) continue;
                    ulong mask = ((1UL << (high + 1)) - 1) ^ ((1UL << low) - 1);
                    if ((rightCounts & mask) != 0) return true;
                }
                return false;
            }

            // Reserve parties before solos and budget each party a distinct eligible squad.
            bool CanFinish(int index, int a, int d, int attackingSquads, int defendingSquads) =>
                Fits(attackers[index, attackingSquads], defenders[index, defendingSquads], a, d);

            if (!CanFinish(0, 0, 0, attackingLimit, defendingLimit)) return null;
            var selected = new List<MagQueuedHuman>();
            int a = 0, d = 0;
            int attackingSquads = attackingLimit, defendingSquads = defendingLimit;
            for (int i = 0; i < units.Length; i++)
            {
                var unit = units[i];
                bool attacker = unit.Faction == profile.AttackingFaction;
                int nextA = a + (attacker ? unit.Size : 0);
                int nextD = d + (attacker ? 0 : unit.Size);
                int nextAttackingSquads = attackingSquads - (attacker && unit.Size > 1 ? 1 : 0);
                int nextDefendingSquads = defendingSquads - (!attacker && unit.Size > 1 ? 1 : 0);
                if (nextAttackingSquads < 0 || nextDefendingSquads < 0 ||
                    !CanFinish(i + 1, nextA, nextD, nextAttackingSquads, nextDefendingSquads)) continue;
                selected.Add(unit);
                a = nextA; d = nextD;
                attackingSquads = nextAttackingSquads;
                defendingSquads = nextDefendingSquads;
            }
            return new MagFactionSelection(profile, selected.AsReadOnly());
        }

        static MagQueuedHuman[] ValidateWaiting(IReadOnlyList<MagQueuedHuman> waiting)
        {
            if (waiting == null) throw new ArgumentNullException(nameof(waiting));
            var ids = new HashSet<string>(StringComparer.Ordinal);
            foreach (var player in waiting)
                if (player == null || string.IsNullOrWhiteSpace(player.TicketId) || player.GameType < 0 ||
                    player.GameType > 2 || player.Faction > 2 || player.Size < 1 || player.Size > 32 || !ids.Add(player.TicketId))
                    throw new ArgumentException("Human tickets require unique identities, mode 0..2 and saved faction 0..2.", nameof(waiting));
            return waiting.OrderBy(player => player.Order).ThenBy(player => player.TicketId, StringComparer.Ordinal).ToArray();
        }

        static void ValidateProfile(MagMatchProfile profile)
        {
            if (profile == null) throw new ArgumentNullException(nameof(profile));
            if (string.IsNullOrWhiteSpace(profile.ProfileId) || profile.GameType < 0 || profile.GameType > 2 ||
                profile.AttackingFaction > 2 || profile.DefendingFaction > 2 ||
                profile.AttackingFaction == profile.DefendingFaction || profile.MaximumPlayers < 1 ||
                profile.MaximumPlayers > 64 || profile.TeamCapacity < 1 || profile.TeamCapacity > 32 ||
                (profile.SquadCapacity.HasValue && (profile.SquadCapacity < 1 || profile.SquadCapacity > profile.TeamCapacity)) ||
                (profile.SquadCounts != null && (profile.SquadCounts.Count != 2 || profile.SquadCounts.Any(n => n < 1 || n > 32))) ||
                (profile.BotFillTotal.HasValue && (profile.BotFillTotal < 2 || profile.BotFillTotal > profile.MaximumPlayers)))
                throw new ArgumentException("Profiles require an identity, mode 0..2, opposing factions and bounded capacities.", nameof(profile));
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
