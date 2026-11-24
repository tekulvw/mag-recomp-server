using System;
using System.Collections.Generic;
using System.Linq;

namespace Server.Medius
{
    /// <summary>What one configured worker incarnation offers the registry: its whole advertised
    /// match-profile matrix and the capabilities that change what the registry itself offers.
    /// A restarted worker is adopted only when its contract equals the entry's established one.</summary>
    public sealed class MagWorkerContract
    {
        /// <summary>Capabilities that change registry-wide placement, parties or loadout behaviour.
        /// Other capabilities (reward kinds, event replay and the like) are read again by the
        /// adopted host for its own admissions, exactly as at startup.</summary>
        public static readonly IReadOnlyList<string> RegistryCapabilities = Array.AsReadOnly(new[]
        {
            "worker_registry_v1", "match_profiles_v1", "independent_match_cohorts_v1",
            "committed_loadout_selection_v1", "active_loadout_refresh_v1", "loadout_layout_revision_v1",
            "bot_fill_v1", "party_reservation_v1", "party_late_entry_v1"
        });

        public IReadOnlyList<string> Profiles { get; }
        public IReadOnlyList<string> Capabilities { get; }

        MagWorkerContract(IEnumerable<string> profiles, IEnumerable<string> capabilities)
        {
            Profiles = Array.AsReadOnly(profiles.OrderBy(p => p, StringComparer.Ordinal).ToArray());
            Capabilities = Array.AsReadOnly(capabilities.OrderBy(c => c, StringComparer.Ordinal).ToArray());
        }

        public static MagWorkerContract From(GameplayBridgeResponse hello)
        {
            ArgumentNullException.ThrowIfNull(hello);
            return new MagWorkerContract(hello.MatchProfiles.Select(Describe),
                hello.Capabilities.Where(c => RegistryCapabilities.Contains(c, StringComparer.Ordinal)).Distinct(StringComparer.Ordinal));
        }

        static string Describe(MagMatchProfile p) =>
            $"{p.ProfileId}:type={p.GameType}:{p.AttackingFaction}v{p.DefendingFaction}:max={p.MaximumPlayers}:team={p.TeamCapacity}" +
            $":squad={p.SquadCapacity?.ToString() ?? "-"}:squads={(p.SquadCounts == null ? "-" : string.Join(",", p.SquadCounts))}" +
            $":bots={p.BotFillTotal?.ToString() ?? "-"}";

        /// <summary>Null when both contracts are identical; otherwise a short, loggable description.</summary>
        public string Difference(MagWorkerContract other)
        {
            if (other == null) return "contract=missing";
            var parts = new List<string>();
            void Compare(string name, IReadOnlyList<string> mine, IReadOnlyList<string> theirs)
            {
                var missing = mine.Except(theirs, StringComparer.Ordinal).ToArray();
                var added = theirs.Except(mine, StringComparer.Ordinal).ToArray();
                if (missing.Length > 0) parts.Add($"{name}_missing={missing.Length}[{string.Join(";", missing.Take(3))}{(missing.Length > 3 ? ";..." : "")}]");
                if (added.Length > 0) parts.Add($"{name}_added={added.Length}[{string.Join(";", added.Take(3))}{(added.Length > 3 ? ";..." : "")}]");
            }
            Compare("profiles", Profiles, other.Profiles);
            Compare("capabilities", Capabilities, other.Capabilities);
            return parts.Count == 0 ? null : string.Join(" ", parts);
        }
    }

    /// <summary>Receipts (rewards and loadout selections) a worker reported that Horizon has not
    /// yet settled durably. Only what a worker reported can be listed here; a receipt it never
    /// reported is unknown to Horizon. Callers hold the owning host's gate.</summary>
    sealed class GameplayReceiptLedger
    {
        public const int Capacity = 4096;
        public sealed record Receipt(string Kind, string Id, string Account, uint Character, string Amount, string Match, bool Reward);
        readonly Dictionary<string, Receipt> _open = new(StringComparer.Ordinal);
        readonly HashSet<string> _settled = new(StringComparer.Ordinal);
        readonly Queue<string> _settledOrder = new();
        readonly HashSet<string> _untracked = new(StringComparer.Ordinal);

        public int Count => _open.Count;

        public void Observe(string key, Receipt receipt)
        {
            if (_settled.Contains(key) || _open.ContainsKey(key)) return;
            if (_open.Count >= Capacity) { if (_untracked.Count < Capacity) _untracked.Add(key); return; }
            _open.Add(key, receipt);
        }

        public void Settle(string key)
        {
            _open.Remove(key);
            _untracked.Remove(key);
            if (!_settled.Add(key)) return;
            _settledOrder.Enqueue(key);
            while (_settledOrder.Count > 2 * Capacity) _settled.Remove(_settledOrder.Dequeue());
        }

        /// <summary>Remove and return every unsettled receipt, plus the count that exceeded capacity.</summary>
        public (IReadOnlyList<Receipt> Open, int Untracked) Drain()
        {
            var open = _open.Values.ToArray();
            int untracked = _untracked.Count;
            _open.Clear();
            _untracked.Clear();
            return (Array.AsReadOnly(open), untracked);
        }

        public static string RewardKey(GameplayReward reward) => "reward:" + reward.RewardId;
        public static string SelectionKey(GameplayLoadoutSelection selection) => $"selection:{selection.SelectionId}@{selection.Revision}";
    }
}
