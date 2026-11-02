using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using RT.Models.MAG;

namespace Server.Medius
{
    public sealed class StoredLoadout
    {
        [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
        [JsonRequired]
        public byte UpdateId { get; set; }
        [JsonRequired]
        public short[] Items { get; set; }
        public MagLoadout ToLoadout() => new MagLoadout { UpdateId = UpdateId, Items = (short[])Items.Clone() };
        public static StoredLoadout FromLoadout(MagLoadout loadout) => new StoredLoadout { UpdateId = loadout.UpdateId, Items = (short[])loadout.Items.Clone() };
    }
    /// <summary>Authoritative saved native byte fields; absence preserves legacy policy fallback.</summary>
    [JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
    public sealed class StoredLeadershipRecord
    {
        [JsonRequired] public byte LeadershipLevel { get; set; }
        [JsonRequired] public byte Nominations { get; set; }
        [JsonRequired] public byte MatchesSinceLastCommand { get; set; }
    }
    public sealed class StoredCharacter
    {
        [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
        public string Account { get; set; }
        [JsonRequired]
        public uint Id { get; set; }
        [JsonRequired]
        public string Name { get; set; }
        [JsonRequired]
        public byte Faction { get; set; }
        [JsonRequired]
        public byte HeadId { get; set; }
        [JsonRequired]
        public byte VoiceId { get; set; }
        // Older stores omit this field and remain incomplete; XP/level never imply completion.
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public byte Flags { get; set; }
        [JsonRequired]
        public uint UniverseId { get; set; }
        [JsonRequired]
        public byte[] NpId { get; set; }
        // Null preserves legacy files and the existing native default-import path until the first save.
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public StoredLoadout[] Loadouts { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public byte LastUsedLoadout { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public byte? SkillPoints { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public uint[] OwnedSkills { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public uint[] UnlockedItems { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public int? Experience { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public byte? Level { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public StoredLeadershipRecord Leadership { get; set; }
        [JsonIgnore]
        internal MagProgressionCatalog RecordProgressionCatalog { get; set; }
        [JsonIgnore]
        internal byte RecordInitialSkillPoints { get; set; }
        [JsonIgnore]
        internal MagLoadoutCatalog RecordCatalog { get; set; }
        public MagCharacterRecord ToRecord()
        {
            var grants = RecordCatalog?.GrantsForFaction(Faction);
            return new MagCharacterRecord
            {
                AccountId = Id, Name = Name, Faction = Faction, HeadId = HeadId, VoiceId = VoiceId, Flags = Flags,
                UniverseId = UniverseId, NpId = (byte[])NpId.Clone(), LastUsedLoadout = LastUsedLoadout,
                SkillPoints = SkillPoints ?? RecordInitialSkillPoints, Experience = (uint)(Experience ?? 0), Level = Level ?? 1,
                LeadershipLevel = Leadership?.LeadershipLevel ?? 1,
                LeadershipNominations = Leadership?.Nominations ?? 0,
                MatchesSinceLastCommand = Leadership?.MatchesSinceLastCommand ?? 0,
                AvailableItems = MergeMask(grants?.AvailableItems, UnlockedItems, 24), Skills = MergeMask(grants?.Skills, OwnedSkills, 5),
                Loadouts = Loadouts?.Select(row => row.ToLoadout()).ToArray() ?? RecordCatalog?.DefaultsForFaction(Faction) ??
                    Enumerable.Range(0, 5).Select(_ => new MagLoadout()).ToArray()
            };
        }
        private static uint[] MergeMask(uint[] starter, uint[] purchased, int length)
        {
            var result = new uint[length];
            for (int index = 0; index < length; index++) result[index] = (starter?[index] ?? 0) | (purchased?[index] ?? 0);
            return result;
        }
        public MagCharacterSummary ToSummary() => new MagCharacterSummary
        {
            AccountId = Id, Name = Name, Faction = Faction, HeadId = HeadId, VoiceId = VoiceId, UniverseId = UniverseId,
            Level = Level ?? 1, Loadout = Loadouts == null ? new MagLoadout() : Loadouts[LastUsedLoadout].ToLoadout()
        };
    }
    public sealed class CharacterCreationOutcome
    {
        public MagNetResult Result { get; set; }
        public string Reason { get; set; }
    }
    public sealed class TrainingCompletionOutcome
    {
        public MagNetResult Result { get; set; }
        public string Reason { get; set; }
        public MagCharacterRecord Record { get; set; }
        public bool NewlyCompleted { get; set; }
    }
    public sealed class ProgressionRewardOutcome
    {
        [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
        [JsonRequired] public MagNetResult Result { get; set; }
        [JsonRequired] public string Reason { get; set; }
        public bool Duplicate { get; set; }
        [JsonRequired] public int Experience { get; set; }
        [JsonRequired] public byte Level { get; set; }
        [JsonRequired] public byte PreviousLevel { get; set; }
        [JsonRequired] public byte SkillPoints { get; set; }
        [JsonRequired] public int ExperienceAwarded { get; set; }
        [JsonRequired] public int MatchExperience { get; set; }
        [JsonRequired] public float MatchFraction { get; set; }
        internal ProgressionRewardOutcome Copy(bool duplicate) => new ProgressionRewardOutcome {
            Result = Result, Reason = Reason, Duplicate = duplicate, Experience = Experience, Level = Level,
            PreviousLevel = PreviousLevel, SkillPoints = SkillPoints, ExperienceAwarded = ExperienceAwarded,
            MatchExperience = MatchExperience, MatchFraction = MatchFraction };
    }
    public interface ICharacterStoreCommitter { void WriteAtomically(string path, byte[] contents); }

    /// <summary>Linux local-filesystem commit: flush file, atomic rename, then sync directory.
    /// A reported error after rename is an uncertain commit; never delete the destination.</summary>
    public sealed class AtomicCharacterStoreCommitter : ICharacterStoreCommitter
    {
        [DllImport("libc", SetLastError = true)] private static extern IntPtr opendir(string path);
        [DllImport("libc", SetLastError = true)] private static extern int dirfd(IntPtr directory);
        [DllImport("libc", SetLastError = true)] private static extern int fsync(int fd);
        [DllImport("libc", SetLastError = true)] private static extern int closedir(IntPtr directory);
        public void WriteAtomically(string path, byte[] contents)
        {
            if (!RuntimeInformation.IsOSPlatform(OSPlatform.Linux)) throw new IOException("Durable store commits require the Linux lab runtime.");
            string directory = Path.GetDirectoryName(path);
            string temporary = Path.Combine(directory, ".mag-character-" + Guid.NewGuid().ToString("N") + ".tmp");
            try
            {
                using (var file = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                { file.Write(contents, 0, contents.Length); file.Flush(true); }
                File.Move(temporary, path, true);
                IntPtr handle = opendir(directory); // libc avoids architecture-specific O_DIRECTORY constants.
                if (handle == IntPtr.Zero) throw new IOException("Cannot open character-store directory for sync.");
                try { int fd = dirfd(handle); if (fd < 0 || fsync(fd) != 0) throw new IOException("Character-store directory sync failed; commit uncertain."); }
                finally { closedir(handle); }
            }
            finally { if (File.Exists(temporary)) File.Delete(temporary); }
        }
    }
    public sealed class LocalCharacterStore
    {
        private sealed class RewardReceipt
        {
            [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
            [JsonRequired] public string Account { get; set; }
            [JsonRequired] public uint CharacterId { get; set; }
            [JsonRequired] public string MatchKey { get; set; }
            [JsonRequired] public string Kind { get; set; }
            [JsonRequired] public int Units { get; set; }
            [JsonRequired] public ProgressionRewardOutcome Snapshot { get; set; }
        }
        private sealed class MatchRewardProgress
        {
            [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
            [JsonRequired] public string Account { get; set; }
            [JsonRequired] public uint CharacterId { get; set; }
            [JsonRequired] public string MatchKey { get; set; }
            [JsonRequired] public int Experience { get; set; }
            [JsonRequired] public float Fraction { get; set; }
            // Older schema-three matches have no end receipt and remain open.
            public bool Finalized { get; set; }
        }
        private sealed class Document
        {
            [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
            [JsonRequired]
            public int SchemaVersion { get; set; }
            [JsonRequired]
            public string Account { get; set; }
            [JsonRequired]
            public string DisplayName { get; set; }
            [JsonRequired]
            public List<StoredCharacter> Characters { get; set; }
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
            public Dictionary<string, RewardReceipt> RewardReceipts { get; set; }
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
            public Dictionary<string, MatchRewardProgress> MatchRewardProgress { get; set; }
        }
        private static readonly JsonSerializerOptions JsonOptions = new JsonSerializerOptions
        { PropertyNamingPolicy = JsonNamingPolicy.CamelCase, WriteIndented = true };
        private const int MaximumStoreBytes = 1048576; // 128 accounts with five complete inventories each.
        private readonly string _path;
        public MagLoadoutCatalog LoadoutCatalog { get; set; }
        public MagSkillCatalog SkillCatalog { get; set; }
        public MagProgressionCatalog ProgressionCatalog { get; set; }
        public MagTrainingCompletionCatalog TrainingCompletionCatalog { get; set; }
        // Local receipt bound: reject new awards when full; never evict replay protection.
        public const int MaximumRewardReceipts = 4096;
        // Explicit local development policy, supplied by the caller; authored starter data grants no blanket unlock.
        public byte InitialSkillPoints { get; set; }
        private readonly ICharacterStoreCommitter _committer;
        public LocalCharacterStore(string path) : this(path, new AtomicCharacterStoreCommitter()) { }
        public LocalCharacterStore(string path, ICharacterStoreCommitter committer)
        { _path = path; _committer = committer ?? throw new ArgumentNullException(nameof(committer)); }
        private string ValidPath()
        {
            if (string.IsNullOrWhiteSpace(_path) || !Path.IsPathFullyQualified(_path)) throw new IOException("An absolute character-store path must be configured.");
            return Path.GetFullPath(_path);
        }
        // Readers do not create directories or lock files; usable through a read-only mount.
        public IReadOnlyList<StoredCharacter> ReadAll() => ReadDocument().Characters;
        private static Document EmptyDocument() => new Document { SchemaVersion = 2, Account = "player", DisplayName = "Player", Characters = new List<StoredCharacter>() };
        private Document ReadDocument()
        {
            string path = ValidPath();
            byte[] bytes;
            try
            {
                using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
                if (stream.Length > MaximumStoreBytes) throw new InvalidDataException("Character store exceeds local bound.");
                using var memory = new MemoryStream(); stream.CopyTo(memory); bytes = memory.ToArray();
                if (bytes.Length > MaximumStoreBytes) throw new InvalidDataException("Character store exceeds local bound.");
            }
            catch (FileNotFoundException) { return EmptyDocument(); }
            catch (DirectoryNotFoundException) { return EmptyDocument(); }
            Document document;
            try { document = JsonSerializer.Deserialize<Document>(bytes, JsonOptions); }
            catch (JsonException e) { throw new InvalidDataException("Invalid character-store JSON.", e); }
            if (document == null || (document.SchemaVersion != 1 && document.SchemaVersion != 2 && document.SchemaVersion != 3) || document.Account != "player" || document.DisplayName != "Player" ||
                document.Characters == null || document.Characters.Count > (document.SchemaVersion == 1 ? 1 : 128) || document.Characters.Any(c => !Valid(c, document.SchemaVersion)) ||
                document.Characters.Select(c => c.Id).Distinct().Count() != document.Characters.Count ||
                document.Characters.Select(c => c.Name.ToLowerInvariant()).Distinct().Count() != document.Characters.Count ||
                (document.SchemaVersion >= 2 && document.Characters.Select(c => c.Account).Distinct().Count() != document.Characters.Count))
                throw new InvalidDataException("Character-store schema or local account policy invalid.");
            if (document.SchemaVersion == 1) foreach (var c in document.Characters) c.Account = "player";
            ValidateRewardLedger(document);
            // Runtime-derived starter masks/defaults enrich replies without rewriting the character JSON.
            foreach (var character in document.Characters)
            {
                character.RecordCatalog = LoadoutCatalog;
                character.RecordInitialSkillPoints = InitialSkillPoints;
                character.RecordProgressionCatalog = ProgressionCatalog;
                if (ProgressionCatalog != null && (character.Level ?? 1) != ProgressionCatalog.LevelForExperience(character.Experience ?? 0))
                    throw new InvalidDataException("Persisted XP/level does not match the progression catalog.");
            }
            return document;
        }
        private static bool ValidName(string name) => !string.IsNullOrWhiteSpace(name) && name.Length <= 32 && name.All(c => c >= 33 && c <= 126);
        private static bool Valid(StoredCharacter c, int schema) => c != null && c.Id > 0 && ValidName(c.Name) && c.Faction <= 2 &&
            (c.Experience == null || c.Experience >= 0) && (c.Level == null || c.Level >= 1) &&
            (c.OwnedSkills == null || c.OwnedSkills.Length == 5) && (c.UnlockedItems == null || c.UnlockedItems.Length == 24) &&
            c.LastUsedLoadout < 5 && (c.Loadouts == null || (c.Loadouts.Length == 5 && c.Loadouts.All(ValidLoadout))) &&
            c.UniverseId == 26 && c.NpId != null && c.NpId.Length == 36 && c.NpId.All(b => b == 0) &&
            (schema == 1 ? c.Id == 1 && c.Name == "Player" : MagDevelopmentAccountPolicy.ValidAccount(c.Account));
        private static bool ValidLoadout(StoredLoadout row) => row?.Items != null && row.Items.Length == 22 && row.Items.All(id => id >= -1 && id < 4096);
        public IReadOnlyList<StoredCharacter> ReadForAccount(string account) => MagDevelopmentAccountPolicy.ValidAccount(account)
            ? ReadAll().Where(c => c.Account == account).ToArray() : Array.Empty<StoredCharacter>();
        // Compatibility entry point for older local lab tools; live sessions use the account-bound API.
        public CharacterCreationOutcome Create(MagCreateCharacterRequest request) => request?.AccountName == "Player"
            ? CreateForAccount(request, "player") : Outcome(MagNetResult.NameInvalid, "invalid-legacy-name");
        public CharacterCreationOutcome CreateForAccount(MagCreateCharacterRequest request, string account)
        {
            if (request == null || !MagDevelopmentAccountPolicy.HasZeroRsa(request) || !ValidName(request.AccountName) || !MagDevelopmentAccountPolicy.ValidAccount(account))
                return Outcome(MagNetResult.NameInvalid, "invalid-local-name-or-header");
            if (request.Faction > 2 || request.UniverseId != 26 || request.NpId == null || request.NpId.Length != 36 || request.NpId.Any(b => b != 0))
                return Outcome(MagNetResult.Error, "unsupported-local-profile-fields");
            string path = ValidPath(); Directory.CreateDirectory(Path.GetDirectoryName(path));
            using var lease = AcquireLock(path + ".lock");
            var all = ReadAll().ToList();
            var existing = all.SingleOrDefault(c => c.Account == account);
            if (existing != null)
            {
                bool same = existing.Name == request.AccountName && existing.Faction == request.Faction && existing.HeadId == request.HeadId &&
                    existing.VoiceId == request.VoiceId && existing.UniverseId == request.UniverseId && existing.NpId.SequenceEqual(request.NpId);
                if (!same) return Outcome(MagNetResult.NameTaken, "local-account-slot-occupied");
            }
            else
            {
                if (all.Any(c => string.Equals(c.Name, request.AccountName, StringComparison.OrdinalIgnoreCase)))
                    return Outcome(MagNetResult.NameTaken, "character-name-taken");
                if (all.Count >= 128) return Outcome(MagNetResult.Error, "local-store-full");
                uint next = all.Count == 0 ? 1u : checked(all.Max(c => c.Id) + 1u);
                var created = new StoredCharacter { Account = account, Id = next, Name = request.AccountName, Faction = request.Faction,
                    HeadId = request.HeadId, VoiceId = request.VoiceId, UniverseId = request.UniverseId, NpId = (byte[])request.NpId.Clone() };
                if (ProgressionCatalog != null) { created.Experience = ProgressionCatalog.InitialExperience; created.Level = ProgressionCatalog.InitialLevel; created.SkillPoints = ProgressionCatalog.InitialSkillPoints; }
                all.Add(created);
            }
            // Recommit exact retries too: recover a prior uncertain directory sync before success.
            Commit(path, all);
            return Outcome(MagNetResult.Success, existing == null ? "local-profile-committed" : "existing-exact-profile");
        }
        // The authenticated session supplies the account; ownership is rechecked inside the durable lock.
        public CharacterCreationOutcome SaveLoadoutForAccount(string account, uint characterId, byte loadoutIndex, MagLoadout loadout)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || loadoutIndex >= 5 || loadout?.Items == null || loadout.Items.Length != 22)
                return Outcome(MagNetResult.Error, "invalid-loadout-save");
            var replacement = StoredLoadout.FromLoadout(loadout); // Never persist caller-owned arrays.
            var catalog = LoadoutCatalog;
            if (catalog == null) return Outcome(MagNetResult.Error, "loadout-catalog-unavailable");
            if (!catalog.Valid(replacement.ToLoadout())) return Outcome(MagNetResult.Error, "invalid-authored-loadout");
            string path = ValidPath(); Directory.CreateDirectory(Path.GetDirectoryName(path));
            using var lease = AcquireLock(path + ".lock");
            var all = ReadAll().ToList();
            var character = all.SingleOrDefault(c => c.Account == account && c.Id == characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            if (!catalog.ValidForFaction(replacement.ToLoadout(), character.Faction))
                return Outcome(MagNetResult.Error, "invalid-faction-loadout");
            if (!HasLoadoutOwnership(character, catalog, replacement.Items))
                return Outcome(MagNetResult.Error, "loadout-item-not-owned");
            if (character.Loadouts == null || character.Loadouts.All(row => row.Items.All(id => id == -1)))
            {
                // A populated kit1 alone leaves kit0 absent and triggers native InitializeLoadouts,
                // which would discard the edit. Retain every available authored default on first save.
                var defaults = catalog.DefaultsForFaction(character.Faction);
                if (defaults == null) return Outcome(MagNetResult.Error, "authored-faction-defaults-unavailable");
                character.Loadouts = defaults.Select(StoredLoadout.FromLoadout).ToArray();
            }
            character.Loadouts[loadoutIndex] = replacement;
            // The native record importer uses kit0's primary as its default-fallback gate.
            // Gameplay admission also needs the selected kit to have a usable primary.
            if (!catalog.HasValidPrimary(character.Loadouts[0].ToLoadout()))
                return Outcome(MagNetResult.Error, "primary-loadout-required");
            if (!catalog.HasValidPrimary(character.Loadouts[character.LastUsedLoadout].ToLoadout()))
                return Outcome(MagNetResult.Error, "selected-loadout-primary-required");
            Commit(path, all); // Success is returned only after the existing atomic durable commit.
            return Outcome(MagNetResult.Success, "loadout-committed");
        }
        public CharacterCreationOutcome PurchaseSkillForAccount(string account, uint characterId, int skillId)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || skillId < 0 || skillId > 127)
                return Outcome(MagNetResult.Error, "invalid-skill-purchase");
            var catalog = SkillCatalog;
            if (catalog == null || (catalog.ApplicationId == 20230 && LoadoutCatalog == null)) return Outcome(MagNetResult.Error, "skill-catalog-unavailable");
            string path = ValidPath(); Directory.CreateDirectory(Path.GetDirectoryName(path));
            using var lease = AcquireLock(path + ".lock");
            var all = ReadAll().ToList();
            var character = all.SingleOrDefault(c => c.Account == account && c.Id == characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            var record = character.ToRecord();
            if (!catalog.TryPurchase(character.Faction, skillId, record.SkillPoints, record.Skills, out byte cost, out var equipment, out string reason))
                return Outcome(MagNetResult.Error, reason);
            character.SkillPoints = (byte)(record.SkillPoints - cost);
            character.OwnedSkills = character.OwnedSkills == null ? new uint[5] : (uint[])character.OwnedSkills.Clone();
            character.OwnedSkills[skillId >> 5] |= 1u << (skillId & 31);
            character.UnlockedItems = character.UnlockedItems == null ? new uint[24] : (uint[])character.UnlockedItems.Clone();
            foreach (int item in equipment) character.UnlockedItems[item >> 5] |= 1u << (item & 31);
            Commit(path, all); // Atomic durable save must finish before the session acknowledges the purchase.
            return Outcome(MagNetResult.Success, "skill-purchase-committed");
        }
        public CharacterCreationOutcome SaveLastUsedLoadoutForAccount(string account, uint characterId, byte loadoutIndex)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || loadoutIndex >= 5)
                return Outcome(MagNetResult.Error, "invalid-last-used-loadout");
            string path = ValidPath(); Directory.CreateDirectory(Path.GetDirectoryName(path));
            using var lease = AcquireLock(path + ".lock");
            var all = ReadAll().ToList();
            var character = all.SingleOrDefault(c => c.Account == account && c.Id == characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            var catalog = LoadoutCatalog;
            if (catalog == null) return Outcome(MagNetResult.Error, "loadout-catalog-unavailable");
            var rows = character.Loadouts?.Select(row => row.ToLoadout()).ToArray() ?? catalog.DefaultsForFaction(character.Faction);
            if (rows == null) return Outcome(MagNetResult.Error, "authored-faction-defaults-unavailable");
            if (rows.Any(row => !catalog.ValidForFaction(row, character.Faction))) return Outcome(MagNetResult.Error, "invalid-faction-loadout");
            if (!catalog.HasValidPrimary(rows[0])) return Outcome(MagNetResult.Error, "primary-loadout-required");
            if (!catalog.HasValidPrimary(rows[loadoutIndex])) return Outcome(MagNetResult.Error, "selected-loadout-primary-required");
            if (!HasLoadoutOwnership(character, catalog, rows[loadoutIndex].Items))
                return Outcome(MagNetResult.Error, "loadout-item-not-owned");
            character.LastUsedLoadout = loadoutIndex;
            Commit(path, all);
            return Outcome(MagNetResult.Success, "last-used-loadout-committed");
        }
        private bool HasLoadoutOwnership(StoredCharacter character, MagLoadoutCatalog catalog, short[] items)
        {
            if (catalog.ApplicationId != 20231 && SkillCatalog == null) return true;
            var available = character.ToRecord().AvailableItems;
            for (int slot = 0; slot < items.Length; slot++)
            {
                short id = items[slot];
                if (id < 0 || (catalog.ApplicationId != 20231 && !SkillCatalog.RequiresPurchaseOwnership(id))) continue;
                if (catalog.ApplicationId == 20231 && catalog.IsStarterDefaultItem(character.Faction, slot, id)) continue;
                if (!catalog.TryOwnershipOrdinal(id, out int ordinal) || (available[ordinal >> 5] & (1u << (ordinal & 31))) == 0)
                    return false;
            }
            return true;
        }
        private static bool ValidRewardKey(string value) => !string.IsNullOrEmpty(value) && value.Length <= 256 && value.All(c => c >= 33 && c <= 126);
        private static string MatchProgressKey(string account, uint characterId, string matchKey) => account + ":" + characterId.ToString(System.Globalization.CultureInfo.InvariantCulture) + ":" + matchKey;
        private static bool ValidKindUnits(string kind, int units) => MagProgressionCatalog.ValidKindUnits(kind, units);
        private static void ValidateRewardLedger(Document document)
        {
            if (document.SchemaVersion < 3)
            {
                if (document.RewardReceipts != null || document.MatchRewardProgress != null)
                    throw new InvalidDataException("Reward ledger requires character schema three.");
                return;
            }
            if (document.RewardReceipts == null || document.MatchRewardProgress == null ||
                document.RewardReceipts.Count > MaximumRewardReceipts || document.MatchRewardProgress.Count > MaximumRewardReceipts)
                throw new InvalidDataException("Malformed or oversized persistent reward ledger.");
            foreach (var entry in document.MatchRewardProgress)
            {
                var progress = entry.Value;
                if (progress == null || !ValidRewardKey(progress.MatchKey) || progress.Experience < 0 ||
                    !float.IsFinite(progress.Fraction) || progress.Fraction < 0 || progress.Fraction >= 1 ||
                    entry.Key != MatchProgressKey(progress.Account, progress.CharacterId, progress.MatchKey) ||
                    !document.Characters.Any(c => c.Account == progress.Account && c.Id == progress.CharacterId))
                    throw new InvalidDataException("Malformed match reward state.");
            }
            foreach (var entry in document.RewardReceipts)
            {
                var receipt = entry.Value; var snapshot = receipt?.Snapshot;
                if (!ValidRewardKey(entry.Key) || receipt == null || !ValidRewardKey(receipt.MatchKey) ||
                    !ValidKindUnits(receipt.Kind, receipt.Units) || snapshot == null || snapshot.Result != MagNetResult.Success ||
                    snapshot.Reason != "reward-committed" || snapshot.Duplicate || snapshot.Experience < 0 || snapshot.ExperienceAwarded < 0 ||
                    snapshot.MatchExperience < snapshot.ExperienceAwarded || snapshot.Level < 1 || snapshot.PreviousLevel < 1 || snapshot.PreviousLevel > snapshot.Level ||
                    !float.IsFinite(snapshot.MatchFraction) || snapshot.MatchFraction < 0 || snapshot.MatchFraction >= 1 ||
                    !document.Characters.Any(c => c.Account == receipt.Account && c.Id == receipt.CharacterId && (c.Experience ?? 0) >= snapshot.Experience && (c.Level ?? 1) >= snapshot.Level) ||
                    !document.MatchRewardProgress.TryGetValue(MatchProgressKey(receipt.Account, receipt.CharacterId, receipt.MatchKey), out var progress) ||
                    progress.Experience < snapshot.MatchExperience ||
                    MagProgressionCatalog.IsFinalRewardKind(receipt.Kind) && (!progress.Finalized || progress.Experience != snapshot.MatchExperience))
                    throw new InvalidDataException("Malformed immutable reward receipt.");
            }
        }
        /// <summary>Commit authoritative action XP immediately as local policy. IDs must be stable
        /// across delivery retries; matchKey isolates native fractional XP between match lifetimes.</summary>
        public ProgressionRewardOutcome ApplyRewardForAccount(string account, uint characterId, string rewardId, string matchKey, string kind, int units)
        {
            ProgressionRewardOutcome Deny(string reason) => new ProgressionRewardOutcome { Result = MagNetResult.Error, Reason = reason };
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || !ValidRewardKey(rewardId) ||
                !ValidRewardKey(matchKey) || !ValidKindUnits(kind, units)) return Deny("invalid-progression-reward");
            var catalog = ProgressionCatalog;
            if (catalog == null) return Deny("progression-catalog-unavailable");
            string path = ValidPath(); Directory.CreateDirectory(Path.GetDirectoryName(path));
            using var lease = AcquireLock(path + ".lock");
            var document = ReadDocument();
            var character = document.Characters.SingleOrDefault(c => c.Account == account && c.Id == characterId);
            if (character == null) return Deny("character-not-owned");
            document.RewardReceipts = document.RewardReceipts ?? new Dictionary<string, RewardReceipt>(StringComparer.Ordinal);
            document.MatchRewardProgress = document.MatchRewardProgress ?? new Dictionary<string, MatchRewardProgress>(StringComparer.Ordinal);
            if (document.RewardReceipts.TryGetValue(rewardId, out var existing))
            {
                if (existing.Account != account || existing.CharacterId != characterId || existing.MatchKey != matchKey || existing.Kind != kind || existing.Units != units)
                    return Deny("reward-receipt-mismatch");
                return existing.Snapshot.Copy(true); // Historical response stays exact after further actions or purchases.
            }
            if (document.RewardReceipts.Count >= MaximumRewardReceipts) return Deny("reward-receipt-ledger-full");
            string progressKey = MatchProgressKey(account, characterId, matchKey);
            document.MatchRewardProgress.TryGetValue(progressKey, out var progress);
            if (progress == null && document.MatchRewardProgress.Count >= MaximumRewardReceipts) return Deny("match-reward-ledger-full");
            progress = progress ?? new MatchRewardProgress { Account = account, CharacterId = characterId, MatchKey = matchKey };
            if (progress.Finalized) return Deny("match-reward-finalized");
            bool final = MagProgressionCatalog.IsFinalRewardKind(kind);
            int awarded; float fraction = progress.Fraction;
            if (final ? !catalog.TryFinalAward(kind, units, progress.Experience, out awarded) :
                !catalog.TryAward(kind, units, progress.Fraction, out awarded, out fraction)) return Deny("invalid-authored-reward");
            int previousExperience = character.Experience ?? 0;
            if (awarded > int.MaxValue - previousExperience || awarded > int.MaxValue - progress.Experience) return Deny("experience-overflow");
            int experience = previousExperience + awarded;
            byte previousLevel = character.Level ?? 1; byte level = catalog.LevelForExperience(experience);
            byte points = catalog.PointsAfterGain(character.ToRecord().SkillPoints, previousLevel, level);
            var outcome = new ProgressionRewardOutcome { Result = MagNetResult.Success, Reason = "reward-committed",
                Experience = experience, Level = level, PreviousLevel = previousLevel, SkillPoints = points,
                ExperienceAwarded = awarded, MatchExperience = progress.Experience + awarded, MatchFraction = fraction };
            character.Experience = experience; character.Level = level; character.SkillPoints = points;
            progress.Experience = outcome.MatchExperience; progress.Fraction = fraction; progress.Finalized = final;
            document.MatchRewardProgress[progressKey] = progress;
            document.RewardReceipts.Add(rewardId, new RewardReceipt { Account = account, CharacterId = characterId,
                MatchKey = matchKey, Kind = kind, Units = units, Snapshot = outcome.Copy(false) });
            Commit(path, document.Characters, document);
            return outcome.Copy(false);
        }
        public TrainingCompletionOutcome CompleteTrainingForAccount(string account, uint characterId)
        {
            TrainingCompletionOutcome Deny(string reason) => new TrainingCompletionOutcome { Result = MagNetResult.Error, Reason = reason };
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0) return Deny("invalid-training-identity");
            var rules = TrainingCompletionCatalog;
            if (rules == null) return Deny("training-catalog-unavailable");
            string path = ValidPath(); Directory.CreateDirectory(Path.GetDirectoryName(path));
            using var lease = AcquireLock(path + ".lock");
            var document = ReadDocument();
            var character = document.Characters.SingleOrDefault(c => c.Account == account && c.Id == characterId);
            if (character == null) return Deny("character-not-owned");
            if ((character.Flags & rules.CompletedFlagMask) != 0)
            {
                // A previous rename may have become visible before directory fsync failed.
                // Complete the durability barrier again before acknowledging this receipt.
                Commit(path, document.Characters, document);
                return new TrainingCompletionOutcome { Result = MagNetResult.Success, Reason = "training-already-completed", Record = character.ToRecord() };
            }
            int previousExperience = character.Experience ?? 0;
            byte previousLevel = character.Level ?? 1;
            if (previousLevel != rules.Progression.LevelForExperience(previousExperience)) return Deny("training-progression-inconsistent");
            if (rules.ExperienceBonus > int.MaxValue - previousExperience) return Deny("experience-overflow");
            int experience = previousExperience + rules.ExperienceBonus;
            byte level = rules.Progression.LevelForExperience(experience);
            byte points = rules.Progression.PointsAfterGain(character.ToRecord().SkillPoints, previousLevel, level);
            character.Flags |= rules.CompletedFlagMask;
            character.Experience = experience; character.Level = level; character.SkillPoints = points;
            Commit(path, document.Characters, document);
            // The flag is the durable receipt. A failed/uncertain save never returns a success
            // snapshot; after a rename the next request observes it and cannot reward twice.
            return new TrainingCompletionOutcome { Result = MagNetResult.Success, Reason = "training-completed", NewlyCompleted = true, Record = character.ToRecord() };
        }
        private void Commit(string path, List<StoredCharacter> all, Document document = null)
        {
            document = document ?? ReadDocument();
            document.Characters = all;
            document.SchemaVersion = document.SchemaVersion == 3 || ProgressionCatalog != null ? 3 : 2;
            if (document.SchemaVersion == 3)
            {
                document.RewardReceipts = document.RewardReceipts ?? new Dictionary<string, RewardReceipt>(StringComparer.Ordinal);
                document.MatchRewardProgress = document.MatchRewardProgress ?? new Dictionary<string, MatchRewardProgress>(StringComparer.Ordinal);
                foreach (var character in all) { character.Experience = character.Experience ?? 0; character.Level = character.Level ?? 1; }
            }
            var contents = JsonSerializer.SerializeToUtf8Bytes(document, JsonOptions);
            if (contents.Length > MaximumStoreBytes) throw new InvalidDataException("Character store exceeds local bound.");
            _committer.WriteAtomically(path, contents);
        }
        private static CharacterCreationOutcome Outcome(MagNetResult result, string reason) => new CharacterCreationOutcome { Result = result, Reason = reason };
        private static FileStream AcquireLock(string path)
        {
            var timer = System.Diagnostics.Stopwatch.StartNew();
            while (true)
            {
                try { return new FileStream(path, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None); }
                catch (IOException) when (timer.ElapsedMilliseconds < 2000) { Thread.Sleep(10); }
            }
        }
    }
}
