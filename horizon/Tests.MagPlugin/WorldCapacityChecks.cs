using System.Net;
using System.Text.Json;
using System.Text.Json.Nodes;
using Server.Medius;

// Invented profiles, queue identities and registry rows only (world_capacity_v1).
internal static class WorldCapacityChecks
{
    static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
    static void Throws<T>(Action action, string message) where T : Exception
    {
        try { action(); }
        catch (T) { return; }
        throw new Exception(message);
    }

    static JsonObject Row(int maximum, int team, int? humans = null, int? teamHumans = null, int? bots = null)
    {
        var row = new JsonObject { ["profile_id"] = $"invented-{maximum}", ["game_type"] = 2, ["attacking_faction"] = 0,
            ["defending_faction"] = 1, ["maximum_players"] = maximum, ["team_capacity"] = team };
        if (humans is int h) row["human_capacity"] = h;
        if (teamHumans is int t) row["human_team_capacity"] = t;
        if (bots is int b) row["bot_fill_total"] = b;
        return row;
    }
    static bool Parse(JsonObject row, bool worldCapacity, out MagMatchProfile profile)
    {
        var capabilities = new JsonArray("match_profiles_v1");
        if (worldCapacity) capabilities.Add("world_capacity_v1");
        var reply = new JsonObject { ["v"] = 2, ["request_id"] = "synthetic", ["ok"] = true, ["error"] = null,
            ["messages"] = new JsonArray(), ["capabilities"] = capabilities, ["match_profiles"] = new JsonArray(row) };
        profile = null;
        if (!GameplayBridgeResponseCodec.TryParse(JsonSerializer.SerializeToUtf8Bytes(reply), "synthetic", true, out var hello)) return false;
        profile = hello.MatchProfiles.Single();
        return true;
    }

    public static void Codec()
    {
        // Legacy rows keep their meaning with or without the capability: every seat may be human.
        foreach (bool capability in new[] { false, true })
        {
            Check(Parse(Row(64, 32, bots: 64), capability, out var legacy) && legacy.HumanCapacity == 64 &&
                legacy.HumanTeamCapacity == 32 && !legacy.LargeWorld && legacy.AdvertisedHumanCapacity == null,
                "A 64-Character row changed meaning");
            Check(!Parse(Row(65, 33), capability, out _), "A row above 64 without human caps was accepted");
        }
        // Rows above 64 need the capability and the human caps.
        Check(!Parse(Row(128, 64, 32, 16), false, out _), "A 128-Character row was accepted without world_capacity_v1");
        Check(!Parse(Row(64, 32, 32, 16), false, out _), "Human caps were accepted without world_capacity_v1");
        Check(Parse(Row(128, 64, 32, 16, 128), true, out var acquisition) && acquisition.LargeWorld &&
            acquisition.MaximumPlayers == 128 && acquisition.TeamCapacity == 64 && acquisition.HumanCapacity == 32 &&
            acquisition.HumanTeamCapacity == 16 && acquisition.BotFillTotal == 128, "A 128-Character row was refused or mistyped");
        Check(Parse(Row(256, 128, 32, 16, 256), true, out var domination) && domination.HumanCapacity == 32,
            "A 256-Character row was refused");
        Check(Parse(Row(64, 32, 32, 16), true, out var capped) && capped.HumanCapacity == 32 && capped.HumanTeamCapacity == 16,
            "Optional human caps on a 64-Character row were refused");
        foreach (var bad in new[]
        {
            Row(128, 64), Row(128, 64, 32), Row(128, 64, teamHumans: 16), Row(257, 129, 32, 16), Row(256, 129, 32, 16),
            Row(130, 64, 32, 16), Row(128, 64, 33, 16), Row(128, 64, 32, 17), Row(128, 64, 32, 15), Row(128, 64, 15, 16),
            Row(128, 64, 32, 0), Row(128, 64, 32, 16, 129), Row(128, 64, 32, 16, 1)
        })
            Check(!Parse(bad, true, out _), $"An invalid world_capacity_v1 row was accepted: {bad.ToJsonString()}");
        Check(!Parse(Row(16, 8, 32, 16), true, out _), "Human caps above world size were accepted");

        // The worker contract pins the capability and a large row's caps; a legacy row's description is unchanged.
        Check(MagWorkerContract.RegistryCapabilities.Contains("world_capacity_v1"), "world_capacity_v1 is not a registry capability");
    }

    static MagMatchProfile Large(byte attacker = 0, byte defender = 1, int world = 256, int? bots = 256) =>
        new($"invented-large-{attacker}-{defender}", 2, attacker, defender, world, world / 2, BotFillTotal: bots,
            AdvertisedHumanCapacity: 32, AdvertisedHumanTeamCapacity: 16);
    static MagQueuedHuman[] Queue(int count, byte faction, long start = 0, int mode = 2) =>
        Enumerable.Range(0, count).Select(i => new MagQueuedHuman($"human-{faction}-{start + i:000}", mode, faction, start + i)).ToArray();

