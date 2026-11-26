using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using Server.Dme;

namespace Server.Medius
{
    /// <summary>Strict reader for the world stream's voice_groups frame (voice_groups_v1):
    /// {"revision":n,"members":[{"reservation_id","player_id","side","listen","transmit"}]}.
    /// Each frame is the full table. A malformed frame fails the whole world frame.</summary>
    public static class GameplayVoiceGroupsCodec
    {
        public const int MaximumMembers = 256;

        static bool ExactFields(JsonElement value, params string[] expected)
        {
            if (value.ValueKind != JsonValueKind.Object) return false;
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in value.EnumerateObject())
                if (!names.Add(property.Name) || !expected.Contains(property.Name)) return false;
            return names.Count == expected.Length;
        }

        static bool Identifier(string value) => value != null && value.Length >= 16 && value.Length <= 64 &&
            value.All(c => c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '-' || c == '_');

        static bool TryGroups(JsonElement value, int limit, out IReadOnlyList<int> groups)
        {
            groups = null;
            if (value.ValueKind != JsonValueKind.Array || value.GetArrayLength() > limit) return false;
            var parsed = new List<int>();
            foreach (var item in value.EnumerateArray())
            {
                if (item.ValueKind != JsonValueKind.Number || !item.TryGetInt32(out var group) ||
                    !VoiceGroupNumbers.IsRelayGroup(group) || parsed.Contains(group)) return false;
                parsed.Add(group);
            }
            groups = parsed.AsReadOnly();
            return true;
        }

        public static bool TryParse(JsonElement value, out VoiceGroupsFrame frame)
        {
            frame = null;
            if (!ExactFields(value, "revision", "members") ||
                !value.GetProperty("revision").TryGetUInt64(out var revision) || revision == 0) return false;
            var members = value.GetProperty("members");
            if (members.ValueKind != JsonValueKind.Array || members.GetArrayLength() > MaximumMembers) return false;
            var parsed = new List<VoiceGroupsMember>();
            var players = new HashSet<ushort>();
            var reservations = new HashSet<string>(StringComparer.Ordinal);
            foreach (var member in members.EnumerateArray())
            {
                if (!ExactFields(member, "reservation_id", "player_id", "side", "listen", "transmit")) return false;
                var reservation = member.GetProperty("reservation_id");
                if (reservation.ValueKind != JsonValueKind.String || !Identifier(reservation.GetString()) ||
                    !member.GetProperty("player_id").TryGetUInt16(out var player) ||
                    !member.GetProperty("side").TryGetByte(out var side) || side > 1 ||
                    !TryGroups(member.GetProperty("listen"), VoiceGroupNumbers.MaxListenGroups, out var listen) ||
                    !TryGroups(member.GetProperty("transmit"), VoiceGroupNumbers.MaxTransmitGroups, out var transmit) ||
                    !players.Add(player) || !reservations.Add(reservation.GetString())) return false;
                parsed.Add(new VoiceGroupsMember(reservation.GetString(), player, side, listen, transmit));
            }
            frame = new VoiceGroupsFrame(revision, parsed.AsReadOnly());
            return true;
        }
    }
}
