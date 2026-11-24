using RT.Models.MAG;
using Server.Medius;

// Invented accounts and preference bytes built in memory; no client data or real stores.
internal static partial class LeadershipProgressionChecks
{
    private static string Volunteer(MagMlsSession session, byte nominations)
    {
        var replies = session.HandleMany(CardEnvelope(new MagRetailNominationsUpdate { Nominations = nominations }, session.ApplicationId), DateTime.UtcNow);
        Check(replies.Count == 0, "The lobby must not answer a nomination preference");
        return session.StorageStatus;
    }

    private static StoredLeadershipRecord SavedLeadership(LocalCharacterStore store) =>
        store.ReadForAccount("alpha").Single().Leadership;

    public static void NominationPreferenceWire()
    {
        // Type 0243 followed directly by the one bit-field byte, as the retail lobby carries it.
        byte[] payload = MagMessageCodec.SerializeForApplication(new MagRetailNominationsUpdate { Nominations = 1 }, 20231);
        Check(payload.SequenceEqual(new byte[] { 0x02, 0x43, 0x01 }), "Nomination preference wire width changed");
        Check(MagMessageCodec.TryParseClientForApplication(payload, 20231, out var parsed) &&
            parsed is MagRetailNominationsUpdate { Nominations: 1 }, "Retail lobby did not accept the nomination preference");
        Check(!MagMessageCodec.TryParseClientForApplication(payload, 20230, out _), "The prototype lobby must not claim retail 0243");
        Check(!MagMessageCodec.TryParseClientForApplication(new byte[] { 0x02, 0x43 }, 20231, out _), "A preference without its byte parsed");
        Check(!MagMessageCodec.TryParseClientForApplication(new byte[] { 0x02, 0x43, 0x01, 0x05 }, 20231, out _), "Trailing preference bytes parsed");
    }

    public static void NominationPreferenceAdmission() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, Catalog()) };
        Create(store, "alpha", "Alpha");
        Check(SavedLeadership(store).Nominations == 0, "Synthetic character must start without a saved nomination");
        var before = SavedLeadership(store);
        WithAdmission(store, async (host, bridge, session) =>
        {
            // Volunteer for squad leader, then join without waiting: the join's admission carries the bit.
            Check(Volunteer(session, 1) == "nominations-committed=0x01", "Squad-leader volunteer was not committed");
            Check(SavedLeadership(store).Nominations == 1 && store.ReadForAccount("alpha").Single().ToRecord().LeadershipNominations == 1,
                "Saved record does not carry the squad-leader nomination");
            Check(Volunteer(session, 1) == "nominations-unchanged=0x01", "Repeated preference must be idempotent");
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Volunteer admission failed");
            var reserve = bridge.Latest("reserve");
            Check(reserve["character"]!["leadership"]!["nominations"]!.GetValue<int>() == 1,
                "Gameplay admission did not receive the saved squad-leader nomination");
            await Release(host, reserve).WaitAsync(AdmissionTimeout);

            // Withdrawing is saved too, so the next admission is no longer a candidate.
            Check(Volunteer(session, 0) == "nominations-committed=0x00", "Withdrawn volunteer was not committed");
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Withdrawn admission failed");
            Check(bridge.Latest("reserve")["character"]!["leadership"]!["nominations"]!.GetValue<int>() == 0,
                "Gameplay admission kept a withdrawn nomination");

            // Bits outside squad/platoon/OIC are refused visibly and change nothing.
            Check(Volunteer(session, 0x09) == "unsupported-nomination-bits=0x09", "Unknown nomination bits were not refused");
            Check(SavedLeadership(store).Nominations == 0, "Refused nomination bits changed the saved record");
        }).GetAwaiter().GetResult();
        var after = SavedLeadership(store);
        Check(after.LeadershipLevel == before.LeadershipLevel && after.LeadershipPoints == before.LeadershipPoints &&
            after.LeadershipLevelFloor == before.LeadershipLevelFloor && after.MatchesSinceLastCommand == before.MatchesSinceLastCommand,
            "Saving a nomination changed the leadership rank, points or recency");
    });

    public static void NominationPreferenceLegacyAndUnbound() => WithDirectory(directory =>
    {
        // A store without leadership rules keeps no leadership record until the player chooses.
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        Create(store, "alpha", "Alpha");
        Check(SavedLeadership(store) == null, "Legacy synthetic character unexpectedly has a leadership record");
        WithApplication(20231, () =>
        {
            var session = MakeSession(store, 20231, "alpha", 1);
            Check(Volunteer(session, 3) == "nominations-committed=0x03", "Legacy row volunteer was not committed");
            var saved = SavedLeadership(store);
            Check(saved != null && saved.Nominations == 3 && saved.LeadershipLevel == 1 && saved.LeadershipPoints == null && saved.MatchesSinceLastCommand == 0,
                "Legacy row must gain the published level-1 record with only the chosen nominations");

            // A session that has not finished its handshake stores nothing.
            var now = DateTime.UtcNow;
            var registry = new MagLoginRegistry();
            var ticket = registry.IssueForAccount("alpha", 1, 1, now);
            var pending = new MagMlsSession { CharacterStore = store };
            Check(pending.TryBind(new RT.Models.RT_MSG_CLIENT_CONNECT_TCP { AppId = 20231, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey },
                registry, "synthetic-pending-channel", now) && pending.Accept(now) && pending.CompleteTransport(now), "Synthetic pending MLS session setup failed");
            Check(pending.HandleMany(CardEnvelope(new MagRetailNominationsUpdate { Nominations = 0 }, 20231), now).Count == 0 &&
                SavedLeadership(store).Nominations == 3, "A non-ready MLS session changed the saved nomination");
        });
    });
}
