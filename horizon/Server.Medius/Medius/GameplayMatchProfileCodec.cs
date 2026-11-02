using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace Server.Medius
{
    public sealed record GameplaySelectedMatch(string FormationId,string ProfileId,int GameType,
        byte AttackingFaction,byte DefendingFaction);
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
            return row.GetProperty("game_type").TryGetInt32(out mode)&&(mode==0||mode==1)&&
                row.GetProperty("attacking_faction").TryGetByte(out attacker)&&attacker<3&&
                row.GetProperty("defending_faction").TryGetByte(out defender)&&defender<3&&attacker!=defender;
        }
        public static bool Profiles(JsonElement rows,bool success,out IReadOnlyList<MagMatchProfile> result)
        {
            result=null;
            if(rows.ValueKind!=JsonValueKind.Array||rows.GetArrayLength()>64||(!success&&rows.GetArrayLength()!=0))return false;
            var list=new List<MagMatchProfile>();var ids=new HashSet<string>(StringComparer.Ordinal);
            foreach(var row in rows.EnumerateArray())
            {
                if(!Fields(row,"profile_id","game_type","attacking_faction","defending_faction","maximum_players","team_capacity")||
                    !Pair(row,out var mode,out var attacker,out var defender))return false;
                var id=row.GetProperty("profile_id").GetString();
                if(!Id(id)||!ids.Add(id)||!row.GetProperty("maximum_players").TryGetInt32(out var maximum)||
                    !row.GetProperty("team_capacity").TryGetInt32(out var team)||team<1||team>32||maximum<2||maximum>64||maximum>team*2)return false;
                list.Add(new MagMatchProfile(id,mode,attacker,defender,maximum,team));
            }
            result=list.AsReadOnly();return true;
        }
        public static bool Selection(JsonElement row,bool success,out GameplaySelectedMatch result)
        {
            result=null;if(row.ValueKind==JsonValueKind.Null)return true;
            if(!success||!Fields(row,"formation_id","profile_id","game_type","attacking_faction","defending_faction")||
                !Pair(row,out var mode,out var attacker,out var defender))return false;
            var formation=row.GetProperty("formation_id").GetString();var profile=row.GetProperty("profile_id").GetString();
            if(!Id(formation,16)||!Id(profile))return false;
            result=new GameplaySelectedMatch(formation,profile,mode,attacker,defender);return true;
        }
    }
}
