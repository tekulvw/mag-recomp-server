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
        InviteFlow(now);
    }

    // Retail invite UI flow, driven by client-ordered wire bytes.
    static void InviteFlow(DateTime now)
    {
        var directory = new MagPartyDirectory(TimeSpan.FromSeconds(10));
        var wire = new List<(uint To, MagRetailPartyMessage Message)>();
        MagPartyDirectory.Member Online(uint id, byte faction = 1)
        {
            var member = new MagPartyDirectory.Member($"flow-{id}", id, faction, () => true, m => wire.Add((id, m)));
            Check(directory.Register(member), "Flow fixture registration failed");
            return member;
        }
        MagRetailPartyMessage Client(string hex)
        {
            Check(MagMessageCodec.TryParseClientForApplication(Convert.FromHexString(hex), 20231, out var parsed) &&
                parsed is MagRetailPartyMessage, $"Client fixture {hex} refused");
            return (MagRetailPartyMessage)parsed;
        }
        var leader = Online(0x11);
        var friend = Online(0x13);
        var shy = Online(0x15);
        var late = Online(0x17);
        var outsider = Online(0x19);
        var stranger = Online(0x1B);

        Check(directory.Apply(leader, Client("001A000000000000001300000011"), now), "Leader's first invite refused");
        Check(wire.Count == 3 && wire[0] is (0x11, MagRetailPartyJoined) &&
            wire[1] is (0x11, MagRetailPartyRoster { Members: [0x11], Status: 1, OpenInvite: false }) &&
            wire[2] is (0x13, MagRetailPartyInvite { Invitee: 0x13, Inviter: 0x11 }), "New party or forwarded invite missing");
        uint party = ((MagRetailPartyJoined)wire[0].Message).PartyId;
        Check(party != 0 && ((MagRetailPartyInvite)wire[2].Message).PartyId == party, "Forwarded invite lost the party id");

        // Accept: the joiner learns its party first, then every member gets the roster, leader first.
        wire.Clear();
        Check(directory.Apply(friend, Client($"001B00" + "00000011" + "00000013" + $"{party:X8}"), now), "Client-ordered accept refused");
        Check(wire.Count == 3 && wire[0] is (0x13, MagRetailPartyJoined { PartyId: var joined }) && joined == party &&
            wire[1] is (0x11, MagRetailPartyRoster { Members: [0x11, 0x13] }) &&
            wire[2] is (0x13, MagRetailPartyRoster { Members: [0x11, 0x13] }), "Accept did not deliver join then leader-first rosters");

        // Decline: the inviter is told who declined and why.
        wire.Clear();
        Check(directory.Apply(leader, Client($"001A{party:X8}0000001500000011"), now) && wire.Single().To == 0x15,
            "Second invite not forwarded");
        wire.Clear();
        Check(directory.Apply(shy, Client($"001B01" + "00000011" + "00000015" + $"{party:X8}"), now), "Decline refused");
        Check(wire.Single() is (0x11, MagRetailPartyDecline { Declined: 0x15, Reason: 1 } decline) &&
            MagMessageCodec.SerializeForApplication(decline, 20231).SequenceEqual(Convert.FromHexString($"002200000015{party:X8}01")),
            "Decline notice missing or misencoded");

        // Expiry: an unanswered invitation is reported to the inviter as a timeout, once.
        wire.Clear();
        Check(directory.Apply(leader, Client($"001A{party:X8}0000001700000011"), now), "Expiring invite not forwarded");
        wire.Clear();
        directory.Prune(now.AddSeconds(9));
        Check(wire.Count == 0, "Live invitation reported early");
        directory.Prune(now.AddSeconds(10));
        directory.Prune(now.AddSeconds(11));
        Check(wire.Single() is (0x11, MagRetailPartyDecline { Declined: 0x17, Reason: 4 } timeout) && timeout.PartyId == party,
            "Invitation expiry not reported once to the inviter");
        wire.Clear();
        Check(!directory.Apply(late, Client($"001B00" + "00000011" + "00000017" + $"{party:X8}"), now.AddSeconds(11)) && wire.Count == 0,
            "Expired invitation accepted");

        // An invite to someone already partied is answered with an in-party notice, not forwarded.
        var other = Online(0x1D);
        Check(directory.Apply(outsider, Client("001A000000000000001D00000019"), now) &&
            directory.Apply(other, Client($"001B00000000190000001D{((MagRetailPartyJoined)wire[0].Message).PartyId:X8}"), now),
            "Second party fixture failed");
        wire.Clear();
        Check(!directory.Apply(leader, Client($"001A{party:X8}0000001900000011"), now) &&
            wire.Single() is (0x11, MagRetailPartyDecline { Declined: 0x19, Reason: 3 }), "Partied invitee not reported as in party");
        wire.Clear();
        Check(!directory.Apply(stranger, Client("001A000000000000001300000011"), now) && wire.Count == 0,
            "Forged inviter drew a notice");

        // Open invite: only the leader toggles it, and every roster then carries it.
        wire.Clear();
        Check(!directory.Apply(friend, new MagRetailPartyOpenInvite { OpenInvite = true, PartyId = party }, now) && wire.Count == 0,
            "A non-leader changed open invite");
        var open = Client("0025" + Convert.ToHexString(MagMessageCodec.SerializeForApplication(
            new MagRetailPartyOpenInvite { OpenInvite = true, PartyId = party }, 20231)[2..]));
        Check(directory.Apply(leader, open, now) && wire.Count == 2 &&
            wire.All(x => x.Message is MagRetailPartyRoster { OpenInvite: true }) && wire[0].To == 0x11,
            "Open invite not reflected in every roster");
        wire.Clear();
        Check(directory.Apply(leader, open, now) && wire.Count == 0, "Unchanged open invite resent rosters");
        Check(directory.Apply(leader, new MagRetailPartyOpenInvite { OpenInvite = false, PartyId = party }, now) &&
            wire.Count == 2 && wire.All(x => x.Message is MagRetailPartyRoster { OpenInvite: false }), "Open invite not cleared");

        // Voice rejoin: recognised for a member and refused without a reply (no party voice world).
        wire.Clear();
        Check(directory.Apply(friend, Client($"0026{party:X8}"), now) && wire.Count == 0, "Voice rejoin not handled quietly");
        Check(!directory.Apply(shy, Client($"0026{party:X8}"), now), "Non-member voice rejoin accepted");
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
