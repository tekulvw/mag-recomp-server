using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    private const string RepairCapability = "repair_rewards_v1";
    private const string RepairAdmissionFlag = "repair_rewards_enabled";
    private const string RepairKind = "repair_units";

    private static JsonObject RepairCatalog(string ratio, string coefficient = "1", float? resuscitations = 1)
    {
        var catalog = FragoActionCatalog(coefficient, resuscitations);
        if (ratio != null)
            catalog["table"]!["Awards"]!["ExperiencePerRepairUnit"] = JsonNode.Parse(ratio);
        return catalog;
    }

    public static void RepairRewardSchema() => WithDirectory(directory =>
    {
        foreach (string ratio in new[] { null, "0", "0.1875" }) Load(directory, RepairCatalog(ratio));
        foreach (string invalid in new[] { "-0.25", "-1e-100", "1e100", "null", "true", "\"0.1875\"" })
        {
            try { Load(directory, RepairCatalog(invalid)); }
            catch (InvalidDataException) { continue; }
            throw new Exception("Invalid authored repair ratio accepted: " + invalid);
        }
        foreach (string context in new[] { "", ",\"frago\":true" })
        {
            foreach (var (units, accepted) in new[] { (0, false), (1, true), (65535, true), (65536, false) })
            {
                var response = RewardReply(RepairKind, context);
                response["rewards"]![0]!["units"] = units;
                Check(Parse(response, out _) == accepted, $"Wrong receipt validation for repair with {units} units");
            }
        }
        Check(!Parse(RewardReply(RepairKind, ",\"frago\":false"), out _), "Explicit false accepted on repair");
    });

    public static void RepairRewardAdmissionCompatibility() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var authored = Load(directory, RepairCatalog("0.25"));
        var missingRatio = Load(directory, RepairCatalog(null));
        var missingCoefficient = Load(directory, RepairCatalog("0.25", null));
        var missingRevive = Load(directory, RepairCatalog("0.25", resuscitations: null));
        var authoredZero = Load(directory, RepairCatalog("0"));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        Create(store, "alpha", "Alpha");
        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Older worker admission failed");
            var character = bridge.Latest("reserve")["character"]!.AsObject();
            Check(!character.ContainsKey(RepairAdmissionFlag) && character["frago_action_bonus_enabled"]?.GetValue<bool>() == true,
                "New Horizon sent an unknown repair field or affected older worker support negotiation");
        }, FragoCapability, FragoActionCapability, "REPAIR_REWARDS_V1").GetAwaiter().GetResult();

        WithAdmission(store, async (host, bridge, session) =>
        {
            store.ProgressionCatalog = missingRatio;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Missing-ratio worker admission failed");
            var noRepair = bridge.Latest("reserve");
            Check(noRepair["character"]![RepairAdmissionFlag]?.GetValue<bool>() == false &&
                noRepair["character"]!["frago_action_bonus_enabled"]?.GetValue<bool>() == true,
                "Missing repair ratio did not disable repair independently of existing support");
            store.ProgressionCatalog = authored;
            byte[] before = File.ReadAllBytes(path);
            int requests = bridge.Requests.Count;
            await SettleFragoReply(host, noRepair, "unnegotiated-repair", qualifying: false, kind: RepairKind, units: 4);
            Check(before.SequenceEqual(File.ReadAllBytes(path)) && bridge.Requests.Count == requests,
                "A disabled reservation committed or acknowledged unsolicited base repair");
            await Release(host, noRepair).WaitAsync(AdmissionTimeout);

            store.ProgressionCatalog = missingCoefficient;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Base-only repair admission failed");
            var baseOnly = bridge.Latest("reserve");
            Check(baseOnly["character"]![RepairAdmissionFlag]?.GetValue<bool>() == true &&
                baseOnly["character"]!["frago_action_bonus_enabled"]?.GetValue<bool>() == false,
                "Missing coefficient did not preserve base-only repair protocol");
            await SettleFragoReply(host, baseOnly, "ordinary-repair", qualifying: false, kind: RepairKind, units: 4);
            var ordinary = bridge.Latest("progression_committed");
            Check(ordinary["match_experience"]!.GetValue<int>() == 1 && !ordinary.ContainsKey("match_frago_points"),
                "Base-only repair did not commit ordinary XP");
            store.ProgressionCatalog = authored;
            before = File.ReadAllBytes(path);
            requests = bridge.Requests.Count;
            await SettleFragoReply(host, baseOnly, "unnegotiated-frago-repair", kind: RepairKind, units: 4);
            Check(before.SequenceEqual(File.ReadAllBytes(path)) && bridge.Requests.Count == requests,
                "A base-only reservation committed or acknowledged unsolicited FRAGO repair");
            await Release(host, baseOnly).WaitAsync(AdmissionTimeout);

            store.ProgressionCatalog = authoredZero;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Zero-ratio repair admission failed");
            var enabled = bridge.Latest("reserve");
            Check(enabled["character"]![RepairAdmissionFlag]?.GetValue<bool>() == true, "Authored zero disabled repair rewards");
            await SettleFragoReply(host, enabled, "zero-frago-repair", kind: RepairKind, units: 4);
            var zero = bridge.Latest("progression_committed");
            Check(zero["match_experience"]!.GetValue<int>() == 1 && zero["match_frago_points"]!.GetValue<int>() == 0,
                "Zero-ratio contextual receipt did not return the zero tally");
            await Release(host, enabled).WaitAsync(AdmissionTimeout);

            store.ProgressionCatalog = authored;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Full support repair admission failed");
            var fullSupport = bridge.Latest("reserve");
            Check(fullSupport["character"]![RepairAdmissionFlag]?.GetValue<bool>() == true &&
                fullSupport["character"]!["frago_action_bonus_enabled"]?.GetValue<bool>() == true,
                "Full authored repair and support rules did not enable both protocols");
            store.ProgressionCatalog = missingRevive;
            await SettleFragoReply(host, fullSupport, "repair-after-revive-rule-loss", kind: RepairKind, units: 4);
            var retained = bridge.Latest("progression_committed");
            Check(retained["reward_id"]!.GetValue<string>() == "repair-after-revive-rule-loss" &&
                retained["match_experience"]!.GetValue<int>() == 3 && retained["match_frago_points"]!.GetValue<int>() == 1,
                "An admitted FRAGO repair depended on a later unrelated revive rule");
            await Release(host, fullSupport).WaitAsync(AdmissionTimeout);

            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Repair admission without a revive rule failed");
            var withoutRevive = bridge.Latest("reserve");
            Check(withoutRevive["character"]![RepairAdmissionFlag]?.GetValue<bool>() == true &&
                withoutRevive["character"]!["frago_action_bonus_enabled"]?.GetValue<bool>() == false,
                "A fresh admission without a revive rule did not preserve base-only repair");
            await SettleFragoReply(host, withoutRevive, "base-repair-without-revive-rule", qualifying: false, kind: RepairKind, units: 4);
            var baseAfterLoss = bridge.Latest("progression_committed");
            Check(baseAfterLoss["match_experience"]!.GetValue<int>() == 4 && !baseAfterLoss.ContainsKey("match_frago_points"),
                "Base-only repair failed after admission without a revive rule");
        }, FragoCapability, FragoActionCapability, RepairCapability).GetAwaiter().GetResult();
    });

    public static void RepairRewardDurableSettlement() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var authored = Load(directory, RepairCatalog("0.1875"));
        var missingRatio = Load(directory, RepairCatalog(null));
        var oversizedAward = Load(directory, RepairCatalog("40000"));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        Create(store, "alpha", "Alpha");
        WithAdmission(store, async (host, bridge, session) =>
        {
            string match = $"world-{host.World.WorldId}-synthetic-match-123";
            var seed = store.ApplyRewardForAccount("alpha", 1, "heal-fraction", match, "teammate_healed", 1);
            Check(seed.MatchFraction == 0.625f, "Shared XP carry setup failed");
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Repair synthetic admission failed");
            var admitted = bridge.Latest("reserve");
            await SettleFragoReply(host, admitted, "repair-below-whole", qualifying: false, kind: RepairKind);
            var fractional = bridge.Latest("progression_committed");
            Check(fractional["match_experience"]!.GetValue<int>() == 0 && !fractional.ContainsKey("match_frago_points"),
                "Fractional repair produced ordinary or FRAGO whole XP");
            await SettleFragoReply(host, admitted, "repair-cross-whole", kind: RepairKind);
            var crossed = bridge.Latest("progression_committed");
            Check(crossed["match_experience"]!.GetValue<int>() == 2 && crossed["match_frago_points"]!.GetValue<int>() == 1,
                "The repair crossing the whole XP threshold did not own the FRAGO bonus");
            await SettleFragoReply(host, admitted, "repair-bulk", kind: RepairKind, units: 127);
            var bulk = bridge.Latest("progression_committed");
            Check(bulk["match_experience"]!.GetValue<int>() == 48 && bulk["match_frago_points"]!.GetValue<int>() == 24 &&
                bulk["progression"]!["level"]!.GetValue<int>() == 2 && bulk["progression"]!["skill_points"]!.GetValue<int>() == 3 &&
                bulk["leadership"]!["leadership_level"]!.GetValue<int>() == 2,
                "Bulk repair did not award the authored quantity, whole bonus and progression atomically");

            store.ProgressionCatalog = oversizedAward;
            byte[] before = File.ReadAllBytes(path);
            int requests = bridge.Requests.Count;
            await SettleFragoReply(host, admitted, "repair-xp-overflow", qualifying: false, kind: RepairKind, units: 65535);
            Check(before.SequenceEqual(File.ReadAllBytes(path)) && bridge.Requests.Count == requests,
                "Overflowing repair XP partly committed or was acknowledged");
            store.ProgressionCatalog = missingRatio;
            await SettleFragoReply(host, admitted, "repair-no-ratio", qualifying: false, kind: RepairKind);
            await SettleFragoReply(host, admitted, "frago-repair-no-ratio", kind: RepairKind);
            Check(before.SequenceEqual(File.ReadAllBytes(path)) && bridge.Requests.Count == requests,
                "A fresh repair receipt settled with no authored ratio");

            var reopened = new LocalCharacterStore(path) { ProgressionCatalog = missingRatio };
            var final = reopened.ApplyRewardForAccount("alpha", 1, "round-result", match, "match_minor_victory", 1);
            Check(final.Result == MagNetResult.Success && final.Experience == 54 && final.MatchFraction == 0.8125f,
                "Final award did not preserve repair XP and shared carry");
            before = File.ReadAllBytes(path);
            requests = bridge.Requests.Count;
            await SettleFragoReply(host, admitted, "repair-cross-whole", qualifying: false, kind: RepairKind);
            await SettleFragoReply(host, admitted, "repair-cross-whole", kind: RepairKind, units: 2);
            Check(before.SequenceEqual(File.ReadAllBytes(path)) && bridge.Requests.Count == requests,
                "A repair replay changed units or qualification on an immutable receipt");
            await SettleFragoReply(host, admitted, "repair-cross-whole", kind: RepairKind);
            var replay = bridge.Latest("progression_committed").DeepClone().AsObject();
            replay["request_id"] = crossed["request_id"]!.DeepClone();
            Check(JsonNode.DeepEquals(replay, crossed) && before.SequenceEqual(File.ReadAllBytes(path)),
                "Finalized contextual repair did not replay the original XP and tally without the current ratio");
        }, FragoActionCapability, RepairCapability).GetAwaiter().GetResult();
    });
}
