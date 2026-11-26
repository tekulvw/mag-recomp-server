using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;

namespace Server.Medius
{
    /// <summary>A running match in one world: counts only, never accounts.</summary>
    /// <param name="Bots">The profile's bot fill less the humans in the match; zero without bot fill.</param>
    public sealed record MagWorldMatchStatus(string ProfileId, int GameType, int AttackingFaction, int DefendingFaction,
        int Humans, int AttackingHumans, int DefendingHumans, int HumanCapacity, int Bots, int WorldCapacity);

    /// <summary>One configured world for the server status (<c>GET /admin/v1/status</c>).</summary>
    /// <param name="State">See <see cref="MagServerStatus.WorldState"/>.</param>
    /// <param name="GameTypes">Game types the world's profiles serve.</param>
    /// <param name="HumanCapacity">Humans the running match admits, or the most any profile admits.</param>
    /// <param name="Pending">Humans placed on this world and not admitted yet.</param>
    public sealed record MagWorldStatus(string WorkerId, string State, IReadOnlyList<int> GameTypes, int HumanCapacity,
        MagWorldMatchStatus Match, int Pending);

    /// <summary>
    /// The read-only server status the accounts portal publishes on its status page: whether each
    /// configured world admits players, what it is playing, population counts, DME ports and the
    /// deployed versions. It carries counts only: no account, character or player name.
    /// </summary>
    public static class MagServerStatus
    {
        /// <summary>Admitting players.</summary>
        public const string Open = "open";
        /// <summary>Running, but the operator closed it to new players (registry <c>draining</c>).</summary>
        public const string Draining = "draining";
        /// <summary>Closed after a worker fault (unreachable at start, a failed tick, a foreign or
        /// refused incarnation, or the adoption cap); it reopens only when a matching worker is adopted.</summary>
        public const string Faulted = "faulted";
        /// <summary>Not faulted, but not ready for gameplay: starting, or its world stream lapsed.</summary>
        public const string Unavailable = "unavailable";

        static readonly Lazy<DateTime> ProcessStart = new(() =>
        {
            try { return System.Diagnostics.Process.GetCurrentProcess().StartTime.ToUniversalTime(); }
            catch (Exception error) when (error is InvalidOperationException || error is NotSupportedException ||
                error is System.ComponentModel.Win32Exception) { return DateTime.UtcNow; }
        });

        public static string WorldState(bool faulted, bool ready, bool draining) =>
            faulted ? Faulted : !ready ? Unavailable : draining ? Draining : Open;

        /// <summary>One world from its host's placement state.</summary>
        public static MagWorldStatus Describe(string workerId, bool faulted, LocalDmeWorkerPlacement placement)
        {
            ArgumentNullException.ThrowIfNull(placement);
            var profiles = placement.Profiles ?? Array.Empty<MagMatchProfile>();
            // Up to 64 the configured row also bounds humans; larger worlds keep their own caps beneath it.
            int rowHumans = placement.StartupPolicy?.MaximumPlayers ?? int.MaxValue;
            int Humans(MagMatchProfile profile) => Math.Min(profile.HumanCapacity, rowHumans);
            MagWorldMatchStatus match = null;
            if (placement.CurrentProfile is MagMatchProfile current)
            {
                int Side(byte faction) => placement.LiveTeamCounts != null && faction < placement.LiveTeamCounts.Count
                    ? placement.LiveTeamCounts[faction] : 0;
                int humans = placement.LiveCount;
                match = new MagWorldMatchStatus(current.ProfileId, current.GameType, current.AttackingFaction, current.DefendingFaction,
                    humans, Side(current.AttackingFaction), Side(current.DefendingFaction), Humans(current),
                    current.BotFillTotal is int fill ? Math.Max(0, fill - humans) : 0, current.MaximumPlayers);
            }
            int capacity = match?.HumanCapacity ?? (profiles.Count == 0 ? 0 : profiles.Max(Humans));
            return new MagWorldStatus(workerId, WorldState(faulted, placement.Ready, placement.Draining),
                Array.AsReadOnly(profiles.Select(p => p.GameType).Distinct().OrderBy(t => t).ToArray()),
                capacity, match, placement.PendingQueueCount);
        }

