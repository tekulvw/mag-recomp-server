using System.Text.Json;
using System.Text.Json.Nodes;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
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

    static RT_MSG_CLIENT_APP_TO_PLUGIN ArmoryEnvelope(byte[] payload, byte pluginId = 0)
    {
        using var buffer = new MemoryStream();
        using (var writer = new BinaryWriter(buffer, System.Text.Encoding.UTF8, true))
        {
            writer.Write((ushort)payload.Length);
            writer.Write(pluginId);
            writer.Write(payload);
        }
        buffer.Position = 0;
        using var reader = new MessageReader(buffer);
        var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN();
        envelope.Deserialize(reader);
        return envelope;
    }

    public static void DeleteThroughMls()
    {
        const int app = 20231;
        int previousApp = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = app;
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-delete-kit-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var now = DateTime.UtcNow;
            var path = Path.Combine(directory, "characters.json");
            var original = Fixture();
            File.WriteAllText(path, original.ToJsonString());
            var store = new LocalCharacterStore(path) { LoadoutCatalog = Catalog(directory) };
            var registry = new MagLoginRegistry();
            var ticket = registry.IssueForAccount("alpha", 1, 1, now);
            var session = new MagMlsSession { CharacterStore = store };
            Check(session.TryBind(new RT_MSG_CLIENT_CONNECT_TCP
            {
                AppId = app, TargetWorldId = 1, AccessToken = ticket.AccessToken, SessionKey = ticket.SessionKey
            }, registry, "synthetic-delete-kit", now), "Authenticated MLS session failed to bind");
            Check(session.Accept(now) && session.CompleteTransport(now), "MLS transport failed to become ready");
            session.HandleMany(ArmoryEnvelope(MagMessageCodec.SerializeForApplication(new MagHello(), app)), now);
            session.HandleMany(ArmoryEnvelope(MagMessageCodec.SerializeForApplication(
                new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, app)), now);
            Check(session.State == MagMlsSession.SessionState.Ready, "MLS protocol handshake did not finish");
            byte[] wire = { 0x02, 0x21, 0 };
            byte[] before = File.ReadAllBytes(path);
            int callbacks = 0;
            session.LoadoutCommitted = (account, character) => {
                Check(account == "alpha" && character == 1, "Delete notified the wrong owner");
                Check(store.ReadForAccount(account).Single().LoadoutLayoutRevision == 1, "Delete notified before its durable write");
                callbacks++;
            };
            var replies = session.HandleMany(ArmoryEnvelope(wire), now);
            Check(replies.Count == 1 && replies[0].Payload.SequenceEqual(new byte[] { 0x02, 0x22, 0 }) &&
                session.StorageStatus == "loadout-layout-capability-unavailable" && callbacks == 0 &&
                File.ReadAllBytes(path).SequenceEqual(before), "Older worker did not prevent a delete before mutation");
            Check(session.HandleMany(ArmoryEnvelope(wire, 1), now).Count == 0, "Wrong plugin entered the Armory delete route");
            session.CanDeleteLoadout = () => true;
            session.CharacterStore = new LocalCharacterStore(path, new FailedCommitter()) { LoadoutCatalog = store.LoadoutCatalog };
            replies = session.HandleMany(ArmoryEnvelope(wire), now);
            Check(replies.Count == 1 && replies[0].Payload.SequenceEqual(new byte[] { 0x02, 0x22, 0 }) &&
                session.StorageStatus == "loadout-storage-error-no-success" && callbacks == 0 &&
                File.ReadAllBytes(path).SequenceEqual(before), "Failed durable delete acknowledged success or changed the store");
            session.CharacterStore = store;
            replies = session.HandleMany(ArmoryEnvelope(wire), now);
            Check(replies.Count == 1 && replies[0].PluginId == 0 && replies[0].Payload.SequenceEqual(new byte[] { 0x02, 0x22, 0x80 }),
                "Authenticated retail delete did not return the native success response");
            var current = new LocalCharacterStore(path) { LoadoutCatalog = store.LoadoutCatalog }.ReadForAccount("alpha").Single();
            Check(current.LastUsedLoadout == 0 && current.Loadouts.Select(row => row.UpdateId).SequenceEqual(new byte[] { 2, 3, 4, 5, 0 }) &&
                current.Loadouts[1].Items[0] == 2 && current.Loadouts[4].Items.All(id => id == -1) &&
                current.LoadoutLayoutRevision == 1 && callbacks == 1, "Durable saved rows or preference were not compacted");
            var expected = original.DeepClone();
            var owner = expected["characters"]![0]!;
            owner.AsObject().Remove("lastUsedLoadout");
            owner["loadoutLayoutRevision"] = 1;
            owner["loadouts"]!.AsArray().RemoveAt(0);
            owner["loadouts"]!.AsArray().Add(JsonSerializer.SerializeToNode(new {
                updateId = 0, items = Enumerable.Repeat(-1, 22)
            }));
            Check(JsonNode.DeepEquals(expected, JsonNode.Parse(File.ReadAllBytes(path))), "Delete changed unrelated saved rows, accounts, or progression");
            var snapshot = GameplayLoadoutAdmission.Project(current, current.ToRecord(), store.LoadoutCatalog, supportsLayoutChanges: true);
            Check(snapshot.LayoutRevision == 1 && snapshot.SelectedIndex == 0 && snapshot.Inventories[1][0] == 2 && snapshot.Inventories[4][0] == null &&
                JsonSerializer.Serialize(snapshot).Contains("\"layout_revision\":1"), "Compaction was lost in the future-kit projection");
            var legacy = GameplayLoadoutAdmission.Project(current, current.ToRecord(), store.LoadoutCatalog);
            Check(legacy.LayoutRevision == 0 && !JsonSerializer.Serialize(legacy).Contains("layout_revision"), "Old peer projection contains the optional layout field");

            byte[] committed = File.ReadAllBytes(path);
            var settlement = new GameplayLoadoutSelectionSettlement(new object());
            bool Owns(GameplayLoadoutSelection selection) => selection.MatchesOwner("synthetic-reservation", 7, 1, "alpha", 1);
            CharacterCreationOutcome Save(GameplayLoadoutSelection selection) => store.SaveLastUsedLoadoutForAccount(
                selection.AccountId, selection.CharacterId, selection.InventoryIndex, selection.LayoutRevision);
            const string queuedOldSelection = """
                {"v":2,"request_id":"test","ok":true,"error":null,"messages":[],"loadout_selections":[{"selection_id":"old-choice","reservation_id":"synthetic-reservation","world_id":7,"player_id":1,"account_id":"alpha","character_id":1,"revision":1,"inventory_index":1}]}
                """;
            Check(GameplayBridgeResponseCodec.TryParse(System.Text.Encoding.UTF8.GetBytes(queuedOldSelection), "test", true, out var reply) &&
                settlement.Settle(reply.LoadoutSelections.Single(), now, Owns, Save) == "rejected" &&
                settlement.Settle(reply.LoadoutSelections.Single(), now, Owns, Save) == "rejected" &&
                store.DeleteLoadoutForAccount("alpha", 2, 0).Reason == "character-not-owned" &&
                store.DeleteLoadoutForAccount("alpha", 1, 4).Reason == "loadout-delete-row-empty" &&
                committed.SequenceEqual(File.ReadAllBytes(path)), "Queued old preference, empty tail, or foreign delete mutated the compacted store");
            var fresh = JsonNode.Parse(queuedOldSelection);
            var choice = fresh!["loadout_selections"]![0]!;
            choice["selection_id"] = "new-choice";
            choice["revision"] = 2;
            choice["layout_revision"] = 1;
            Check(GameplayBridgeResponseCodec.TryParse(JsonSerializer.SerializeToUtf8Bytes(fresh), "test", true, out var newReply) &&
                settlement.Settle(newReply.LoadoutSelections.Single(), now, Owns, Save) == "persisted" &&
                store.ReadForAccount("alpha").Single().LastUsedLoadout == 1, "A same-layout live preference could not select a surviving row");
            Check(store.DeleteLoadoutForAccount("alpha", 1, 1).Result == MagNetResult.Success &&
                store.ReadForAccount("alpha").Single().LastUsedLoadout == 0 &&
                store.ReadForAccount("alpha").Single().LoadoutLayoutRevision == 2, "Deleting the preferred row did not choose the first playable kit");

            var defaults = Fixture();
            defaults["characters"]![0]!.AsObject().Remove("loadouts");
            File.WriteAllText(path, defaults.ToJsonString());
            Check(store.DeleteLoadoutForAccount("alpha", 1, 0).Result == MagNetResult.Success &&
                store.ReadForAccount("alpha").Single().Loadouts[3].Items[0] == 1 &&
                store.ReadForAccount("alpha").Single().Loadouts[4].Items[0] == -1, "Deleting before the first save lost the authored starter rows");
            var finalKit = Fixture();
            for (int index = 1; index < 5; index++) finalKit["characters"]![0]!["loadouts"]![index]!["items"]![0] = -1;
            File.WriteAllText(path, finalKit.ToJsonString());
            before = File.ReadAllBytes(path);
            Check(store.DeleteLoadoutForAccount("alpha", 1, 0).Reason == "primary-loadout-required" &&
                File.ReadAllBytes(path).SequenceEqual(before), "Last playable kit deletion left an invalid record");
        }
        finally
        {
            Server.Medius.Program.Settings.MagApplicationId = previousApp;
            Directory.Delete(directory, true);
        }
    }

    // Invented retail catalogs: faction-local wire ids (1024 + n) whose ownership index is n.
    static (MagLoadoutCatalog Armory, MagSkillCatalog Skills) RetailSkillCatalogs(string directory)
    {
        object Item(int bit, string name, int? ammo) => new {
            wire_id = 1024 + bit, name, authored_ref = "synthetic:" + bit, available_bit = bit, factions = new[] { 0, 1, 2 }, primary_ammo = ammo };
        var starter = Enumerable.Repeat<short?>(null, 22).ToArray();
        starter[0] = 1025;
        string armory = Path.Combine(directory, "retail-armory.json");
        File.WriteAllText(armory, JsonSerializer.Serialize(new {
            schema_version = 1, application_id = 20231,
            source_sha256 = new { synthetic = new string('b', 64) },
            items = new[] {
                Item(1, "Example starter rifle", 7), Item(2, "Example bought rifle", 9), Item(4, "Example bought sight", null),
                Item(5, "Example cap", null), Item(6, "Example reserve rifle", 5), Item(7, "Example locked sight", null)
            },
            factions = Enumerable.Range(0, 3).Select(faction => new {
                faction, available_item_bits = faction == 0 ? new[] { 1 } : Array.Empty<int>(), skill_ids = new[] { 0 },
                loadouts = Enumerable.Range(0, 5).Select(_ => new { update_id = 1, items = starter })
            })
        }));
        object Faction(bool available, params string[] tokens) => new {
            Available = available,
            Equipment = tokens.Select((token, index) => (token, index)).ToDictionary(pair => "Reward" + pair.index, pair => new { element = pair.token })
        };
        object Skill(int cost, int tier, params string[] tokens) => new { MagCharacterSkill = new {
            Cost = cost, Tier = tier, Category = "kSkillCategoryExample",
            Green = Faction(true, tokens), Black = Faction(false), Red = Faction(false)
        } };
        object Thresholds() => new { Tier2_PrereqPoints = 0, Tier3_PrereqPoints = 0, Tier4_PrereqPoints = 0, Tier5_PrereqPoints = 0 };
        string skills = Path.Combine(directory, "retail-skills.json");
        File.WriteAllText(skills, JsonSerializer.Serialize(new {
            schema_version = 2, application_id = 20231,
            equipment_token_item_ids = new Dictionary<string, int[]> {
                ["starter_rifle"] = new[] { 1 }, ["bought_rifle"] = new[] { 2 }, ["bought_sight"] = new[] { 4 }, ["locked_sight"] = new[] { 7 }
            },
            table = new {
                Tiers = new { ExampleTier = new { MagCharacterSkillTier = new {
                    Category = "kSkillCategoryExample", Green = Thresholds(), Black = Thresholds(), Red = Thresholds() } } },
                Skills = new Dictionary<string, object> {
                    ["MagCharacterSkill0"] = Skill(0, 0, "starter_rifle"),
                    ["MagCharacterSkill3"] = Skill(2, 1, "bought_rifle", "bought_sight"),
                    ["MagCharacterSkill5"] = Skill(1, 1, "locked_sight")
                }
            }
        }));
        return (MagLoadoutCatalog.LoadRetail(armory), MagSkillCatalog.LoadForApplication(skills, 20231));
    }

    public static void RetailSkillPurchaseOwnsItsEquipment()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-skill-kit-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var (armory, skills) = RetailSkillCatalogs(directory);
            string path = Path.Combine(directory, "characters.json");
            var fixture = Fixture();
            var owner = fixture["characters"]![0]!.AsObject();
            owner.Remove("loadouts"); owner.Remove("lastUsedLoadout"); owner.Remove("ownedSkills");
            owner["unlockedItems"] = JsonSerializer.SerializeToNode(new uint[24]);
            File.WriteAllText(path, fixture.ToJsonString());
            var store = new LocalCharacterStore(path) { LoadoutCatalog = armory, SkillCatalog = skills };
            MagLoadout Kit(params (int Slot, short Item)[] items)
            {
                var kit = new MagLoadout { UpdateId = 2 };
                foreach (var (slot, item) in items) kit.Items[slot] = item;
                return kit;
            }
            void Refused(LocalCharacterStore target, MagLoadout kit, string reason)
            {
                byte[] before = File.ReadAllBytes(path);
                var outcome = target.SaveLoadoutForAccount("alpha", 1, 1, kit);
                Check(outcome.Result != MagNetResult.Success && outcome.Reason == reason, "Wrong loadout refusal: " + outcome.Reason);
                Check(before.SequenceEqual(File.ReadAllBytes(path)), "Refused loadout rewrote the store");
            }
            var bought = Kit((0, 1026), (10, 1029), (14, 1028));
            Refused(store, bought, "loadout-item-not-owned:slot=0:item=1026");
            Check(store.PurchaseSkillForAccount("alpha", 1, 3).Reason == "skill-purchase-committed", "Synthetic skill purchase failed");
            var unlocked = store.ReadForAccount("alpha").Single().UnlockedItems;
            Check(unlocked[0] == ((1u << 2) | (1u << 4)), "Purchase did not set the equipment ownership indices");
            Check(store.SaveLoadoutForAccount("alpha", 1, 1, bought).Result == MagNetResult.Success,
                "A kit using purchased equipment and never-awarded apparel was refused");
            var saved = store.ReadForAccount("alpha").Single().Loadouts[1].Items;
            Check(saved[0] == 1026 && saved[10] == 1029 && saved[14] == 1028, "Purchased kit was not saved");
            Refused(store, Kit((0, 1026), (14, 1028), (15, 1031)), "loadout-item-not-owned:slot=15:item=1031");
            Refused(store, Kit((0, 1026), (1, 1030)), "loadout-item-not-owned:slot=1:item=1030");
            var withoutSkills = new LocalCharacterStore(path) { LoadoutCatalog = armory };
            Refused(withoutSkills, Kit((0, 1026), (10, 1029)), "loadout-item-not-owned:slot=10:item=1029");
        }
        finally { Directory.Delete(directory, true); }
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
            Refused(doc => doc["characters"]![0]!["unlockedItems"]![0] = 0, "alpha", 1, 2, "loadout-item-not-owned:slot=0:item=2");
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
