using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace Server.Medius
{
    public sealed record GameplaySelectedMatch(string FormationId,string ProfileId,int GameType,
        byte AttackingFaction,byte DefendingFaction, ulong? AdmissionEpoch = null);
    internal static class GameplayMatchProfileCodec
    {
        static bool Id(string text,int minimum=1)=>text!=null&&text.Length>=minimum&&text.Length<=64&&
            text.All(c=>char.IsAsciiLetterOrDigit(c)||c=='-'||c=='_');
        static bool Fields(JsonElement row,params string[] fields)=>row.ValueKind==JsonValueKind.Object&&
            row.EnumerateObject().Count()==fields.Length&&row.EnumerateObject().Select(p=>p.Name).Distinct().Count()==fields.Length&&
            row.EnumerateObject().All(p=>fields.Contains(p.Name));
        static bool Pair(JsonElement row,out int mode,out byte attacker,out byte defender)
        {
            mode=-1;attacker=defender=255;
            return row.GetProperty("game_type").TryGetInt32(out mode)&&(mode is >= 0 and <= 2)&&
                row.GetProperty("attacking_faction").TryGetByte(out attacker)&&attacker<3&&
                row.GetProperty("defending_faction").TryGetByte(out defender)&&defender<3&&attacker!=defender;
        }
        /// <param name="worldCapacity">The worker advertised <c>world_capacity_v1</c>. Only then may a row
        /// exceed 64 Characters (32 per side) or carry human caps, and every such large row must carry them.</param>
        public static bool Profiles(JsonElement rows,bool success,bool worldCapacity,out IReadOnlyList<MagMatchProfile> result)
        {
            result=null;
            if(rows.ValueKind!=JsonValueKind.Array||rows.GetArrayLength()>64||(!success&&rows.GetArrayLength()!=0))return false;
            var list=new List<MagMatchProfile>();var ids=new HashSet<string>(StringComparer.Ordinal);
            foreach(var row in rows.EnumerateArray())
            {
                bool hasSquad = row.TryGetProperty("squad_capacity", out var squadCapacity);
                bool hasCounts = row.TryGetProperty("squad_counts", out var squadCounts);
                bool hasBots = row.TryGetProperty("bot_fill_total", out var botFill);
                bool hasHumans = row.TryGetProperty("human_capacity", out var humanCapacity);
                bool hasTeamHumans = row.TryGetProperty("human_team_capacity", out var humanTeamCapacity);
                var required = new[] { "profile_id", "game_type", "attacking_faction", "defending_faction", "maximum_players", "team_capacity" };
                var fields = (hasSquad ? required.Concat(new[]{"squad_capacity","squad_counts"}) : required)
                    .Concat(hasBots ? new[]{"bot_fill_total"} : Array.Empty<string>())
                    .Concat(hasHumans ? new[]{"human_capacity","human_team_capacity"} : Array.Empty<string>()).ToArray();
                if(hasCounts!=hasSquad||hasHumans!=hasTeamHumans||(hasHumans&&!worldCapacity)||!Fields(row,fields)||
                    !Pair(row,out var mode,out var attacker,out var defender))return false;
                var id=row.GetProperty("profile_id").GetString();
                if(!Id(id)||!ids.Add(id)||!row.GetProperty("maximum_players").TryGetInt32(out var maximum)||
                    !row.GetProperty("team_capacity").TryGetInt32(out var team)||team<1||maximum<2||maximum>team*2)return false;
                // Without world_capacity_v1 a row keeps the original 64/32 bounds. A larger world
                // must say how many of its seats humans may take.
                bool large = MagWorldCapacity.IsLarge(maximum, team);
                if(large&&(!worldCapacity||!hasHumans||maximum>MagWorldCapacity.MaximumWorldCharacters||
                    team>MagWorldCapacity.MaximumTeamCharacters))return false;
                int? humans = null, teamHumans = null;
                if (hasHumans)
                {
                    if (!humanCapacity.TryGetInt32(out int total) || !humanTeamCapacity.TryGetInt32(out int side) ||
                        !MagWorldCapacity.ValidHumanCaps(maximum, team, total, side)) return false;
                    humans = total; teamHumans = side;
                }
                int? squad = null;
                int[] counts = null;
                if (hasSquad)
                {
                    if (!squadCapacity.TryGetInt32(out int value) || value < 1 || value > team) return false;
                    squad = value;
                    if (squadCounts.ValueKind != JsonValueKind.Array || squadCounts.GetArrayLength() != 2) return false;
                    counts = squadCounts.EnumerateArray().Select(count => count.TryGetInt32(out int n) ? n : 0).ToArray();
                    if (counts.Any(count => count < 1 || count > 32)) return false;
                }
                int? bots = null;
                if (hasBots)
                {
                    if (!botFill.TryGetInt32(out int total) || total < 2 || total > maximum) return false;
                    bots = total;
                }
                list.Add(new MagMatchProfile(id,mode,attacker,defender,maximum,team,squad,counts == null ? null : Array.AsReadOnly(counts),bots,humans,teamHumans));
            }
            result=list.AsReadOnly();return true;
        }
        public static bool Selection(JsonElement row,bool success,out GameplaySelectedMatch result)
        {
            result=null;if(row.ValueKind==JsonValueKind.Null)return true;
            var required = new[] { "formation_id", "profile_id", "game_type", "attacking_faction", "defending_faction" };
            bool hasEpoch = row.TryGetProperty("admission_epoch", out var admissionEpoch);
            if(!success||!Fields(row,hasEpoch ? required.Append("admission_epoch").ToArray() : required)||
                !Pair(row,out var mode,out var attacker,out var defender))return false;
            var formation=row.GetProperty("formation_id").GetString();var profile=row.GetProperty("profile_id").GetString();
            if(!Id(formation,16)||!Id(profile))return false;
            ulong? epoch = null;
            if (hasEpoch)
            {
                if (!admissionEpoch.TryGetUInt64(out ulong value) || value == 0) return false;
                epoch = value;
            }
            result=new GameplaySelectedMatch(formation,profile,mode,attacker,defender,epoch);return true;
        }
    }
}