        /// <summary>Distinct accounts holding a live login lease in any marker; null when none can be read.</summary>
        public static int? PlayersOnline(string markerDirectory, DateTime now)
        {
            if (string.IsNullOrEmpty(markerDirectory)) return null;
            try { return MagActiveAccounts.Read(markerDirectory, now)?.Count; }
            catch (Exception error) when (error is IOException || error is UnauthorizedAccessException ||
                error is InvalidDataException || error is System.Text.Json.JsonException || error is KeyNotFoundException ||
                error is FormatException || error is InvalidOperationException || error is ArgumentException)
            { return null; }
        }

        /// <summary>The status of this process: the configured registry, else the single local world.</summary>
        public static object Capture(MagWorldRegistry registry, LocalDmeHost single, MagDmePortBudget ports, string markerDirectory, DateTime now)
        {
            string mode;
            IReadOnlyList<MagWorldStatus> worlds;
            int[] queued;
            if (registry != null)
            {
                mode = "registry";
                worlds = registry.StatusWorlds();
                queued = registry.QueuedByGameType();
            }
            else if (single != null)
            {
                mode = "single";
                worlds = new[] { Describe("gameplay", false, single.GetPlacement()) };
                queued = new int[3];
            }
            else
            {
                mode = "none";
                worlds = Array.Empty<MagWorldStatus>();
                queued = new int[3];
            }
            string mediusVersion = null, gameplayVersion = null;
            try { var news = MagServerNews.Current; mediusVersion = news.MediusVersion; gameplayVersion = news.GameplayVersion; }
            catch (Exception error) when (error is InvalidDataException || error is IOException || error is UnauthorizedAccessException) { }
            return Report(mode, worlds, queued, PlayersOnline(markerDirectory, now), ports ?? MagDmePortBudget.Unbounded(),
                mediusVersion, gameplayVersion, ProcessStart.Value, now);
        }

        /// <summary>The <c>GET /admin/v1/status</c> body.</summary>
        public static object Report(string mode, IReadOnlyList<MagWorldStatus> worlds, IReadOnlyList<int> queuedByGameType, int? playersOnline,
            MagDmePortBudget ports, string mediusVersion, string gameplayVersion, DateTime startedAt, DateTime now)
        {
            ArgumentNullException.ThrowIfNull(worlds);
            ArgumentNullException.ThrowIfNull(queuedByGameType);
            ArgumentNullException.ThrowIfNull(ports);
            return new
            {
                medius_version = mediusVersion,
                gameplay_version = gameplayVersion,
                started_at = startedAt.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", System.Globalization.CultureInfo.InvariantCulture),
                uptime_seconds = (long)Math.Max(0, (now.ToUniversalTime() - startedAt.ToUniversalTime()).TotalSeconds),
                mode,
                players_online = playersOnline,
                queued = queuedByGameType.Sum(),
                queued_by_game_type = queuedByGameType,
                dme_ports = new { capacity = ports.Capacity, in_use = ports.InUse },
                worlds = worlds.Select(world => new
                {
                    worker_id = world.WorkerId,
                    state = world.State,
                    game_types = world.GameTypes,
                    human_capacity = world.HumanCapacity,
                    pending = world.Pending,
                    match = world.Match == null ? null : (object)new
                    {
                        profile_id = world.Match.ProfileId,
                        game_type = world.Match.GameType,
                        attacking_faction = world.Match.AttackingFaction,
                        defending_faction = world.Match.DefendingFaction,
                        humans = world.Match.Humans,
                        attacking_humans = world.Match.AttackingHumans,
                        defending_humans = world.Match.DefendingHumans,
                        human_capacity = world.Match.HumanCapacity,
                        bots = world.Match.Bots,
                        world_capacity = world.Match.WorldCapacity,
                    },
                }).ToArray(),
            };
        }
    }
}
