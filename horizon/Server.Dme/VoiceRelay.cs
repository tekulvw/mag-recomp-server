using System;
using System.Collections.Generic;
using System.Linq;

namespace Server.Dme
{
    // In-match voice relay (docs/superpowers/specs/2026-11-25-voice-relay-design.md). Gameplay
    // publishes each world's relay table on the world stream as a voice_groups frame; the DME
    // decides from it who receives a talker's voice packet. Nothing here touches traffic yet:
    // the transport message that carries voice packets is still being captured in the lab, so no
    // packet path calls VoiceRelay.Decide. See LocalDmeHost.DecideVoiceRelay for the seam.

    /// <summary>Relay group numbers, as the client computes them from its side and world squad
    /// index: team = side (0, 1), company = 2 + side, platoon = 4 + platoon, platoon broadcast =
    /// 12 + platoon, squad = 20 + squad (0..31). Party groups 52..82 are never served; 83..338
    /// are reserved for proximity.</summary>
    public static class VoiceGroupNumbers
    {
        public const int FirstSquad = 20, LastSquad = 51;
        public const int FirstParty = 52, LastParty = 82;
        public const int FirstProximity = 83, LastProximity = 338;
        public const int MaxListenGroups = 8, MaxTransmitGroups = 8;
        public static bool IsSquad(int group) => group >= FirstSquad && group <= LastSquad;
        public static bool IsParty(int group) => group >= FirstParty && group <= LastParty;
        public static bool IsProximity(int group) => group >= FirstProximity && group <= LastProximity;
        /// <summary>A group a voice_groups frame may name: a static tier or a proximity group.</summary>
        public static bool IsRelayGroup(int group) => (group >= 0 && group <= LastSquad) || IsProximity(group);
    }

    /// <summary>Which groups the relay serves. The first rollout relays squad chat only.</summary>
    public enum VoiceRelayTier { Squad, All }

    /// <summary>MAG_VOICE_RELAY=1 turns the relay on (off by default); MAG_VOICE_RELAY_TIER is
    /// "squad" (default) or "all" (every static tier).</summary>
    public sealed class VoiceRelayOptions
    {
        public const string Variable = "MAG_VOICE_RELAY";
        public const string TierVariable = "MAG_VOICE_RELAY_TIER";
        /// <summary>Gameplay hello capability; a worker with it sends voice_groups on request.</summary>
        public const string Capability = "voice_groups_v1";
        /// <summary>Nominal voice byte rate: one 37-byte block every 40 ms (the game's 7300 bps port rate).</summary>
        public const int NominalBytesPerSecond = 925;
        /// <summary>Per-talker budget: twice the nominal rate.</summary>
        public const int BudgetBytesPerSecond = 2 * NominalBytesPerSecond;

        public static readonly VoiceRelayOptions Off = new VoiceRelayOptions(false, VoiceRelayTier.Squad);
        public static VoiceRelayOptions Current { get; set; } = FromEnvironment(Environment.GetEnvironmentVariable);

        public bool Enabled { get; }
        public VoiceRelayTier Tier { get; }

        public VoiceRelayOptions(bool enabled, VoiceRelayTier tier) { Enabled = enabled; Tier = tier; }

        /// <summary>Unknown tier values fail clearly instead of widening the relay.</summary>
        public static VoiceRelayOptions FromEnvironment(Func<string, string> environment)
        {
            bool enabled = environment?.Invoke(Variable) == "1";
            string tier = environment?.Invoke(TierVariable);
            VoiceRelayTier parsed;
            if (string.IsNullOrEmpty(tier) || tier == "squad") parsed = VoiceRelayTier.Squad;
            else if (tier == "all") parsed = VoiceRelayTier.All;
            else throw new ArgumentException($"{TierVariable} must be squad or all.");
            return new VoiceRelayOptions(enabled, parsed);
        }

        public bool Serves(int group) =>
            !VoiceGroupNumbers.IsParty(group) &&
            (Tier == VoiceRelayTier.All ? group >= 0 && group <= VoiceGroupNumbers.LastSquad : VoiceGroupNumbers.IsSquad(group));
    }

    /// <summary>One human DME client's sets in a voice_groups frame.</summary>
    public sealed record VoiceGroupsMember(string ReservationId, ushort PlayerId, byte Side,
        IReadOnlyList<int> Listen, IReadOnlyList<int> Transmit);

    /// <summary>A voice_groups world-stream frame: the full relay table at one revision.</summary>
    public sealed record VoiceGroupsFrame(ulong Revision, IReadOnlyList<VoiceGroupsMember> Members);

    /// <summary>A world's relay table: the latest voice_groups frame, by DME client index.</summary>
    public sealed class VoiceRelayTable
    {
        readonly Dictionary<int, VoiceGroupsMember> _members;
        public ulong Revision { get; }
        public IReadOnlyCollection<VoiceGroupsMember> Members => _members.Values;

        public VoiceRelayTable(VoiceGroupsFrame frame)
        {
            if (frame == null) throw new ArgumentNullException(nameof(frame));
            Revision = frame.Revision;
            _members = new Dictionary<int, VoiceGroupsMember>();
            foreach (var member in frame.Members)
                if (!_members.TryAdd(member.PlayerId, member)) throw new ArgumentException("Duplicate voice_groups member.", nameof(frame));
        }

        public VoiceGroupsMember Find(int playerId) => _members.TryGetValue(playerId, out var member) ? member : null;
    }

    public enum VoiceDropReason { None, RelayOff, Unbound, Muted, NotAllowedGroup, OverBudget }

