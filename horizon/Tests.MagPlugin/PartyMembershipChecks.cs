using RT.Models.MAG;
using Server.Medius;

static class PartyMembershipChecks
{
    static void Check(bool value, string note)
    {
        if (!value) throw new Exception(note);
    }

    public static void Matchmaking()
    {
        var profile = new MagMatchProfile("party-profile", 2, 0, 1, 6, 4, 2, new[] { 2, 1 });
        var waiting = new[]
        {
            new MagQueuedHuman("oversize", 2, 0, 1, 3),
            new MagQueuedHuman("older-party", 2, 0, 2, 2),
            new MagQueuedHuman("second-party", 2, 0, 3, 2),
            new MagQueuedHuman("opposing-party", 2, 1, 4, 2)
        };
        var selected = MagFactionMatchmaking.ForProfile(waiting, profile, 6, 6, new[] { 2, 2 });
        Check(selected != null && selected.Players.Select(player => player.TicketId).SequenceEqual(
            new[] { "older-party", "second-party", "opposing-party" }), "Same-side squads or opposing party accounting failed");
        Check(MagFactionMatchmaking.ForProfile(waiting, profile with { SquadCounts = new[] { 1, 1 } }, 6, 6, new[] { 2, 2 }) == null,
            "Party selection exceeded per-side authored squads");
        Check(MagFactionMatchmaking.ForProfile(waiting, profile with { SquadCapacity = null, SquadCounts = null }, 4, 6, new[] { 2, 2 }) == null,
            "Party selection used unsupported worker metadata");
        var earlierSolo = new MagQueuedHuman("old-solo", 2, 0, 0);
        selected = MagFactionMatchmaking.ForProfile(waiting.Prepend(earlierSolo).ToArray(), profile, 4, 5, new[] { 2, 2 });
        Check(selected != null && selected.Players.Select(player => player.TicketId).SequenceEqual(
            new[] { "old-solo", "older-party", "opposing-party" }), "Party grouping lost feasible waiting-order fairness");
        LatePlacement();
    }

    static void LatePlacement()
    {
        var profile = new MagMatchProfile("late-profile", 2, 0, 1, 8, 4, 2, new[] { 2, 2 }, BotFillTotal: 8);
        MagWorldPlacementState World(string id, bool late, MagMatchProfile current = null, int[] live = null, bool canForm = false) =>
            new(id, true, false, 1, 8, new[] { 0, 0 }, new[] { profile }, current, live ?? new[] { 0, 0, 0 },
                new[] { 0, 0, 0 }, (live ?? new[] { 0, 0, 0 }).Sum(), 0, canForm, late);
        var waiting = new[] { new MagQueuedHuman("solo", 2, 1, 0), new MagQueuedHuman("party", 2, 0, 1, 2) };
        var worlds = new[] { World("idle", false, canForm: true), World("quiet", true, profile, new[] { 1, 0, 0 }),
            World("busy", true, profile, new[] { 1, 2, 0 }) };
        var plan = MagWorldPlacement.PlanNext(waiting, worlds, _ => 0);
        Check(plan != null && plan.WorkerId == "busy" && !plan.NewFormation &&
            plan.Players.Single().TicketId == "party", "A party did not join the fullest running match first");
        worlds = new[] { World("idle", false, canForm: true), World("closed", false, profile, new[] { 1, 2, 0 }) };
        plan = MagWorldPlacement.PlanNext(waiting, worlds, _ => 0);
        Check(plan != null && plan.WorkerId == "idle" && plan.NewFormation &&
            plan.Players.Any(human => human.TicketId == "party"), "A party entered a world without late entry");
        // Its side would exceed the team, or the party one squad: form instead.
        worlds = new[] { World("idle", false, canForm: true), World("side-full", true, profile, new[] { 3, 0, 0 }) };
        plan = MagWorldPlacement.PlanNext(waiting, worlds, _ => 0);
        Check(plan != null && plan.WorkerId == "idle" && plan.NewFormation, "A party overfilled a running side");
        var large = new[] { new MagQueuedHuman("large", 2, 0, 1, 3) };
        Check(MagWorldPlacement.PlanNext(large, new[] { World("quiet", true, profile, new[] { 1, 0, 0 }) }, _ => 0) == null,
            "A party larger than one squad entered late");
        var opposing = new[] { new MagQueuedHuman("opposing", 2, 2, 1, 2) };
        Check(MagWorldPlacement.PlanNext(opposing, new[] { World("quiet", true, profile, new[] { 1, 0, 0 }) }, _ => 0) == null,
            "A party joined a match without its faction");
    }

