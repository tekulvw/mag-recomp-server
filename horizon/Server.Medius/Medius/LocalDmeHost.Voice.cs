using System;
using System.Collections.Generic;
using RT.Models;
using Server.Dme;
using DmeClient = Server.Dme.Models.ClientObject;

namespace Server.Medius
{
    // In-match voice relay state for this world (docs/superpowers/specs/2026-11-25-voice-relay-design.md).
    // Off unless MAG_VOICE_RELAY=1 and the worker advertises voice_groups_v1. The table follows the
    // world stream. While the switch is on, the DME hands each group-addressed client send to Voice:
    // it is parsed (VoiceGroupSend), decided (DecideVoiceRelay) and delivered in the listener form
    // (VoiceListenerMessage) to each recipient, over its UDP channel when it has one and TCP
    // otherwise. A talker whose account holds an active voice mute or ban (VoiceSanctionCache) is
    // dropped as muted. With the switch off the DME never calls Voice and group sends keep today's path.
    public sealed partial class LocalDmeHost
    {
        readonly VoiceRelayOptions _voiceRelayOptions = VoiceRelayOptions.Current;
        readonly VoiceTalkerBudget _voiceBudget = new VoiceTalkerBudget();
        readonly VoiceRelayCounters _voiceCounters = new VoiceRelayCounters();
        /// <summary>The counters line is logged at most this often, and only after new packets.</summary>
        const long VoiceCountersIntervalMs = 60_000;
        long _voiceCountersLoggedMs = long.MinValue;
        long _voiceCountersLoggedPackets;
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
                // An account voice mute or ban (VoiceSanctionCache) drops both copies of every frame. The
                // lookup reads the in-memory snapshot; the store is never asked per packet.
                bool silenced = bound && VoiceSanctionCache.Current?.Silencing(reservation.Account) != null;
                var decision = VoiceRelay.Decide(_voiceRelayOptions, _voiceTable, talker?.DmeId ?? -1, group, bytes, bound,
                    silenced,
                    member => _reservations.TryGetValue(member.ReservationId, out var r) && r.Client != null &&
                        r.Client.DmeId == member.PlayerId && r.Client.DmeWorld == talker.DmeWorld && BoundForVoice(r, r.Client),
                    _voiceBudget, Environment.TickCount64);
                _voiceCounters.Record(decision, Math.Max(0, bytes));
                return decision;
            }
        }

        /// <summary>Relay one group-addressed send from <paramref name="talker"/>: a voice packet is
        /// decided and its listener form queued for each recipient; anything else is dropped and
        /// counted as malformed. Audio never reaches the gameplay bridge.</summary>
        public void Voice(DmeClient talker, string transport, byte rtType, byte[] body)
        {
            foreach (var (client, message) in PlanVoiceRelay(talker, body))
            {
                if (client.Udp != null && client.RemoteUdpEndpoint != null) client.EnqueueUdp(message);
                else client.EnqueueTcp(message);
            }
        }

        /// <summary>The deliveries for one group send, decided and counted under _gate.</summary>
        IReadOnlyList<(DmeClient client, BaseScertMessage message)> PlanVoiceRelay(DmeClient talker, byte[] body)
        {
            lock (_gate)
            {
                var deliveries = new List<(DmeClient, BaseScertMessage)>();
                if (talker == null || body == null || !VoiceGroupSend.TryParse(body, out var send))
                {
                    _voiceCounters.Record(VoiceRelayDecision.Dropped(VoiceDropReason.Malformed), 0);
                    LogVoiceCounters();
                    return deliveries;
                }
                var decision = DecideVoiceRelay(talker, send.Group, send.DataLength);
                if (decision.Relayed && decision.Recipients.Count > 0)
                {
                    var message = VoiceListenerMessage.Build(talker.DmeId, body, send);
                    foreach (int id in decision.Recipients)
                    {
                        var member = _voiceTable.Find(id);
                        if (member != null && _reservations.TryGetValue(member.ReservationId, out var r) && r.Client != null && r.Client != talker)
                            deliveries.Add((r.Client, message));
                    }
                }
                LogVoiceCounters();
                return deliveries;
            }
        }

        /// <summary>The periodic voice counters line. Called with _gate held.</summary>
        void LogVoiceCounters()
        {
            long now = Environment.TickCount64;
            if (_voiceCounters.ReceivedPackets == _voiceCountersLoggedPackets ||
                (_voiceCountersLoggedMs != long.MinValue && now - _voiceCountersLoggedMs < VoiceCountersIntervalMs)) return;
            _voiceCountersLoggedMs = now;
            _voiceCountersLoggedPackets = _voiceCounters.ReceivedPackets;
            Logger.Info($"Voice relay: {StreamScope} tier={VoiceRelayOptions.Name(_voiceRelayOptions.Tier)} {_voiceCounters}");
        }

        /// <summary>A bound, admitted client of this world's live match. Called with _gate held.</summary>
        bool BoundForVoice(Reservation r, DmeClient client) =>
            r != null && client != null && r.Client == client && ReadyForGameplay && r.Claimed && !r.Invalidated &&
            IsLiveCohort(r) && client.IsAuthenticated && !client.IsDestroyed && !client.Disconnected &&
            client.DmeWorld != null && client.DmeWorld.Clients.TryGetValue(client.DmeId, out var member) && member == client;
    }
}