    public sealed class VoiceRelayDecision
    {
        public VoiceDropReason Drop { get; }
        /// <summary>DME client indices to deliver to, ascending. Empty when dropped.</summary>
        public IReadOnlyList<int> Recipients { get; }
        public bool Relayed => Drop == VoiceDropReason.None;
        VoiceRelayDecision(VoiceDropReason drop, IReadOnlyList<int> recipients) { Drop = drop; Recipients = recipients; }
        public static VoiceRelayDecision Dropped(VoiceDropReason reason) => new VoiceRelayDecision(reason, Array.Empty<int>());
        public static VoiceRelayDecision Relay(IReadOnlyList<int> recipients) => new VoiceRelayDecision(VoiceDropReason.None, recipients);
        public static string Name(VoiceDropReason reason) => reason switch
        {
            VoiceDropReason.RelayOff => "relay_off",
            VoiceDropReason.Unbound => "unbound",
            VoiceDropReason.Muted => "muted",
            VoiceDropReason.NotAllowedGroup => "not_allowed_group",
            VoiceDropReason.OverBudget => "over_budget",
            _ => "relayed",
        };
    }

    /// <summary>Per-talker byte budget: a bucket that holds one second at the budget rate and
    /// refills continuously. Not thread-safe; the caller holds its world's lock.</summary>
    public sealed class VoiceTalkerBudget
    {
        readonly int _bytesPerSecond;
        readonly Dictionary<int, (double tokens, long atMs)> _talkers = new Dictionary<int, (double, long)>();

        public VoiceTalkerBudget(int bytesPerSecond = VoiceRelayOptions.BudgetBytesPerSecond)
        {
            if (bytesPerSecond <= 0) throw new ArgumentOutOfRangeException(nameof(bytesPerSecond));
            _bytesPerSecond = bytesPerSecond;
        }

        public bool TryConsume(int talker, int bytes, long nowMs)
        {
            if (bytes < 0) return false;
            double tokens = _bytesPerSecond;
            if (_talkers.TryGetValue(talker, out var state))
                tokens = Math.Min(_bytesPerSecond, state.tokens + Math.Max(0, nowMs - state.atMs) * _bytesPerSecond / 1000.0);
            bool allowed = bytes <= tokens;
            _talkers[talker] = (allowed ? tokens - bytes : tokens, nowMs);
            return allowed;
        }

        /// <summary>A departed client's slot starts afresh for its next occupant.</summary>
        public void Forget(int talker) => _talkers.Remove(talker);
    }

    /// <summary>Per-world counters for the periodic world log line.</summary>
    public sealed class VoiceRelayCounters
    {
        public long ReceivedPackets, ReceivedBytes, RelayedPackets, RelayedBytes;
        readonly Dictionary<VoiceDropReason, long> _dropped = new Dictionary<VoiceDropReason, long>();
        public long Dropped(VoiceDropReason reason) => _dropped.TryGetValue(reason, out var count) ? count : 0;

        public void Record(VoiceRelayDecision decision, int bytes)
        {
            ReceivedPackets++; ReceivedBytes += bytes;
            if (decision.Relayed) { RelayedPackets++; RelayedBytes += (long)bytes * decision.Recipients.Count; }
            else _dropped[decision.Drop] = Dropped(decision.Drop) + 1;
        }

        public override string ToString() =>
            $"voice received={ReceivedPackets}/{ReceivedBytes}B relayed={RelayedPackets}/{RelayedBytes}B " +
            string.Join(" ", new[] { VoiceDropReason.RelayOff, VoiceDropReason.Unbound, VoiceDropReason.Muted, VoiceDropReason.NotAllowedGroup, VoiceDropReason.OverBudget }
                .Select(reason => $"{VoiceRelayDecision.Name(reason)}={Dropped(reason)}"));
    }

    public static class VoiceRelay
    {
        /// <summary>Decide one voice packet. Checks, in order: relay on; the talker is bound and
        /// in the table; not muted; the group is served by this tier and in the talker's transmit
        /// set; within the talker's byte budget. A relayed packet goes to every other bound client
        /// on the talker's side whose listen set has the group; never to the talker, never to the
        /// other side, never to a party group. A world without a table relays nothing.</summary>
        /// <param name="talkerBound">The talker is a bound, admitted client of this world.</param>
        /// <param name="talkerMuted">Placeholder for the account voice mute or ban (stage 4).</param>
        /// <param name="recipientBound">Whether a DME client index is a bound, admitted client
        /// whose reservation matches its table entry.</param>
        public static VoiceRelayDecision Decide(VoiceRelayOptions options, VoiceRelayTable table, int talker, int group, int bytes,
            bool talkerBound, bool talkerMuted, Func<VoiceGroupsMember, bool> recipientBound, VoiceTalkerBudget budget, long nowMs)
        {
            if (options == null || !options.Enabled) return VoiceRelayDecision.Dropped(VoiceDropReason.RelayOff);
            var source = table?.Find(talker);
            if (!talkerBound || source == null) return VoiceRelayDecision.Dropped(VoiceDropReason.Unbound);
            if (talkerMuted) return VoiceRelayDecision.Dropped(VoiceDropReason.Muted);
            if (VoiceGroupNumbers.IsParty(group) || !options.Serves(group) || !source.Transmit.Contains(group))
                return VoiceRelayDecision.Dropped(VoiceDropReason.NotAllowedGroup);
            if (budget == null || !budget.TryConsume(talker, bytes, nowMs)) return VoiceRelayDecision.Dropped(VoiceDropReason.OverBudget);
            var recipients = table.Members
                .Where(member => member.PlayerId != talker && member.Side == source.Side && member.Listen.Contains(group) &&
                    recipientBound != null && recipientBound(member))
                .Select(member => (int)member.PlayerId)
                .OrderBy(id => id)
                .ToArray();
            return VoiceRelayDecision.Relay(recipients);
        }
    }
}
