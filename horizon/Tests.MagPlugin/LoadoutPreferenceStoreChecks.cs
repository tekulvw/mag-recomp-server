using System.Text.Json;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// Independently invented account/catalog data; no game files or real stores.
internal static class LoadoutPreferenceStoreChecks
{
    static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    sealed class FailedCommitter : ICharacterStoreCommitter
    {
        public void WriteAtomically(string path, byte[] contents) => throw new IOException("synthetic write failure");
    }

    static MagLoadoutCatalog Catalog(string directory)
    {
        string path = Path.Combine(directory, "armory.json");
        var starter = Enumerable.Repeat<short?>(null, 22).ToArray();
        starter[0] = 1;
        File.WriteAllText(path, JsonSerializer.Serialize(new {
            schema_version = 1, application_id = 20231,
            source_sha256 = new { synthetic = new string('a', 64) },
            items = new[] {
                new { wire_id = 1, name = "Example starter", authored_ref = "synthetic:1", available_bit = 1, factions = new[] { 0, 1, 2 }, primary_ammo = 7 },
                new { wire_id = 2, name = "Example purchase", authored_ref = "synthetic:2", available_bit = 2, factions = new[] { 0, 1, 2 }, primary_ammo = 9 },
                new { wire_id = 3, name = "Other faction", authored_ref = "synthetic:3", available_bit = 3, factions = new[] { 1 }, primary_ammo = 11 }
            },
            factions = Enumerable.Range(0, 3).Select(faction => new {
                faction, available_item_bits = Array.Empty<int>(), skill_ids = Array.Empty<int>(),
                loadouts = Enumerable.Range(0, 5).Select(_ => new { update_id = 1, items = starter })
            })
        }));
        return MagLoadoutCatalog.LoadRetail(path);
    }

    static JsonObject Fixture()
    {
        object Character(string account, uint id, string name) => new {
            account, id, name, faction = 0, headId = 2, voiceId = 3, flags = 1,
            universeId = 26, npId = new byte[36], lastUsedLoadout = 1,
            skillPoints = 7, ownedSkills = new uint[] { 5, 0, 0, 0, 0 },
            unlockedItems = Enumerable.Range(0, 24).Select(i => i == 0 ? 4u : 0u).ToArray(),
            experience = 13, level = 2,
            leadership = new { leadershipLevel = 3, nominations = 2, matchesSinceLastCommand = 4 },
            loadouts = Enumerable.Range(0, 5).Select(index => new {
                updateId = index + 1,
                items = Enumerable.Range(0, 22).Select(slot => slot == 0 ? (index == 2 ? 2 : 1) : -1).ToArray(),
                extraKit = new { marker = index, nested = new object[] { true, "retain" } }
            }),
            extraCharacter = new { marker = account, nested = new[] { 4, 7 } }
        };
        return JsonNode.Parse(JsonSerializer.Serialize(new {
            schemaVersion = 3, account = "player", displayName = "Player",
            characters = new[] { Character("alpha", 1, "Alpha"), Character("beta", 2, "Beta") },
            rewardReceipts = new Dictionary<string, object> {
                ["receipt-a"] = new {
                    account = "alpha", characterId = 1, matchKey = "match-a", kind = "enemy_killed", units = 1,
                    snapshot = new {
                        result = MagNetResult.Success, reason = "reward-committed", duplicate = false,
                        experience = 3, level = 1, previousLevel = 1, skillPoints = 7,
                        experienceAwarded = 3, matchExperience = 3, matchFraction = 0.25f,
                        extraSnapshot = new { marker = "retain" }
                    },
                    extraReceipt = new { marker = "retain" }
                }
            },
            matchRewardProgress = new Dictionary<string, object> {
                ["alpha:1:match-a"] = new {
                    account = "alpha", characterId = 1, matchKey = "match-a", experience = 3,
                    fraction = 0.25f, finalized = false, extraProgress = new { marker = "retain" }
                }
            },
            extraDocument = new { marker = "retain", nested = new object[] { false, 7, "value" } }
        }))!.AsObject();
    }

    public static void Run()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-kit-store-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string path = Path.Combine(directory, "characters.json");
            var catalog = Catalog(directory);
            var original = Fixture();
            File.WriteAllText(path, original.ToJsonString());
            var store = new LocalCharacterStore(path) { LoadoutCatalog = catalog };
            Check(store.ReadAll().Count == 2, "Synthetic fixture rejected before save");
            Check(store.SaveLastUsedLoadoutForAccount("alpha", 1, 2).Result == MagNetResult.Success, "Owned selected kit did not commit");
            var expected = original.DeepClone();
            expected["characters"]![0]!["lastUsedLoadout"] = 2;
            Check(JsonNode.DeepEquals(expected, JsonNode.Parse(File.ReadAllBytes(path))),
                "Preference save changed other accounts, progression, leadership, kits, reward ledgers or accepted extension fields");
            Check(new LocalCharacterStore(path).ReadForAccount("alpha").Single().LastUsedLoadout == 2,
                "Selected kit was not retained across store restart");

            void Refused(Action<JsonObject> mutate, string account, uint character, byte index, string reason)
            {
                var fixture = Fixture(); mutate(fixture);
                File.WriteAllText(path, fixture.ToJsonString());
                byte[] before = File.ReadAllBytes(path);
                var outcome = store.SaveLastUsedLoadoutForAccount(account, character, index);
                Check(outcome.Result != MagNetResult.Success && outcome.Reason == reason, "Wrong selected-kit refusal: " + outcome.Reason);
                Check(before.SequenceEqual(File.ReadAllBytes(path)), "Refused selection rewrote the store");
            }
            Refused(_ => { }, "alpha", 2, 2, "character-not-owned");
            Refused(_ => { }, "alpha", 1, 5, "invalid-last-used-loadout");
            Refused(doc => doc["characters"]![0]!["unlockedItems"]![0] = 0, "alpha", 1, 2, "loadout-item-not-owned");
            Refused(doc => doc["characters"]![0]!["loadouts"]![2]!["items"]![0] = -1, "alpha", 1, 2, "selected-loadout-primary-required");
            Refused(doc => doc["characters"]![0]!["loadouts"]![2]!["items"]![0] = 3, "alpha", 1, 2, "invalid-faction-loadout");

            var starter = Fixture();
            starter["characters"]![0]!["unlockedItems"]![0] = 0;
            File.WriteAllText(path, starter.ToJsonString());
            Check(store.SaveLastUsedLoadoutForAccount("alpha", 1, 0).Result == MagNetResult.Success,
                "Authored starter kit exception was lost");

            File.WriteAllText(path, original.ToJsonString());
            byte[] saved = File.ReadAllBytes(path);
            var failing = new LocalCharacterStore(path, new FailedCommitter()) { LoadoutCatalog = catalog };
            try { failing.SaveLastUsedLoadoutForAccount("alpha", 1, 2); throw new Exception("Failed durable save returned success"); }
            catch (IOException) { }
            Check(saved.SequenceEqual(File.ReadAllBytes(path)), "Failed save changed the store");

            var strictLeadership = Fixture();
            strictLeadership["characters"]![0]!["leadership"]!["unknown"] = 1;
            File.WriteAllText(path, strictLeadership.ToJsonString());
            try { store.ReadAll(); throw new Exception("Strict leadership schema became permissive"); }
            catch (InvalidDataException) { }
        }
        finally { Directory.Delete(directory, true); }
    }
}