    public static void Matchmaking()
    {
        var profile = Large();
        Throws<ArgumentException>(() => MagFactionMatchmaking.ForProfile(Queue(1, 0), profile with { AdvertisedHumanCapacity = null,
            AdvertisedHumanTeamCapacity = null }, 1, 64, new[] { 0, 0 }), "A large profile without human caps was accepted");
        Throws<ArgumentException>(() => MagFactionMatchmaking.ForProfile(Queue(1, 0), profile with { AdvertisedHumanTeamCapacity = 17 },
            1, 64, new[] { 0, 0 }), "A large profile with 17 humans per side was accepted");

        // At most 16 humans per side, independently of 128 seats per side.
        var oneSide = MagFactionMatchmaking.ForProfile(Queue(40, 0), profile, 1, 64, new[] { 0, 0 });
        Check(oneSide?.Players.Count == 16, "A 256-Character world took more than 16 humans on one side");
        // At most 32 humans per world, even when the operator's maximum is 64.
        var both = MagFactionMatchmaking.ForProfile(Queue(40, 0).Concat(Queue(40, 1, 100)).ToArray(), profile, 1, 64, new[] { 0, 0 });
        Check(both?.Players.Count == 32 && both.Players.Count(p => p.Faction == 0) == 16, "A 256-Character world took more than 32 humans");
        // The same caps apply at 128.
        var acquisition = MagFactionMatchmaking.ForProfile(Queue(40, 0).Concat(Queue(40, 1, 100)).ToArray(), Large(world: 128, bots: 128),
            1, 64, new[] { 0, 0 });
        Check(acquisition?.Players.Count == 32, "A 128-Character world took more than 32 humans");
        // A side minimum above the human side cap cannot form.
        Check(MagFactionMatchmaking.ForProfile(Queue(40, 0).Concat(Queue(40, 1, 100)).ToArray(), profile, 2, 64, new[] { 17, 1 }) == null,
            "A side minimum above 16 humans formed");
        // New formation over a full ordered-pair matrix of large profiles.
        var profiles = new[] { Large(0, 1), Large(1, 0), Large(0, 2), Large(2, 0), Large(1, 2), Large(2, 1) };
        var formed = MagFactionMatchmaking.Select(Queue(40, 0).Concat(Queue(40, 1, 100)).ToArray(), profiles, 1, 64, new[] { 0, 0 }, n => 0);
        Check(formed?.Players.Count == 32, "A new large formation exceeded the human cap");
        // A legacy 64-Character row still admits 32 per side and 64 in all.
        var legacy = new MagMatchProfile("invented-legacy", 2, 0, 1, 64, 32);
        Check(MagFactionMatchmaking.ForProfile(Queue(40, 0).Concat(Queue(40, 1, 100)).ToArray(), legacy, 2, 64, new[] { 1, 1 })?.Players.Count == 64,
            "A 64-Character row lost its 64-human admission");
    }

    public static void GroupedCohortBound()
    {
        // Squads of 8 on 16 squads per side; the ulong masks count humans per side only.
        var profile = Large() with { SquadCapacity = 8, SquadCounts = new[] { 16, 16 } };
        var waiting = new List<MagQueuedHuman>();
        long order = 0;
        foreach (byte faction in new byte[] { 0, 1 })
        {
            for (int party = 0; party < 3; party++)
                waiting.Add(new MagQueuedHuman($"party-{faction}-{party}", 2, faction, order++, Size: 8));
            waiting.AddRange(Queue(10, faction, 1000 + faction * 100));
        }
        var grouped = MagFactionMatchmaking.ForProfile(waiting, profile, 1, 64, new[] { 0, 0 });
        Check(grouped != null && grouped.Players.Count(p => p.Size == 8) == 4 &&
            grouped.Players.Where(p => p.Faction == 0).Sum(p => p.Size) == 16 &&
            grouped.Players.Where(p => p.Faction == 1).Sum(p => p.Size) == 16, "Grouped selection did not fill exactly 16 per side");
        // Exactly at the side bound: two whole squads of 8 fill 16, a third never joins.
        Check(grouped.Players.Where(p => p.Faction == 0 && p.Size == 8).Count() == 2, "A third squad of 8 exceeded the side cap");
        Check(MagWorldCapacity.MaximumHumansPerTeam <= MagWorldCapacity.MaximumMaskedSide, "Human side cap exceeds the mask bound");
    }

