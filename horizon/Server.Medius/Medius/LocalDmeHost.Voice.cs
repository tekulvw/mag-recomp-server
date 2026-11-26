using System;
using Server.Dme;
using DmeClient = Server.Dme.Models.ClientObject;

namespace Server.Medius
{
    // In-match voice relay state for this world (docs/superpowers/specs/2026-11-25-voice-relay-design.md).
    // Off unless MAG_VOICE_RELAY=1 and the worker advertises voice_groups_v1. The table follows the
    // world stream; DecideVoiceRelay is the seam the DME packet path will call once the lab capture
    // names the transport message that carries voice packets. Until then nothing calls it, and
    // voice traffic (whatever its carrier) keeps today's path.
    public sealed partial class LocalDmeHost
    {
        readonly VoiceRelayOptions _voiceRelayOptions = VoiceRelayOptions.Current;
        readonly VoiceTalkerBudget _voiceBudget = new VoiceTalkerBudget();
        readonly VoiceRelayCounters _voiceCounters = new VoiceRelayCounters();
        bool _supportsVoiceGroups;
        VoiceRelayTable _voiceTable;

        /// <summary>Ask the worker for voice_groups. Called with _gate held.</summary>
        bool WantsVoiceGroups => _voiceRelayOptions.Enabled && _supportsVoiceGroups;

        /// <summary>The latest relay table, or null before the first voice_groups frame.</summary>
        public VoiceRelayTable VoiceTable { get { lock (_gate) return _voiceTable; } }

        /// <summary>Per-world voice counters, for the periodic world log line once packets flow.</summary>
        public string VoiceCounters { get { lock (_gate) return _voiceCounters.ToString(); } }

        /// <summary>Keep the newest table. Called with _gate held, for each routed world frame in
        /// stream order; each frame carries the full table.</summary>
        void ApplyVoiceGroups(GameplayBridgeResponse reply)
        {
            if (reply?.VoiceGroups == null || !WantsVoiceGroups) return;
            _voiceTable = new VoiceRelayTable(reply.VoiceGroups);
        }

        /// <summary>Seam for the DME voice packet hook: decide one voice packet from
        /// <paramref name="talker"/> addressed to relay <paramref name="group"/> carrying
        /// <paramref name="bytes"/> voice data bytes. Returns the recipients' DME client indices, or the
        /// drop reason; counts it either way. Delivery (the server-to-client form, stamped with the
        /// talker's index, over each recipient's UDP channel or TCP) stays with the caller.</summary>
        public VoiceRelayDecision DecideVoiceRelay(DmeClient talker, int group, int bytes)
        {
            lock (_gate)
            {
                var reservation = talker == null ? null : Find(talker);
                var entry = talker == null ? null : _voiceTable?.Find(talker.DmeId);
                bool bound = BoundForVoice(reservation, talker) && entry != null &&
                    string.Equals(entry.ReservationId, reservation.Id, StringComparison.Ordinal);
                var decision = VoiceRelay.Decide(_voiceRelayOptions, _voiceTable, talker?.DmeId ?? -1, group, bytes, bound,
                    talkerMuted: false, // Account voice mutes and bans arrive with the moderation commands.
                    member => _reservations.TryGetValue(member.ReservationId, out var r) && r.Client != null &&
                        r.Client.DmeId == member.PlayerId && r.Client.DmeWorld == talker.DmeWorld && BoundForVoice(r, r.Client),
                    _voiceBudget, Environment.TickCount64);
                _voiceCounters.Record(decision, Math.Max(0, bytes));
                return decision;
            }
        }

        /// <summary>A bound, admitted client of this world's live match. Called with _gate held.</summary>
        bool BoundForVoice(Reservation r, DmeClient client) =>
            r != null && client != null && r.Client == client && ReadyForGameplay && r.Claimed && !r.Invalidated &&
            IsLiveCohort(r) && client.IsAuthenticated && !client.IsDestroyed && !client.Disconnected &&
            client.DmeWorld != null && client.DmeWorld.Clients.TryGetValue(client.DmeId, out var member) && member == client;
    }
}
