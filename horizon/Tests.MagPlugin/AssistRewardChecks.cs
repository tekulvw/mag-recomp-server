using System.Text.Json.Nodes;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    private const string AssistKind = "enemy_kill_assist";

    public static void AssistRewardAdmission() => WithDirectory(directory =>
    {
        const string capability = "assist_rewards_v1";
        const string flag = "assist_rewards_enabled";
        string path = Path.Combine(directory, "characters.json");
        var catalog = FragoActionCatalog("1", null);
        var missing = Load(directory, catalog);
        catalog["table"]!["Awards"]!["EnemyKillAssist"] = 2.5;
        var authored = Load(directory, catalog);
        var store = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        Create(store, "alpha", "Alpha");
        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Older worker admission failed");
            Check(!bridge.Latest("reserve")["character"]!.AsObject().ContainsKey(flag),
                "Horizon sent an unknown assist field to the older worker");
        }, FragoCapability).GetAwaiter().GetResult();
        WithAdmission(store, async (host, bridge, session) =>
        {
            store.ProgressionCatalog = missing;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Missing award prevented joining");
            var disabled = bridge.Latest("reserve");
            Check(disabled["character"]![flag]!.GetValue<bool>() == false, "An absent award enabled assist XP");
            byte[] before = File.ReadAllBytes(path);
            store.ProgressionCatalog = authored;
            await SettleFragoReply(host, disabled, "unnegotiated-assist", qualifying: false, kind: AssistKind);
            Check(before.SequenceEqual(File.ReadAllBytes(path)), "Admission-disabled assist mutated the store");
            await Release(host, disabled).WaitAsync(AdmissionTimeout);

            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Authored assist prevented joining");
            var enabled = bridge.Latest("reserve");
            Check(enabled["character"]![flag]!.GetValue<bool>(), "The authored assist award did not enable XP");
            await SettleFragoReply(host, enabled, "qualified-assist", kind: AssistKind);
            var acknowledgement = bridge.Latest("progression_committed");
            Check(acknowledgement["match_experience"]!.GetValue<int>() == 4 &&
                acknowledgement["match_frago_points"]!.GetValue<int>() == 2,
                "Assist FRAGO depended on unrelated revive/support rules");
        }, FragoCapability, capability).GetAwaiter().GetResult();
    });

    public static void AssistRewardDurableSettlement() => WithDirectory(directory =>
    {
        const string match = "world-11-synthetic-assist-round";
        string path = Path.Combine(directory, "characters.json");
        JsonObject catalog = FragoActionCatalog("1", null);
        catalog["table"]!["Awards"]!["EnemyKillAssist"] = 2.5;
        var authored = Load(directory, catalog);
        var store = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        Create(store, "alpha", "Alpha");

        Check(Parse(RewardReply(AssistKind, ",\"frago\":true"), out _), "Bridge rejected contextual assist");
        var badUnits = RewardReply(AssistKind, "");
        badUnits["rewards"]![0]!["units"] = 2;
        Check(!Parse(badUnits, out _), "Bridge admitted bulk assist");
        var seed = store.ApplyRewardForAccount("alpha", 1, "heal-fraction", match, "teammate_healed", 1);
        Check(seed.MatchFraction == 0.625f, "Assist carry fixture was not seeded");
        var earned = store.ApplyRewardForAccount("alpha", 1, "assist-first", match, AssistKind, 1, frago: true);
        Check(earned.MatchExperience == 6 && earned.MatchFraction == 0.125f && earned.MatchFragoPoints == 3,
            "Assist did not settle shared carry, base and contextual XP");
        var reopened = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        byte[] saved = File.ReadAllBytes(path);
        var replay = reopened.ApplyRewardForAccount("alpha", 1, "assist-first", match, AssistKind, 1, frago: true);
        Check(replay.MatchExperience == earned.MatchExperience && saved.SequenceEqual(File.ReadAllBytes(path)),
            "Assist receipt was applied twice after reopen");
    });
}