    static MagWorldPlacementState Running(MagMatchProfile profile, int[] counts, int maximum = 64, bool parties = false) =>
        new("large", true, false, 1, maximum, new[] { 0, 0 }, new[] { profile }, profile, counts, new[] { 0, 0, 0 },
            counts.Sum(), 0, false, parties);

    public static void Placement()
    {
        var profile = Large() with { SquadCapacity = 8, SquadCounts = new[] { 16, 16 } };
        // Late solos stop at 16 per side and 32 per world.
        var late = MagWorldPlacement.PlanNext(Queue(10, 0), new[] { Running(profile, new[] { 12, 0, 0 }) }, n => 0);
        Check(late != null && !late.NewFormation && late.Players.Count == 4, "Late join passed 16 humans on one side");
        Check(MagWorldPlacement.PlanNext(Queue(1, 0), new[] { Running(profile, new[] { 16, 3, 0 }) }, n => 0) == null,
            "A full human side admitted another late human");
        Check(MagWorldPlacement.PlanNext(Queue(1, 1), new[] { Running(profile, new[] { 16, 16, 0 }) }, n => 0) == null,
            "A world with 32 humans admitted another");
        // A late party fits the human side cap, not the 128-Character side.
        var party = new[] { new MagQueuedHuman("party", 2, 0, 0, Size: 8) };
        Check(MagWorldPlacement.PlanNext(party, new[] { Running(profile, new[] { 8, 0, 0 }, parties: true) }, n => 0)?.Players.Single().Size == 8,
            "A late party that fits 16 per side was refused");
        Check(MagWorldPlacement.PlanNext(party, new[] { Running(profile, new[] { 9, 0, 0 }, parties: true) }, n => 0) == null,
            "A late party exceeded 16 humans on its side");
        // The DME port budget still bounds humans across worlds.
        Check(MagWorldPlacement.PlanNext(Queue(10, 0), new[] { Running(profile, new[] { 0, 0, 0 }) }, n => 0, humanRoom: 3)?.Players.Count == 3,
            "The DME port budget no longer bounds a large world");
    }

    public static void Registry()
    {
        JsonObject Config(int maximum) => new()
        {
            ["format"] = "mag_world_registry_v1", ["application_id"] = 20231, ["client_address"] = "127.0.0.1", ["client_tcp_port"] = 10473,
            ["workers"] = new JsonArray(new JsonObject { ["worker_id"] = "large", ["worker_host"] = "127.0.0.1", ["worker_port"] = 31001,
                ["minimum_players"] = 1, ["maximum_players"] = maximum, ["minimum_players_per_team"] = new JsonArray(0, 0), ["draining"] = false })
        };
        foreach (int maximum in new[] { 64, 128, 256 })
            Check(MagWorldRegistryConfig.Parse(Config(maximum).ToJsonString()).Workers.Single().MaximumPlayers == maximum,
                "A registry world capacity up to 256 was refused");
        Throws<ArgumentException>(() => MagWorldRegistryConfig.Parse(Config(257).ToJsonString()), "A registry row above 256 was accepted");

        // Options: humans stay within 64; world capacity covers the human maximum and stays within 256.
        var address = IPAddress.Parse("192.0.2.9");
        var options = new LocalDmeHostOptions(20231, 0, 1, 64, new[] { 0, 0 }, null, address, 12073, "large", worldCapacity: 256);
        Check(options.WorldCapacity == 256 && options.StartupPolicy.MaximumPlayers == 64, "Registry world capacity and human maximum were mixed");
        Check(new LocalDmeHostOptions(20231, 0, 1, 8, new[] { 0, 0 }, null, address, 12073, "legacy").WorldCapacity == 8,
            "A legacy host's world capacity was not its maximum");
        Throws<ArgumentException>(() => new LocalDmeHostOptions(20231, 0, 1, 64, new[] { 0, 0 }, null, address, 12073, "large", worldCapacity: 32),
            "A world capacity below the human maximum was accepted");
        Throws<ArgumentException>(() => new LocalDmeHostOptions(20231, 0, 1, 64, new[] { 0, 0 }, null, address, 12073, "large", worldCapacity: 257),
            "A world capacity above 256 was accepted");
        Throws<ArgumentException>(() => new LocalDmeHostOptions(20231, 0, 1, 65, new[] { 0, 0 }, null, address, 12073, "large", worldCapacity: 128),
            "More than 64 humans were accepted");
        // configure_match asks for the human cap, never the world size.
        var startup = new MagMatchStartupPolicy(1, 64, new[] { 0, 0 }).WithProfileMaximum(Large().HumanCapacity);
        Check(startup.MaximumPlayers == 32, "configure_match would carry world size as the human maximum");
    }
}