    public static void Run()
    {
        var now = DateTime.UtcNow;
        var directory = new MagPartyDirectory(TimeSpan.FromSeconds(10));
        var first = new List<MagRetailPartyMessage>();
        var second = new List<MagRetailPartyMessage>();
        var third = new List<MagRetailPartyMessage>();
        var leader = new MagPartyDirectory.Member("leader", 17, 1, () => true, first.Add);
        var friend = new MagPartyDirectory.Member("friend", 19, 1, () => true, second.Add);
        var opponent = new MagPartyDirectory.Member("opponent", 23, 0, () => true, third.Add);
        directory.Register(leader);
        directory.Register(friend);
        directory.Register(opponent);
        bool Invite(uint from, uint to, DateTime time) =>
            directory.Apply(leader, new MagRetailPartyInvite { Inviter = from, Invitee = to }, time);
        Check(!Invite(19, 19, now) && !Invite(17, 23, now), "Forged or cross-faction invite accepted");
        Check(Invite(17, 19, now) && second.Single() is MagRetailPartyInvite, "Valid private invite not delivered");
        var invite = (MagRetailPartyInvite)second.Single();
        var response = new MagRetailPartyInviteResponse { PartyId = invite.PartyId, Inviter = 17, Responder = 19 };
        Check(!directory.Apply(opponent, response, now), "Forged response consumed invite");
        Check(directory.Apply(friend, response, now) && !directory.Apply(friend, response, now), "Invite was not consumed once");
        Check(second.Last() is MagRetailPartyRoster roster && roster.Members.SequenceEqual(new uint[] {17, 19}),
            "Leader-first roster missing");
        Check(directory.GetLeaderMembers(friend, invite.PartyId) == null &&
            directory.GetLeaderMembers(leader, invite.PartyId).SequenceEqual(new[] { leader, friend }),
            "Non-leader or stale membership authorized");
        directory.Remove(leader);
        Check(second.Last() is MagRetailPartyLeft && directory.GetLeaderMembers(leader, invite.PartyId) == null,
            "Leader departure did not dissolve membership");

        directory.Register(leader);
        Check(Invite(17, 19, now), "Fresh invite refused");
        invite = (MagRetailPartyInvite)second.Last();
        response.PartyId = invite.PartyId;
        Check(!directory.Apply(friend, response, now.AddSeconds(10)), "Expired invitation accepted");
        directory.Remove(friend);
        var replacement = friend with { Send = _ => {} };
        directory.Register(replacement);
        Check(!directory.Apply(friend, response, now.AddSeconds(10)), "Retired channel acquired replacement authority");
        Capacity(now);
    }

    static void Capacity(DateTime now)
    {
        var directory = new MagPartyDirectory(TimeSpan.FromSeconds(10));
        var inbox = new Dictionary<uint, List<MagRetailPartyMessage>>();
        MagPartyDirectory.Member Join(uint id)
        {
            inbox[id] = new List<MagRetailPartyMessage>();
            var member = new MagPartyDirectory.Member($"member-{id}", id, 1, () => true, inbox[id].Add);
            Check(directory.Register(member), "Capacity fixture registration failed");
            return member;
        }
        var leader = Join(100);
        var friends = Enumerable.Range(1, MagPartyDirectory.MaximumMembers).Select(i => Join(100 + (uint)i)).ToArray();
        bool Invite(MagPartyDirectory.Member target) => directory.Apply(leader,
            new MagRetailPartyInvite { Inviter = leader.AccountId, Invitee = target.AccountId,
                PartyId = inbox[leader.AccountId].OfType<MagRetailPartyJoined>().FirstOrDefault()?.PartyId ?? 0 }, now);
        // Pending invitations hold seats: a full party plus outstanding invites is refused.
        for (int i = 0; i < MagPartyDirectory.MaximumMembers - 1; i++)
            Check(Invite(friends[i]), "An invite within the squad-sized cap was refused");
        Check(!Invite(friends[^1]), "An invite beyond one squad was accepted");
        uint partyId = inbox[leader.AccountId].OfType<MagRetailPartyJoined>().Single().PartyId;
        for (int i = 0; i < MagPartyDirectory.MaximumMembers - 1; i++)
            Check(directory.Apply(friends[i], new MagRetailPartyInviteResponse
                { PartyId = partyId, Inviter = leader.AccountId, Responder = friends[i].AccountId }, now), "An accepted invite failed");
        var roster = inbox[leader.AccountId].OfType<MagRetailPartyRoster>().Last();
        Check(roster.Members.Length == MagPartyDirectory.MaximumMembers && MagPartyDirectory.MaximumMembers == 8,
            "A full party is not exactly one squad");
        Check(!Invite(friends[^1]) && !inbox[friends[^1].AccountId].Any(), "A full party delivered another invite");
    }
}
