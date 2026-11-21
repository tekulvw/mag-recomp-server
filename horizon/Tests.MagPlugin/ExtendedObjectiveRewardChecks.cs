using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    private const string ExtendedObjectiveCapability = "extended_objective_rewards_v1";
    private const string ExtendedObjectiveFlag = "extended_objective_rewards_enabled";
    private static readonly (string Kind, string Key, float Amount, int MatchXp, int FragoPoints)[] ExtendedObjectives = {
        ("defensive_emplacements_destroyed", "DefensiveEmplacementDestroyed", 0.125f, 0, 0),
        ("bridges_destroyed", "BridgeDestroyed", 1.25f, 4, 2),
        ("fortifications_destroyed", "FortificationDestroyed", 2.375f, 8, 4),
        ("gates_destroyed", "GateDestroyed", 3.5f, 14, 7),
        ("mortars_destroyed", "MortarDestroyed", 4.625f, 24, 12),
        ("sensor_networks_destroyed", "SensorNetworkDestroyed", 5.75f, 36, 18),
        ("anti_aircraft_destroyed", "AntiAircraftDestroyed", 6.875f, 50, 25),
        ("motor_pools_destroyed", "MotorPoolsDestroyed", 8f, 66, 33),
        ("escort_vehicle_delivered", "EscortVehicleDelivered", 9.125f, 84, 42)
    };

    private static JsonObject ExtendedObjectiveCatalog(string coefficient, int count = 9, bool zeroAwards = false)
    {
        var catalog = ObjectiveFragoCatalog(coefficient);
        var awards = catalog["table"]!["Awards"]!;
        foreach (var entry in ExtendedObjectives.Take(count))
            awards[entry.Key] = zeroAwards ? 0 : entry.Amount;
        return catalog;
    }

    public static void ExtendedObjectiveAdmissionCapability() => WithDirectory(directory =>
    {
        foreach (var (app, coefficient, count, zeroAwards, capabilities, expected) in
            new (int, string, int, bool, string[], bool?)[] {
                (20231, "1", 9, false, new[] { ObjectiveFragoCapability }, null),
                (20231, "1", 9, false, new[] { "EXTENDED_OBJECTIVE_REWARDS_V1" }, null),
                (20231, null, 9, false, new[] { ExtendedObjectiveCapability, ObjectiveFragoCapability }, true),
                (20231, "1", 0, false, new[] { ExtendedObjectiveCapability, ObjectiveFragoCapability }, false),
                (20231, "1", 8, false, new[] { ExtendedObjectiveCapability, ObjectiveFragoCapability }, false),
                (20231, "0", 9, true, new[] { ExtendedObjectiveCapability, ObjectiveFragoCapability }, true),
                (20230, "1", 9, false, new[] { ExtendedObjectiveCapability, ObjectiveFragoCapability }, false)
            })
        {
            var catalog = ExtendedObjectiveCatalog(coefficient, count, zeroAwards);
            catalog["application_id"] = app;
            var store = new LocalCharacterStore(Path.Combine(directory, $"characters-{Guid.NewGuid():N}.json")) {
                ProgressionCatalog = Load(directory, catalog, app)
            };
            Create(store, "alpha", "Alpha");
            WithAdmission(store, async (host, bridge, session) =>
            {
                Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Extended-objective admission failed");
                var character = bridge.Latest("reserve")["character"]!.AsObject();
                Check(character.ContainsKey(ExtendedObjectiveFlag) == expected.HasValue &&
                    character[ExtendedObjectiveFlag]?.GetValue<bool>() == expected,
                    $"Wrong extended-objective opt-in for app {app}, coefficient {coefficient}, {count} awards and {string.Join(", ", capabilities)}");
                CheckAdmissionFlag(character, capabilities, ObjectiveFragoCapability, ObjectiveFragoFlag, app == 20231 && coefficient != null);
            }, app, capabilities).GetAwaiter().GetResult();
        }
    });

    public static void ExtendedObjectiveBridgeSchema()
    {
        const string qualified = ",\"frago\":true";
        foreach (var entry in ExtendedObjectives)
        {
            Check(Parse(RewardReply(entry.Kind, ""), out var ordinary) && ordinary.Rewards.Single().Frago == null,
                $"Ordinary {entry.Kind} receipt rejected");
            Check(Parse(RewardReply(entry.Kind, qualified), out var contextual) && contextual.Rewards.Single().Frago == true,
                $"Contextual {entry.Kind} receipt rejected");
            foreach (int units in new[] { 0, 2 })
                foreach (string context in new[] { "", qualified })
                {
                    var response = RewardReply(entry.Kind, context);
                    response["rewards"]![0]!["units"] = units;
                    Check(!Parse(response, out _), $"{entry.Kind} accepted {units} units");
                }
            foreach (string invalid in new[] { "false", "null", "\"true\"", "1" })
                Check(!Parse(RewardReply(entry.Kind, ",\"frago\":" + invalid), out _), $"{entry.Kind} accepted malformed FRAGO context");
        }
        Check(!Parse(RewardReply("charges_detonated", ""), out _), "Charges Detonated became an XP receipt kind");
    }

    public static void ExtendedObjectiveCatalogValidation() => WithDirectory(directory =>
    {
        foreach (string invalid in new[] { "-0.25", "-1e-100", "1e100", "2147483648", "null", "\"1\"" })
        {
            var catalog = ExtendedObjectiveCatalog(null);
            catalog["table"]!["Awards"]!["EscortVehicleDelivered"] = JsonNode.Parse(invalid);
            try { Load(directory, catalog); }
            catch (InvalidDataException) { continue; }
            throw new Exception("Invalid authored extended objective award accepted: " + invalid);
        }

        string path = Path.Combine(directory, "characters.json");
        var store = new LocalCharacterStore(path) { ProgressionCatalog = Load(directory, ExtendedObjectiveCatalog("1")) };
        Create(store, "alpha", "Alpha");
        var before = File.ReadAllBytes(path);
        foreach (var missing in ExtendedObjectives)
        {
            var partial = ExtendedObjectiveCatalog("1");
            partial["table"]!["Awards"]!.AsObject().Remove(missing.Key);
            store.ProgressionCatalog = Load(directory, partial);
            foreach (var entry in ExtendedObjectives)
                Check(store.ApplyRewardForAccount("alpha", 1, "partial", "round", entry.Kind, 1).Result != MagNetResult.Success,
                    $"Partial group authorized {entry.Kind} with missing {missing.Key}");
            Check(before.SequenceEqual(File.ReadAllBytes(path)), "Partial extended objective group changed the ledger");
        }
        Check(store.ApplyRewardForAccount("alpha", 1, "old-base", "round", PlantedKind, 1).ExperienceAwarded == 4,
            "Partial group changed an existing objective award");
    });

    public static void ExtendedObjectiveAdmissionRetained() => WithDirectory(directory =>
    {
        const string delivery = "escort_vehicle_delivered";
        string path = Path.Combine(directory, "characters.json");
        var complete = Load(directory, ExtendedObjectiveCatalog("1"));
        var partial = Load(directory, ExtendedObjectiveCatalog("1", 8));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = complete };
        Create(store, "alpha", "Alpha");

        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Old worker admission failed");
            var old = bridge.Latest("reserve");
            byte[] before = File.ReadAllBytes(path);
            await SettleFragoReply(host, old, "unnegotiated-base", qualifying: false, kind: delivery);
            await SettleFragoReply(host, old, "unnegotiated-context", kind: delivery);
            Check(NoExtendedAck(bridge) && before.SequenceEqual(File.ReadAllBytes(path)),
                "Old worker sent a new kind using its objective FRAGO opt-in");
            await SettleFragoReply(host, old, "old-context", kind: PlantedKind);
            Check(bridge.Latest("progression_committed")["match_frago_points"]?.GetValue<int>() == 4,
                "New capability changed an existing objective FRAGO reward");
        }, ObjectiveFragoCapability).GetAwaiter().GetResult();

        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Base-only admission failed");
            var enabled = bridge.Latest("reserve");
            byte[] before = File.ReadAllBytes(path);
            await SettleFragoReply(host, enabled, "unnegotiated-context", kind: delivery);
            Check(NoExtendedAck(bridge) && before.SequenceEqual(File.ReadAllBytes(path)),
                "New base admission alone authorized contextual XP");
            await Release(host, enabled).WaitAsync(AdmissionTimeout);
            store.ProgressionCatalog = partial;
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Partial-catalog replacement admission failed");
            var disabled = bridge.Latest("reserve");
            Check(disabled["character"]![ExtendedObjectiveFlag]?.GetValue<bool>() == false, "Replacement unexpectedly enabled the partial catalog");
            store.ProgressionCatalog = complete;
            await SettleFragoReply(host, disabled, "restored-after-admission", qualifying: false, kind: delivery);
            Check(NoExtendedAck(bridge) && before.SequenceEqual(File.ReadAllBytes(path)),
                "Catalog restoration retroactively enabled a replacement reservation");
            await SettleFragoReply(host, enabled, "released-owner-base", qualifying: false, kind: delivery);
            var ack = bridge.Latest("progression_committed");
            Check(JsonNode.DeepEquals(ack["reservation_id"], enabled["reservation_id"]) &&
                ack["match_experience"]?.GetValue<int>() == 9 && !ack.ContainsKey("match_frago_points"),
                "Released owner lost its original base-only admission");
        }, ExtendedObjectiveCapability).GetAwaiter().GetResult();
    });

    private static bool NoExtendedAck(FixtureBridge bridge) =>
        !bridge.Requests.Any(request => request["op"]!.GetValue<string>() == "progression_committed");

    public static void ExtendedObjectiveDurableSettlement() => WithDirectory(directory =>
    {
        string path = Path.Combine(directory, "characters.json");
        var authored = Load(directory, ExtendedObjectiveCatalog("1"));
        var changed = Load(directory, ExtendedObjectiveCatalog("0", zeroAwards: true));
        var partial = Load(directory, ExtendedObjectiveCatalog("1", 8));
        var store = new LocalCharacterStore(path) { ProgressionCatalog = authored };
        Create(store, "alpha", "Alpha");
        WithAdmission(store, async (host, bridge, session) =>
        {
            Check((await BeginJoin(host, session).WaitAsync(AdmissionTimeout)).Result == 0, "Full extended-objective admission failed");
            var admitted = bridge.Latest("reserve");
            await SettleFragoReply(host, admitted, "fractional-heal", qualifying: false, kind: "teammate_healed");
            foreach (var entry in ExtendedObjectives)
            {
                await SettleFragoReply(host, admitted, entry.Kind, kind: entry.Kind);
                var ack = bridge.Latest("progression_committed");
                Check(ack["reward_id"]?.GetValue<string>() == entry.Kind &&
                    ack["match_experience"]?.GetValue<int>() == entry.MatchXp &&
                    ack["progression"]!["experience"]?.GetValue<int>() == entry.MatchXp &&
                    ack["match_frago_points"]?.GetValue<int>() == entry.FragoPoints,
                    $"Wrong authored XP, shared carry, or stat 3 tally for {entry.Kind}");
            }
            string match = $"world-{host.World.WorldId}-synthetic-match-123";
            var reopened = new LocalCharacterStore(path) { ProgressionCatalog = authored };
            var final = reopened.ApplyRewardForAccount("alpha", 1, "round-result", match, "match_minor_victory", 1);
            Check(final.Result == MagNetResult.Success && final.Experience == 94 && final.MatchFraction == 0.25f &&
                final.MatchFragoPoints == null, "Final result omitted extended XP from its existing basis");
            byte[] before = File.ReadAllBytes(path);
            const string bridgeKind = "bridges_destroyed";
            var saved = bridge.Requests.Single(request => request["reward_id"]?.GetValue<string>() == bridgeKind);
            foreach (var current in new[] { changed, partial })
            {
                store.ProgressionCatalog = current;
                await SettleFragoReply(host, admitted, bridgeKind, kind: bridgeKind);
                var replay = bridge.Latest("progression_committed").DeepClone().AsObject();
                replay["request_id"] = saved["request_id"]!.DeepClone();
                Check(JsonNode.DeepEquals(replay, saved) && before.SequenceEqual(File.ReadAllBytes(path)),
                    "Changed or incomplete current rules changed historical ACK");
            }
            int acknowledgements = bridge.Requests.Count;
            await SettleFragoReply(host, admitted, bridgeKind, qualifying: false, kind: bridgeKind);
            await SettleFragoReply(host, admitted, bridgeKind, kind: "escort_vehicle_delivered");
            Check(bridge.Requests.Count == acknowledgements && before.SequenceEqual(File.ReadAllBytes(path)),
                "Conflicting extended-kind or FRAGO context replay was acknowledged");
        }, ExtendedObjectiveCapability, ObjectiveFragoCapability).GetAwaiter().GetResult();
    });
}
