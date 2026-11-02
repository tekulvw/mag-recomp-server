using System;
using System.Collections.Generic;
using System.Linq;
using Server.Medius;

// Invented queue identities and profiles only; no accounts or live service.
internal static class FactionMatchmakingChecks
{
    static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }
    static void Throws<T>(Action action) where T : Exception
    {
        try { action(); }
        catch (T) { return; }
        throw new Exception("Expected " + typeof(T).Name);
    }
    static MagMatchProfile Profile(byte attacker, byte defender, int mode = 0,
        int maximum = 64, int team = 32) =>
        new MagMatchProfile($"invented-{mode}-{attacker}-{defender}", mode, attacker, defender, maximum, team);
    static MagQueuedHuman Human(string id, byte faction, long order, int mode = 0) =>
        new MagQueuedHuman(id, mode, faction, order);
    static MagMatchProfile[] Profiles() => (from mode in new[] { 0, 1 }
        from attacker in new byte[] { 0, 1, 2 }
        from defender in new byte[] { 0, 1, 2 }
        where attacker != defender select Profile(attacker, defender, mode)).ToArray();

    public static void Run()
    {
        // An infeasible oldest ticket must not block the oldest feasible faction.
        var waiting = new[] { Human("blocked-mode", 0, 0, 1), Human("eligible-oldest", 1, 10),
            Human("third-faction", 2, 11), Human("opponent", 0, 12) };
        var snapshot = waiting.ToArray();
        var draws = new Queue<int>(new[] { 0, 1 });
        var bounds = new List<int>();
        var selected = MagFactionMatchmaking.Select(waiting, Profiles(), 1, 64, new[] { 0, 0 },
            count => { bounds.Add(count); return draws.Dequeue(); });
        Check(selected != null && selected.Profile.GameType == 0 &&
            selected.Profile.AttackingFaction == 0 && selected.Profile.DefendingFaction == 1,
            "Oldest feasible faction, opponent and role draws must determine match");
        Check(bounds.SequenceEqual(new[] { 2, 2 }), "Draw distinct opponents then feasible roles");
        Check(selected.Players.Select(p => p.TicketId).OrderBy(x => x).SequenceEqual(new[] { "eligible-oldest", "opponent" }),
            "Other faction and mode must stay outside cohort");
        Check(waiting.SequenceEqual(snapshot), "Planner changed caller queue");

        // Every saved faction may be oldest, attack, or defend in either mode.
        foreach (int mode in new[] { 0, 1 })
        foreach (byte oldest in new byte[] { 0, 1, 2 })
        foreach (int opponentDraw in new[] { 0, 1 })
        foreach (int roleDraw in new[] { 0, 1 })
        {
            var queue = new[] { Human("oldest", oldest, 0, mode), Human("zero", 0, 1, mode),
                Human("one", 1, 2, mode), Human("two", 2, 3, mode) };
            var eligibleOpponents = new byte[] { 0, 1, 2 }.Where(f => f != oldest).ToArray();
            int call = 0;
            var result = MagFactionMatchmaking.Select(queue, Profiles(), 2, 64, new[] { 1, 1 },
                count => call++ == 0 ? opponentDraw : roleDraw);
            byte opponent = eligibleOpponents[opponentDraw];
            Check(result != null && result.Profile.GameType == mode &&
                result.Profile.AttackingFaction == (roleDraw == 0 ? oldest : opponent) &&
                result.Profile.DefendingFaction == (roleDraw == 0 ? opponent : oldest), "Faction or role permutation lost");
            Check(result.Players.Any(p => p.TicketId == "oldest") && result.Players.All(p =>
                p.GameType == mode && (p.Faction == oldest || p.Faction == opponent)), "Selected cohort changed saved identity");
        }

        // Team minima reserve scarce-side tickets before filling oldest leftovers.
        var crowded = new[] { Human("a0", 0, 0), Human("a1", 0, 1), Human("a2", 0, 2),
            Human("a3", 0, 3), Human("b0", 1, 10), Human("b1", 1, 11), Human("b2", 1, 12), Human("other", 2, -1) };
        var cohort = MagFactionMatchmaking.ForProfile(crowded, Profile(0, 1, maximum: 6, team: 3),
            3, 4, new[] { 1, 2 });
        Check(cohort != null && cohort.Players.Select(p => p.TicketId).SequenceEqual(new[] { "a0", "a1", "b0", "b1" }),
            "Fill must honor role minima before oldest global fill and preserve output order");
        var capped = MagFactionMatchmaking.ForProfile(crowded, Profile(0, 1, maximum: 6, team: 3),
            2, 64, new[] { 1, 1 });
        Check(capped.Players.Select(p => p.TicketId).SequenceEqual(new[] { "a0", "a1", "a2", "b0", "b1", "b2" }),
            "Profile global and team caps must bound cohort");
        Check(MagFactionMatchmaking.ForProfile(crowded, Profile(0, 1, maximum: 4, team: 2),
            5, 8, new[] { 1, 1 }) == null, "Insufficient profile capacity must not form match");

        // Asymmetric minima can permit only one orientation; role draw bound is one.
        var asymmetric = new[] { Human("old", 0, 0), Human("peer0", 1, 1), Human("peer1", 1, 2) };
        bounds.Clear();
        var oriented = MagFactionMatchmaking.Select(asymmetric, new[] { Profile(0, 1), Profile(1, 0) },
            3, 8, new[] { 2, 1 }, n => { bounds.Add(n); return 0; });
        Check(oriented != null && oriented.Profile.AttackingFaction == 1 &&
            bounds.SequenceEqual(new[] { 1, 1 }), "Infeasible orientation entered random draw");
        var single = new[] { Human("late", 1, 0) };
        Check(MagFactionMatchmaking.Select(single, Profiles(), 1, 64, new[] { 0, 0 }, n => 0) == null,
            "One human must not form a new match");
        Check(MagFactionMatchmaking.Select(new[] { Human("same-a", 0, 0), Human("same-b", 0, 1) },
            Profiles(), 1, 64, new[] { 0, 0 }, n => 0) == null, "Same-faction queue formed an opposing match");
        var late = MagFactionMatchmaking.ForProfile(single, Profile(0, 1), 1, 64, new[] { 0, 0 });
        Check(late != null && late.Players.Single().TicketId == "late", "Fixed profile must allow one defender late join");
        Check(MagFactionMatchmaking.ForProfile(new[] { Human("late-attacker", 0, 0) }, Profile(0, 1),
            1, 64, new[] { 0, 0 }) != null, "Fixed profile must allow one attacker late join");
        Check(MagFactionMatchmaking.ForProfile(new[] { Human("wrong-side", 2, 0) }, Profile(0, 1),
            1, 64, new[] { 0, 0 }) == null, "Third faction admitted to fixed match");

        // Validate all inputs, even entries that would otherwise remain unselected.
        var pair = new[] { Human("a", 0, 0), Human("b", 1, 1) };
        Throws<ArgumentException>(() => MagFactionMatchmaking.Select(pair.Concat(new[] { Human("a", 2, 2) }).ToArray(),
            Profiles(), 2, 64, new[] { 1, 1 }, n => 0));
        foreach (var bad in new[] { Human("", 0, 0), Human("bad-faction", 3, 0), Human("bad-mode", 0, 0, 2) })
            Throws<ArgumentException>(() => MagFactionMatchmaking.Select(new[] { bad }, Profiles(), 2, 64, new[] { 1, 1 }, n => 0));
        foreach (var bad in new[] { Profile(0, 0), Profile(3, 1), Profile(0, 1, mode: 3),
            Profile(0, 1, maximum: 65), Profile(0, 1, team: 33), Profile(0, 1, team: 0),
            new MagMatchProfile(" ", 0, 0, 1, 64, 32) })
            Throws<ArgumentException>(() => MagFactionMatchmaking.Select(pair, new[] { bad }, 2, 64, new[] { 1, 1 }, n => 0));
        Throws<ArgumentException>(() => MagFactionMatchmaking.Select(pair, new[] { Profile(0, 1), Profile(0, 1) },
            2, 64, new[] { 1, 1 }, n => 0));
        foreach (var bad in new[] { new[] { 1 }, new[] { -1, 1 }, new[] { 33, 32 }, new[] { int.MaxValue, 1 } })
            Throws<ArgumentException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 2, 64, bad, n => 0));
        Throws<ArgumentException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 0, 64, new[] { 1, 1 }, n => 0));
        Throws<ArgumentException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 3, 2, new[] { 1, 1 }, n => 0));
        Throws<ArgumentException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 2, 65, new[] { 1, 1 }, n => 0));
        Throws<ArgumentNullException>(() => MagFactionMatchmaking.Select(null, Profiles(), 2, 64, new[] { 1, 1 }, n => 0));
        Throws<ArgumentNullException>(() => MagFactionMatchmaking.Select(pair, null, 2, 64, new[] { 1, 1 }, n => 0));
        Throws<ArgumentNullException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 2, 64, new[] { 1, 1 }, null));
        Throws<ArgumentOutOfRangeException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 2, 64, new[] { 1, 1 }, n => -1));
        Throws<ArgumentOutOfRangeException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 2, 64, new[] { 1, 1 }, n => n));
        int randomCall = 0;
        Throws<ArgumentOutOfRangeException>(() => MagFactionMatchmaking.Select(pair, Profiles(), 2, 64, new[] { 1, 1 },
            n => randomCall++ == 0 ? 0 : n));
    }
}
