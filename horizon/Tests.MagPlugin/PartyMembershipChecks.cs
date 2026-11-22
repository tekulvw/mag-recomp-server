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
    }
}
