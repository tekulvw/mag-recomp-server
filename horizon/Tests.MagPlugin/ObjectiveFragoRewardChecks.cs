using System.Text;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    private const string ObjectiveFragoCapability = "frago_objective_bonus_v1";
    private const string ObjectiveFragoFlag = "frago_objective_bonus_enabled";
    private const string ActivatedKind = "primary_objective_activated";
    private const string SecuredKind = "primary_objective_secured";
    private const string PrimaryCompletedKind = "primary_objective_completed";
    private const string SecondaryCompletedKind = "secondary_objective_completed";
    private const string PlantedKind = "charge_planted";
    private const string DefusedKind = "charge_defused";

    public static void ObjectiveFragoBridgeSchema()
    {
        const string qualified = ",\"frago\":true";
        foreach (string kind in new[] { ActivatedKind, SecuredKind, PrimaryCompletedKind, SecondaryCompletedKind, PlantedKind, DefusedKind })
        {
            Check(Parse(RewardReply(kind, ""), out var ordinary) && ordinary.Rewards.Single().Frago == null,
                $"Ordinary {kind} bridge receipt rejected");
            Check(Parse(RewardReply(kind, qualified), out var contextual) && contextual.Rewards.Single().Frago == true,
                $"Contextual {kind} bridge receipt rejected");
            foreach (int units in new[] { 0, 2 })
            {
                var response = RewardReply(kind, qualified);
                response["rewards"]![0]!["units"] = units;
                Check(!Parse(response, out _), $"Contextual {kind} accepted {units} units");
            }
            foreach (string malformed in new[] { "false", "null", "1", "\"true\"", "{}" })
                Check(!Parse(RewardReply(kind, ",\"frago\":" + malformed), out _), $"Contextual {kind} accepted {malformed}");
        }
        Check(!Parse(RewardReply("match_minor_victory", qualified), out _), "Final victory modifier accepted objective FRAGO context");
        string duplicate = RewardReply(PlantedKind, qualified).ToJsonString().Replace(qualified, qualified + qualified);
        Check(!GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(duplicate), "synthetic", true, out _),
            "Duplicate objective FRAGO context accepted");
    }

    public static void ObjectiveFragoAdmissionCapability() => WithDirectory(directory =>
    {
        foreach (var (applicationId, coefficient, capabilities, expected) in new (int, string, string[], bool?)[] {
            (20231, "0.5", Array.Empty<string>(), null),
            (20231, "0.5", new[] { FragoCapability, FragoActionCapability, RepairCapability }, null),
            (20231, "0.5", new[] { "FRAGO_OBJECTIVE_BONUS_V1" }, null),
            (20231, "0.5", new[] { ObjectiveFragoCapability }, true),
            (20231, null, new[] { ObjectiveFragoCapability, FragoCapability, FragoActionCapability, RepairCapability }, false),
            (20231, "0", new[] { ObjectiveFragoCapability, FragoCapability, FragoActionCapability, RepairCapability }, true),
            (20230, "0.5", new[] { ObjectiveFragoCapability, FragoCapability, FragoActionCapability, RepairCapability }, false)
        })
        {
            var catalog = RepairCatalog("1", coefficient);
            catalog["application_id"] = applicationId;
            var store = new LocalCharacterStore(Path.Combine(directory, $"characters-{Guid.NewGuid():N}.json")) {
                ProgressionCatalog = Load(directory, catalog, applicationId)
            };
            Create(store, "alpha", "Alpha");
            WithAdmission(store, async (host, bridge, session) =>
            {
                Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Objective FRAGO synthetic admission failed");
                var character = bridge.Latest("reserve")["character"]!.AsObject();
                Check(character.ContainsKey(ObjectiveFragoFlag) == expected.HasValue &&
                    character[ObjectiveFragoFlag]?.GetValue<bool>() == expected,
                    $"Wrong objective FRAGO opt-in for app {applicationId}, coefficient {coefficient}, and capabilities {string.Join(", ", capabilities)}");
                bool authoredRetail = applicationId == 20231 && coefficient != null;
                CheckAdmissionFlag(character, capabilities, FragoCapability, "frago_bonus_enabled", authoredRetail);
                CheckAdmissionFlag(character, capabilities, FragoActionCapability, "frago_action_bonus_enabled", authoredRetail);
                CheckAdmissionFlag(character, capabilities, RepairCapability, "repair_rewards_enabled", applicationId == 20231);
            }, applicationId, capabilities).GetAwaiter().GetResult();
        }
    });

    private static void CheckAdmissionFlag(JsonObject character, string[] capabilities, string capability, string flag, bool enabled)
    {
        bool? expected = capabilities.Contains(capability) ? enabled : null;
        Check(character.ContainsKey(flag) == expected.HasValue && character[flag]?.GetValue<bool>() == expected,
            $"Objective FRAGO negotiation changed {flag}");
    }

    private static JsonObject ObjectiveFragoCatalog(string coefficient)
    {
        var catalog = FragoCatalog(coefficient);
        var awards = catalog["table"]!["Awards"]!;
        awards["PrimaryObjectiveActivated"] = 0.125;
        awards["PrimaryObjectiveSecured"] = 2.25;
        awards["PrimaryObjective"] = 7.375;
        awards["SecondaryObjective"] = 3.75;
        awards["ChargePlanted"] = 4.375;
        awards["ChargeDefused"] = 5.75;
        return catalog;
    }

    public static void ObjectiveFragoAdmissionRetainedOnRelease() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var authored = Load(directory, ObjectiveFragoCatalog("0.5"));
        var missingCoefficient = Load(directory, ObjectiveFragoCatalog(null));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        Create(store, "alpha", "Alpha");

        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Older-worker synthetic admission failed");
            var admitted = bridge.Latest("reserve");
            byte[] before = File.ReadAllBytes(path);
            await SettleFragoReply(host, admitted, "older-worker-frago", kind: PlantedKind);
            Check(before.SequenceEqual(File.ReadAllBytes(path)) &&
                !bridge.Requests.Any(request => request["op"]!.GetValue<string>() == "progression_committed"),
                "Legacy worker was allowed to submit contextual objectives with other FRAGO capabilities");
            await SettleFragoReply(host, admitted, "older-worker-ordinary", qualifying: false, kind: PlantedKind);
            var ordinary = bridge.Latest("progression_committed");
            Check(ordinary["match_experience"]!.GetValue<int>() == 4 && !ordinary.ContainsKey("match_frago_points"),
                "Unnegotiated objective FRAGO changed ordinary objective XP");
        }, FragoCapability, FragoActionCapability).GetAwaiter().GetResult();

        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Objective-enabled admission failed");
            var enabled = bridge.Latest("reserve");
            await Release(host, enabled).WaitAsync(AdmissionTimeout);
            store.ProgressionCatalog = missingCoefficient;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Objective-disabled replacement admission failed");
            var disabled = bridge.Latest("reserve");
            Check(!JsonNode.DeepEquals(enabled["reservation_id"], disabled["reservation_id"]) &&
                disabled["character"]![ObjectiveFragoFlag]?.GetValue<bool>() == false, "Replacement reservation did not start disabled");

            store.ProgressionCatalog = authored;
            byte[] before = File.ReadAllBytes(path);
            await SettleFragoReply(host, disabled, "disabled-then-restored", kind: DefusedKind);
            Check(before.SequenceEqual(File.ReadAllBytes(path)) &&
                !bridge.Requests.Any(request => request["op"]!.GetValue<string>() == "progression_committed"),
                "A current catalog change retroactively enabled objective FRAGO for a reservation");

            await SettleFragoReply(host, enabled, "released-owner-pending", kind: DefusedKind);
            var retained = bridge.Latest("progression_committed");
            Check(JsonNode.DeepEquals(retained["reservation_id"], enabled["reservation_id"]) &&
                retained["match_frago_points"]!.GetValue<int>() == 2,
                "A released objective-enabled reservation lost its ability to settle a pending reward");
        }, ObjectiveFragoCapability).GetAwaiter().GetResult();
    });

    public static void ObjectiveFragoDurableSettlement() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        // The coefficient rounds to 2 in f32.
        var authored = Load(directory, ObjectiveFragoCatalog("1.99999999"));
        var missingCoefficient = Load(directory, ObjectiveFragoCatalog(null));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        Create(store, "alpha", "Alpha");
        Create(store, "beta", "Beta");
        var other = JsonNode.Parse(File.ReadAllBytes(path))!["characters"]![1]!.DeepClone();

        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Objective FRAGO synthetic admission failed");
            var admitted = bridge.Latest("reserve");
            await SettleFragoReply(host, admitted, "fractional-heal", qualifying: false, kind: "teammate_healed");
            var ordinary = bridge.Latest("progression_committed");
            Check(ordinary["match_experience"]!.GetValue<int>() == 0 && !ordinary.ContainsKey("match_frago_points"),
                "Ordinary heal did not leave a fractional carry");

            foreach (var (kind, experience, fragoPoints) in new[] {
                (ActivatedKind, 0, 0), (SecuredKind, 9, 6), (PlantedKind, 21, 14), (DefusedKind, 39, 26),
                (PrimaryCompletedKind, 60, 40), (SecondaryCompletedKind, 72, 48)
            })
            {
                await SettleFragoReply(host, admitted, kind, kind: kind);
                var ack = bridge.Latest("progression_committed");
                Check(ack["reward_id"]!.GetValue<string>() == kind && ack["match_experience"]!.GetValue<int>() == experience &&
                    ack["match_frago_points"]!.GetValue<int>() == fragoPoints && ack["progression"]!["experience"]!.GetValue<int>() == experience,
                    $"Wrong cumulative XP or FRAGO tally after {kind}");
            }
            var first = bridge.Requests.Single(request => request["reward_id"]?.GetValue<string>() == ActivatedKind);
            var promoted = bridge.Requests.Single(request => request["reward_id"]?.GetValue<string>() == DefusedKind);
            Check(promoted["progression"]!["level"]!.GetValue<byte>() == 2 &&
                promoted["leadership"]!["leadership_level"]!.GetValue<byte>() == 2,
                "Objective FRAGO XP did not advance ordinary character and leadership eligibility");

            int acknowledgements = bridge.Requests.Count;
            byte[] before = File.ReadAllBytes(path);
            store.ProgressionCatalog = missingCoefficient;
            await SettleFragoReply(host, admitted, "fresh-rule-missing", kind: PlantedKind);
            await SettleFragoReply(host, admitted, ActivatedKind, qualifying: false, kind: ActivatedKind);
            await SettleFragoReply(host, admitted, ActivatedKind, kind: SecuredKind);
            Check(bridge.Requests.Count == acknowledgements && before.SequenceEqual(File.ReadAllBytes(path)),
                "A fresh objective lacking its coefficient or a changed historical context committed");

            string match = $"world-{host.World.WorldId}-synthetic-match-123";
            var reopened = new LocalCharacterStore(path) { ProgressionCatalog = authored };
            var final = reopened.ApplyRewardForAccount("alpha", 1, "objective-round-result", match, "match_minor_victory", 1);
            Check(final.Result == MagNetResult.Success && final.Experience == 81 && final.Level == 3 &&
                final.MatchFraction == 0.25f && final.MatchFragoPoints == null,
                "Finalization lost objective FRAGO XP from the existing match basis or discarded shared carry");
            before = File.ReadAllBytes(path);
            Check(reopened.ApplyRewardForAccount("alpha", 1, "after-final", match, ActivatedKind, 1, frago: true).Result != MagNetResult.Success &&
                before.SequenceEqual(File.ReadAllBytes(path)), "A fresh objective FRAGO receipt committed after finalization");

            await SettleFragoReply(host, admitted, ActivatedKind, kind: ActivatedKind);
            var replay = bridge.Latest("progression_committed").DeepClone().AsObject();
            replay["request_id"] = first["request_id"]!.DeepClone();
            Check(JsonNode.DeepEquals(replay, first) && before.SequenceEqual(File.ReadAllBytes(path)),
                "Late replay after coefficient loss and finalization did not return the original zero ACK");
            Check(JsonNode.DeepEquals(other, JsonNode.Parse(before)!["characters"]![1]), "Objective FRAGO settlement changed another account");
        }, ObjectiveFragoCapability).GetAwaiter().GetResult();
    });

    public static void ObjectiveFragoOverflowAndInvalidContext() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var catalog = ObjectiveFragoCatalog("268435456");
        catalog["table"]!["Awards"]!["ChargePlanted"] = 8;
        var store = new LocalCharacterStore(path) { ProgressionCatalog = Load(directory, catalog) };
        Create(store, "alpha", "Alpha");
        byte[] before = File.ReadAllBytes(path);
        ProgressionRewardOutcome Settle(string kind, int units = 1, bool frago = true) =>
            store.ApplyRewardForAccount("alpha", 1, "objective", "round", kind, units, frago: frago);

        Check(Settle(PlantedKind).Reason == "invalid-authored-frago-bonus" && before.SequenceEqual(File.ReadAllBytes(path)),
            "An overflowing objective FRAGO product was accepted or partially committed");
        catalog["table"]!["Awards"]!["ChargePlanted"] = 1073741824;
        catalog["table"]!["Awards"]!["FRAGOBonusPointModifer"] = 1;
        store.ProgressionCatalog = Load(directory, catalog);
        Check(Settle(PlantedKind).Reason == "experience-overflow" && before.SequenceEqual(File.ReadAllBytes(path)),
            "An overflowing objective base-plus-bonus award was accepted or partially committed");
        foreach (var (kind, units, frago) in new[] {
            (PlantedKind, 0, true), (PlantedKind, 2, true), (PlantedKind, 1, false),
            ("match_minor_victory", 1, true), ("enemy_finished", 1, true)
        })
            Check(Settle(kind, units, frago).Reason == "invalid-progression-reward" && before.SequenceEqual(File.ReadAllBytes(path)),
                $"Invalid objective context reached direct settlement for {kind}, units={units}, frago={frago}");
    });
}
