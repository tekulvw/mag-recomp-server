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
    /// <summary>Authoritative saved leadership fields; absence preserves legacy policy fallback.</summary>
    [JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
    public sealed class StoredLeadershipRecord
    {
        [JsonRequired] public byte LeadershipLevel { get; set; }
        [JsonRequired] public byte Nominations { get; set; }
        [JsonRequired] public byte MatchesSinceLastCommand { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public uint? LeadershipPoints { get; set; }
        // Historical rows could carry eligibility with no recorded point balance.
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public byte? LeadershipLevelFloor { get; set; }
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
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public ulong LoadoutLayoutRevision { get; set; }
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
        /// <summary>The character's slot on its account, 0 to <see cref="LocalCharacterStore.CharacterSlots"/> - 1.
        /// Stores written before slots existed have one character per account, which is slot 0.</summary>
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
        public byte Slot { get; set; }
        /// <summary>Unix seconds at which the player deleted the character; null while it is live. A deleted
        /// character keeps its row and progression, but no longer holds its slot or its name (see
        /// LocalCharacterStore.DeleteCharacterForAccount).</summary>
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public long? DeletedAt { get; set; }
        /// <summary>A deleted character's name before the deletion moved it aside; null otherwise.</summary>
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string OriginalName { get; set; }
        /// <summary>Skill respecs the character has taken (retail RespecCount); null for none. It selects the next fee.</summary>
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public byte? RespecCount { get; set; }
        /// <summary>The character's stored respec point balance (retail RespecPoints); null for none. An operator
        /// floor (<see cref="LocalCharacterStore.RespecPointsFloor"/>) is applied when the record is built, not here.</summary>
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public uint? RespecPoints { get; set; }
        [JsonIgnore]
        public bool Deleted => DeletedAt != null;
        [JsonIgnore]
        internal uint? RecordRespecPointsFloor { get; set; }
        /// <summary>The respec points the character can spend: its stored balance, raised to the operator floor
        /// when one is set. A balance above the floor is kept.</summary>
        [JsonIgnore]
        public uint AvailableRespecPoints => Math.Max(RespecPoints ?? 0, RecordRespecPointsFloor ?? 0);
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
                LeadershipPoints = Leadership?.LeadershipPoints ?? 0,
                LeadershipNominations = Leadership?.Nominations ?? 0,
                MatchesSinceLastCommand = Leadership?.MatchesSinceLastCommand ?? 0,
                RespecCount = RespecCount ?? 0, RespecPoints = AvailableRespecPoints,
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
        /// <summary>The name other players see for this character.</summary>
        public string DisplayName() => DisplayNameFor(Name, NpId);
        /// <summary>
        /// A linked character keeps an internal stored name (its login key, such as an online ID
        /// followed by a slot suffix), but other players should see the online ID itself. That ID
        /// is the leading text of the 36-byte NP identity, ending at the first zero byte within its
        /// first 16 bytes. Use it when it is non-empty printable ASCII; otherwise (a zero identity
        /// or anything unexpected) keep the stored name.
        /// </summary>
        public static string DisplayNameFor(string name, byte[] npId)
        {
            if (npId == null || npId.Length != 36) return name;
            int length = Array.IndexOf(npId, (byte)0, 0, 16);
            if (length < 0) length = 16;
            if (length == 0) return name;
            for (int index = 0; index < length; index++)
                if (npId[index] < 33 || npId[index] > 126) return name;
            return System.Text.Encoding.ASCII.GetString(npId, 0, length);
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
    [JsonUnmappedMemberHandling(JsonUnmappedMemberHandling.Disallow)]
    public sealed record LeadershipRewardOutcome(
        [property: JsonRequired] uint LeadershipPoints,
        [property: JsonRequired] byte LeadershipLevel,
        [property: JsonRequired] byte Nominations,
        [property: JsonRequired] byte MatchesSinceLastCommand,
        [property: JsonRequired] bool Changed,
        [property: JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] uint? MatchLeadershipPoints);

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
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public LeadershipRewardOutcome Leadership { get; set; }
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public int? MatchFragoPoints { get; set; }
        [JsonIgnore] public bool ShouldNotifyCharacterRecord => ExperienceAwarded > 0 || Leadership?.Changed == true;
        internal ProgressionRewardOutcome Copy(bool duplicate) => new ProgressionRewardOutcome {
            Result = Result, Reason = Reason, Duplicate = duplicate, Experience = Experience, Level = Level,
            PreviousLevel = PreviousLevel, SkillPoints = SkillPoints, ExperienceAwarded = ExperienceAwarded,
            MatchExperience = MatchExperience, MatchFraction = MatchFraction, Leadership = Leadership, MatchFragoPoints = MatchFragoPoints };
    }
    /// <summary>Planned or applied operator level change; Refused carries a reason and no write.</summary>
    public sealed class AdminLevelChange
    {
        public bool Refusal { get; private set; }
        public bool Applied { get; internal set; }
        public string Reason { get; internal set; }
        public string Account { get; internal set; }
        public uint CharacterId { get; internal set; }
        public string Name { get; internal set; }
        public byte PreviousLevel { get; internal set; }
        public int PreviousExperience { get; internal set; }
        public byte PreviousSkillPoints { get; internal set; }
        public byte Level { get; internal set; }
        public int Experience { get; internal set; }
        public byte SkillPoints { get; internal set; }
        public string BackupPath { get; internal set; }
        public byte Slot { get; internal set; }
        internal static AdminLevelChange Refused(string reason, MagCharacterRecord current = null) => new AdminLevelChange {
            Refusal = true, Reason = reason, PreviousLevel = current?.Level ?? 0, Level = current?.Level ?? 0 };
    }
    /// <summary>A planned or applied operator restore or purge of a deleted character; Refused carries a
    /// reason and no write.</summary>
    public sealed class AdminCharacterChange
    {
        public bool Refusal { get; internal set; }
        public bool Applied { get; internal set; }
        /// <summary>A refusal word (<c>account-not-found</c>, <c>character-not-deleted</c>, <c>ambiguous-character</c>,
        /// <c>character-slots-full</c>, <c>backup-directory-required</c>), or <c>dry-run</c> / <c>applied</c>.</summary>
        public string Reason { get; internal set; }
        public string Account { get; internal set; }
        public uint CharacterId { get; internal set; }
        /// <summary>The character's name after the change: for a restore, its original name when that is free,
        /// else its moved-aside name.</summary>
        public string Name { get; internal set; }
        /// <summary>The stored name before the change (a deleted character's moved-aside name).</summary>
        public string PreviousName { get; internal set; }
        public byte Level { get; internal set; }
        /// <summary>The slot a restore puts the character in; for a purge, the slot it last held.</summary>
        public byte Slot { get; internal set; }
        public long? DeletedAt { get; internal set; }
        public string BackupPath { get; internal set; }
        internal static AdminCharacterChange Refused(string reason, string account, string name = null) =>
            new AdminCharacterChange { Refusal = true, Reason = reason, Account = account, Name = name };
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
    public sealed partial class LocalCharacterStore
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
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public GameplayLeadershipResult Leadership { get; set; }
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public bool? Frago { get; set; }
        }
        private sealed class MatchRewardProgress
        {
            [JsonExtensionData] public Dictionary<string, JsonElement> ExtensionData { get; set; }
            [JsonRequired] public string Account { get; set; }
            [JsonRequired] public uint CharacterId { get; set; }
            [JsonRequired] public string MatchKey { get; set; }
            [JsonRequired] public int Experience { get; set; }
            [JsonRequired] public float Fraction { get; set; }
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public int? FragoPoints { get; set; }
            // Older schema-three matches have no end receipt and remain open.
            public bool Finalized { get; set; }
            // Unix seconds of the row's last committed reward. Rows written before this field existed
            // are all stamped at the next reward commit to any row (see Commit).
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)] public long? LastRewardAt { get; set; }
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
            /// <summary>Server-side voice mutes and bans, at most one of each per account; absent when none
            /// (LocalCharacterStore.VoiceSanctions.cs). An older build keeps the field as extension data.</summary>
            [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
            public List<StoredVoiceSanction> VoiceSanctions { get; set; }
        }
        private static readonly JsonSerializerOptions JsonOptions = new JsonSerializerOptions
        { PropertyNamingPolicy = JsonNamingPolicy.CamelCase, WriteIndented = true };
        public const int MaximumStoreBytes = 1048576; // 128 accounts with five complete inventories each.
        // Settled reward history is bounded: once a commit would leave the store above the trigger, whole
        // finalized and abandoned match rows are evicted (least recently rewarded first) until it is back at the target.
        public const int LedgerCompactionTriggerBytes = MaximumStoreBytes / 4 * 3;
        public const int LedgerCompactionTargetBytes = MaximumStoreBytes / 2;
        // Compactable rows kept for redelivery after a lost acknowledgement; evicted only to fit the hard bound.
        public const int RetainedFinalizedMatchRows = 8;
        // An open row whose last reward is older than this is abandoned: its player left before the
        // match result, or the match, world or worker went away. It becomes compactable like a finalized
        // row, but still accepts a reward while it is kept (see IsCompactable).
        public static readonly TimeSpan DefaultOpenMatchRowTtl = TimeSpan.FromHours(24);
        public static readonly TimeSpan MinimumOpenMatchRowTtl = TimeSpan.FromHours(1);
        public static readonly TimeSpan MaximumOpenMatchRowTtl = TimeSpan.FromDays(30);
        private TimeSpan _openMatchRowTtl = DefaultOpenMatchRowTtl;
        /// <summary>How long an open match row may go without a reward before it can be compacted.</summary>
        public TimeSpan OpenMatchRowTtl
        {
            get => _openMatchRowTtl;
            set => _openMatchRowTtl = value >= MinimumOpenMatchRowTtl && value <= MaximumOpenMatchRowTtl ? value :
                throw new ArgumentOutOfRangeException(nameof(OpenMatchRowTtl), value, "Open match row TTL is outside its bounds.");
        }
        /// <summary>Wall clock for reward-row ages; replaceable by synthetic checks.</summary>
        public Func<DateTimeOffset> Clock { get; set; } = () => DateTimeOffset.UtcNow;
        private readonly string _path;
        public MagLoadoutCatalog LoadoutCatalog { get; set; }
        public MagSkillCatalog SkillCatalog { get; set; }
        public MagProgressionCatalog ProgressionCatalog { get; set; }
        public MagTrainingCompletionCatalog TrainingCompletionCatalog { get; set; }
        // Local receipt bound: reject new awards when full. Active open matches keep every receipt; only
        // finalized and abandoned matches are compacted (see IsCompactable and CompactRewardLedger).
        public const int MaximumRewardReceipts = 4096;
        /// <summary>Receives a one-line operator summary after the reward ledger is compacted.</summary>
        public Action<string> LedgerMaintenance { get; set; }
        // Explicit local development policy, supplied by the caller; authored starter data grants no blanket unlock.
        public byte InitialSkillPoints { get; set; }
        // Operator policy: new characters start at this level as if they had earned it
        // (threshold XP and every gained level's skill points). Null keeps the catalog default.
        public byte? StartingLevel { get; set; }
        // Operator policy: a character completing Training reaches at least this level as if it had
        // earned it (threshold XP and every gained level's skill points). Only new completions are
        // affected; the completed flag stays the receipt. Null keeps the catalog's bonus alone.
        public byte? TrainingLevel { get; set; }
        /// <summary>Operator policy (MAG_RESPEC_POINTS_FLOOR): every character has at least this many respec points
        /// whenever its record is built (at login, on the character screens and after a respec), so a spend is topped
        /// back up. Applied on read; the stored balance is unchanged and a higher one is kept. Null leaves balances as stored.</summary>
        public uint? RespecPointsFloor { get; set; }
        private readonly ICharacterStoreCommitter _committer;
        // Where characters persist: the JSON file below, or PostgreSQL (LocalCharacterStore.Postgres.cs).
        // Every rule in this class runs unchanged against either; a backend only loads and saves.
        private readonly ICharacterBackend _backend;
        public LocalCharacterStore(string path) : this(path, new AtomicCharacterStoreCommitter()) { }
        public LocalCharacterStore(string path, ICharacterStoreCommitter committer)
        { _path = path; _committer = committer ?? throw new ArgumentNullException(nameof(committer)); _backend = new JsonBackend(this); }
        private string ValidPath()
        {
            if (string.IsNullOrWhiteSpace(_path) || !Path.IsPathFullyQualified(_path)) throw new IOException("An absolute character-store path must be configured.");
            return Path.GetFullPath(_path);
        }
        // Readers do not create directories or lock files; usable through a read-only mount.
        /// <summary>Every live character. Deleted characters are left out (see <see cref="ReadEveryCharacter"/>).</summary>
        public IReadOnlyList<StoredCharacter> ReadAll() => _backend.ReadAll().Where(c => !c.Deleted).ToArray();
        /// <summary>Every character, deleted ones included: for operator tools.</summary>
        public IReadOnlyList<StoredCharacter> ReadEveryCharacter() => _backend.ReadAll();
        /// <summary>The account's character with this id, deleted or not; null when it has none. For paths that
        /// follow a character already playing (reward notifications), which a deletion must not break.</summary>
        public StoredCharacter ReadCharacter(string account, uint characterId) => MagDevelopmentAccountPolicy.ValidAccount(account)
            ? _backend.ReadForAccount(account, includeDeleted: true).SingleOrDefault(c => c.Id == characterId) : null;
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
                !SlotsUnique(document.Characters))
                throw new InvalidDataException("Character-store schema or local account policy invalid.");
            if (document.SchemaVersion == 1) foreach (var c in document.Characters) c.Account = "player";
            ValidateRewardLedger(document);
            ValidateVoiceSanctions(document);
            foreach (var character in document.Characters) PrepareCharacter(character);
            return document;
        }
        // Runtime-derived starter masks/defaults enrich replies without rewriting the stored character.
        private void PrepareCharacter(StoredCharacter character)
        {
            character.RecordCatalog = LoadoutCatalog;
            character.RecordInitialSkillPoints = InitialSkillPoints;
            character.RecordRespecPointsFloor = RespecPointsFloor;
            character.RecordProgressionCatalog = ProgressionCatalog;
            if (ProgressionCatalog != null && (character.Level ?? 1) != ProgressionCatalog.LevelForExperience(character.Experience ?? 0))
                throw new InvalidDataException("Persisted XP/level does not match the progression catalog.");
        }
        private static bool ValidName(string name) => !string.IsNullOrWhiteSpace(name) && name.Length <= 32 && name.All(c => c >= 33 && c <= 126);
        /// <summary>Character slots per account. Retail MAG gave every account one character slot and, from
        /// August 2010, two more on the character login screen (a paid subscription there; free here). The
        /// client shows as many slots as the character list's allowed count (0x0237).</summary>
        public const int CharacterSlots = 3;
        /// <summary>No two live characters of one account share a slot. Deleted characters hold none.</summary>
        private static bool SlotsUnique(IEnumerable<StoredCharacter> characters)
        {
            var live = characters.Where(c => !c.Deleted).Select(c => (c.Account, c.Slot)).ToList();
            return live.Distinct().Count() == live.Count;
        }
        /// <summary>The lowest slot <paramref name="account"/>'s live characters leave free, or null when all are taken.</summary>
        private static byte? FreeSlot(IEnumerable<StoredCharacter> characters, string account)
        {
            var taken = characters.Where(c => c.Account == account && !c.Deleted).Select(c => c.Slot).ToHashSet();
            for (byte slot = 0; slot < CharacterSlots; slot++) if (!taken.Contains(slot)) return slot;
            return null;
        }
        private static bool IsFree(IEnumerable<StoredCharacter> characters, string account, byte slot) =>
            slot < CharacterSlots && !characters.Any(c => c.Account == account && !c.Deleted && c.Slot == slot);
        /// <summary>The slot a client-chosen name ends with (<c>!0</c> to <c>!2</c>), or null.</summary>
        internal static byte? NamedSlot(string name) =>
            name != null && name.Length >= 3 && name[^2] == '!' && name[^1] >= '0' && name[^1] < '0' + CharacterSlots ? (byte)(name[^1] - '0') : null;
        /// <summary>The account's live character with this id, or null (another account's, deleted or absent).</summary>
        private static StoredCharacter Owned(IEnumerable<StoredCharacter> characters, string account, uint characterId) =>
            characters.SingleOrDefault(c => c.Account == account && c.Id == characterId && !c.Deleted);
        private static bool Valid(StoredCharacter c, int schema) => c != null && c.Id > 0 && ValidName(c.Name) && c.Faction <= 2 &&
            c.Slot < CharacterSlots && (c.DeletedAt == null || c.DeletedAt >= 0) &&
            (c.OriginalName == null || c.DeletedAt != null && ValidName(c.OriginalName)) &&
            (c.Experience == null || c.Experience >= 0) && (c.Level == null || c.Level >= 1) &&
            (c.Leadership?.LeadershipLevelFloor == null || c.Leadership.LeadershipPoints.HasValue && c.Leadership.LeadershipLevelFloor == c.Leadership.LeadershipLevel) &&
            (c.OwnedSkills == null || c.OwnedSkills.Length == 5) && (c.UnlockedItems == null || c.UnlockedItems.Length == 24) &&
            c.LastUsedLoadout < 5 && (c.Loadouts == null || (c.Loadouts.Length == 5 && c.Loadouts.All(ValidLoadout))) &&
            c.UniverseId == 26 && MagDevelopmentAccountPolicy.ValidNpId(c.NpId, schema == 1 ? null : c.Account) &&
            (schema == 1 ? c.Id == 1 && c.Name == "Player" : MagDevelopmentAccountPolicy.ValidAccount(c.Account));
        private static bool ValidLoadout(StoredLoadout row) => row?.Items != null && row.Items.Length == 22 && row.Items.All(id => id >= -1 && id < 4096);
        /// <summary>The account's live characters in slot order.</summary>
        public IReadOnlyList<StoredCharacter> ReadForAccount(string account) => MagDevelopmentAccountPolicy.ValidAccount(account)
            ? _backend.ReadForAccount(account, includeDeleted: false) : Array.Empty<StoredCharacter>();
        // Compatibility entry point for older local lab tools; live sessions use the account-bound API.
        public CharacterCreationOutcome Create(MagCreateCharacterRequest request) => request?.AccountName == "Player"
            ? CreateForAccount(request, "player") : Outcome(MagNetResult.NameInvalid, "invalid-legacy-name");
        // A concurrent create in PostgreSQL can lose a unique-index race; the retry then sees the winner.
        public CharacterCreationOutcome CreateForAccount(MagCreateCharacterRequest request, string account) =>
            Retrying(() => CreateForAccountOnce(request, account));
        private CharacterCreationOutcome CreateForAccountOnce(MagCreateCharacterRequest request, string account)
        {
            if (request == null || !MagDevelopmentAccountPolicy.HasZeroRsa(request) || !ValidName(request.AccountName) || !MagDevelopmentAccountPolicy.ValidAccount(account))
                return Outcome(MagNetResult.NameInvalid, "invalid-local-name-or-header");
            if (request.Faction > 2 || request.UniverseId != 26)
                return Outcome(MagNetResult.Error, "unsupported-local-profile-fields");
            if (!MagDevelopmentAccountPolicy.ValidNpId(request.NpId, account))
                return Outcome(MagNetResult.Error, "np-id-account-mismatch");
            using var session = _backend.Open(new StoreScope(account, Name: request.AccountName));
            var all = session.Document.Characters;
            // Names are unique across the whole store. A deleted character's name was moved aside, so the
            // client can create a new character under the same "<online ID>!<slot>" name at once.
            var existing = all.SingleOrDefault(c => string.Equals(c.Name, request.AccountName, StringComparison.OrdinalIgnoreCase));
            if (existing != null)
            {
                // Only an exact retry of this account's own creation succeeds again (a lost acknowledgement).
                bool same = existing.Account == account && !existing.Deleted && existing.Name == request.AccountName &&
                    existing.Faction == request.Faction && existing.HeadId == request.HeadId && existing.VoiceId == request.VoiceId &&
                    existing.UniverseId == request.UniverseId && existing.NpId.SequenceEqual(request.NpId);
                if (!same) return Outcome(MagNetResult.NameTaken, "character-name-taken");
            }
            else
            {
                // Retail has no result for a full account; the client's generic error is the closest.
                if (FreeSlot(all, account) is not byte slot) return Outcome(MagNetResult.Error, "character-slots-full");
                // The request has no slot field, but the client names a character after the slot the player picked
                // ("<online ID>!<slot>"); that slot is used when it is free, so the list keeps the player's layout.
                if (NamedSlot(request.AccountName) is byte named && IsFree(all, account, named)) slot = named;
                if (session.CharacterLimit is int limit && all.Count >= limit) return Outcome(MagNetResult.Error, "local-store-full");
                uint next = session.NextCharacterId();
                var created = new StoredCharacter { Account = account, Id = next, Slot = slot, Name = request.AccountName, Faction = request.Faction,
                    HeadId = request.HeadId, VoiceId = request.VoiceId, UniverseId = request.UniverseId, NpId = (byte[])request.NpId.Clone() };
                if (ProgressionCatalog != null)
                {
                    var catalog = ProgressionCatalog;
                    created.Experience = catalog.InitialExperience; created.Level = catalog.InitialLevel; created.SkillPoints = catalog.InitialSkillPoints;
                    if (StartingLevel is byte start && start > catalog.InitialLevel && start <= catalog.MaximumLevel)
                    {
                        created.Experience = catalog.ThresholdForLevel(start); created.Level = start;
                        created.SkillPoints = catalog.PointsAfterGain(catalog.InitialSkillPoints, catalog.InitialLevel, start);
                    }
                    if (catalog.InitialLeadershipPoints is uint leadershipPoints)
                        created.Leadership = new StoredLeadershipRecord {
                            LeadershipPoints = leadershipPoints, LeadershipLevel = catalog.LeadershipLevelForPoints(leadershipPoints, created.Level.Value) };
                }
                all.Add(created);
            }
            // Recommit exact retries too: recover a prior uncertain directory sync before success.
            session.Commit();
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
            using var session = _backend.Open(new StoreScope(account, characterId));
            var all = session.Document.Characters;
            var character = Owned(all, account, characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            if (!catalog.ValidForFaction(replacement.ToLoadout(), character.Faction))
                return Outcome(MagNetResult.Error, "invalid-faction-loadout");
            if (FindUnownedLoadoutSlot(character, catalog, replacement.Items) is int unowned)
                return Outcome(MagNetResult.Error, NotOwned(replacement.Items, unowned));
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
            session.Commit(); // Success is returned only after the durable commit.
            return Outcome(MagNetResult.Success, "loadout-committed");
        }
        public CharacterCreationOutcome DeleteLoadoutForAccount(string account, uint characterId, byte loadoutIndex)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || loadoutIndex >= 5)
                return Outcome(MagNetResult.Error, "invalid-loadout-delete");
            var catalog = LoadoutCatalog;
            if (catalog?.ApplicationId != 20231) return Outcome(MagNetResult.Error, "retail-loadout-catalog-unavailable");
            using var session = _backend.Open(new StoreScope(account, characterId));
            var all = session.Document.Characters;
            var character = Owned(all, account, characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            if (character.LoadoutLayoutRevision == ulong.MaxValue) return Outcome(MagNetResult.Error, "loadout-layout-revision-exhausted");
            var rows = character.Loadouts;
            if (rows == null || rows.All(row => row.Items.All(id => id == -1)))
                rows = catalog.DefaultsForFaction(character.Faction)?.Select(StoredLoadout.FromLoadout).ToArray();
            if (rows == null) return Outcome(MagNetResult.Error, "authored-faction-defaults-unavailable");
            if (!catalog.HasValidPrimary(rows[loadoutIndex].ToLoadout())) return Outcome(MagNetResult.Error, "loadout-delete-row-empty");

            Array.Copy(rows, loadoutIndex + 1, rows, loadoutIndex, rows.Length - 1 - loadoutIndex);
            rows[^1] = StoredLoadout.FromLoadout(new MagLoadout { UpdateId = 0 });
            if (rows.Any(row => !catalog.ValidForFaction(row.ToLoadout(), character.Faction)))
                return Outcome(MagNetResult.Error, "invalid-faction-loadout");
            if (!catalog.HasValidPrimary(rows[0].ToLoadout())) return Outcome(MagNetResult.Error, "primary-loadout-required");
            byte preferred = character.LastUsedLoadout;
            if (preferred > loadoutIndex) preferred--;
            else if (preferred == loadoutIndex) preferred = 0;
            if (!catalog.HasValidPrimary(rows[preferred].ToLoadout())) preferred = 0;
            if (FindUnownedLoadoutSlot(character, catalog, rows[preferred].Items) is int unowned)
                return Outcome(MagNetResult.Error, NotOwned(rows[preferred].Items, unowned));
            character.Loadouts = rows;
            character.LastUsedLoadout = preferred;
            character.LoadoutLayoutRevision++;
            session.Commit();
            return Outcome(MagNetResult.Success, "loadout-delete-committed");
        }
        public CharacterCreationOutcome PurchaseSkillForAccount(string account, uint characterId, int skillId)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || skillId < 0 || skillId > 127)
                return Outcome(MagNetResult.Error, "invalid-skill-purchase");
            var catalog = SkillCatalog;
            if (catalog == null || (catalog.ApplicationId == 20230 && LoadoutCatalog == null)) return Outcome(MagNetResult.Error, "skill-catalog-unavailable");
            using var session = _backend.Open(new StoreScope(account, characterId));
            var all = session.Document.Characters;
            var character = Owned(all, account, characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            var record = character.ToRecord();
            if (!catalog.TryPurchase(character.Faction, skillId, record.SkillPoints, record.Skills, out byte cost, out var equipment, out string reason))
                return Outcome(MagNetResult.Error, reason);
            character.SkillPoints = (byte)(record.SkillPoints - cost);
            character.OwnedSkills = character.OwnedSkills == null ? new uint[5] : (uint[])character.OwnedSkills.Clone();
            character.OwnedSkills[skillId >> 5] |= 1u << (skillId & 31);
            character.UnlockedItems = character.UnlockedItems == null ? new uint[24] : (uint[])character.UnlockedItems.Clone();
            foreach (int item in equipment) character.UnlockedItems[item >> 5] |= 1u << (item & 31);
            session.Commit(); // The durable save must finish before the session acknowledges the purchase.
            return Outcome(MagNetResult.Success, "skill-purchase-committed");
        }
        // Saves the volunteer bits as sent; gameplay masks them by leadership level at admission,
        // so a later rank-up honours a standing preference. A row without a leadership record
        // gains one at the level its native record already publishes (1), with no point balance.
        public CharacterCreationOutcome SaveNominationsForAccount(string account, uint characterId, byte nominations)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0)
                return Outcome(MagNetResult.Error, "invalid-nominations-identity");
            if ((nominations & ~MagRetailNominationsUpdate.KnownBits) != 0)
                return Outcome(MagNetResult.Error, "unsupported-nomination-bits");
            using var session = _backend.Open(new StoreScope(account, characterId));
            var all = session.Document.Characters;
            var character = Owned(all, account, characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            if (character.Leadership?.Nominations == nominations) return Outcome(MagNetResult.Success, "nominations-unchanged");
            character.Leadership ??= new StoredLeadershipRecord { LeadershipLevel = 1 };
            character.Leadership.Nominations = nominations;
            session.Commit(); // The preference is durable before the session handles its join request.
            return Outcome(MagNetResult.Success, "nominations-committed");
        }
        public CharacterCreationOutcome SaveLastUsedLoadoutForAccount(string account, uint characterId, byte loadoutIndex, ulong expectedLayoutRevision = 0)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || loadoutIndex >= 5)
                return Outcome(MagNetResult.Error, "invalid-last-used-loadout");
            using var session = _backend.Open(new StoreScope(account, characterId));
            var all = session.Document.Characters;
            var character = Owned(all, account, characterId);
            if (character == null) return Outcome(MagNetResult.Error, "character-not-owned");
            if (expectedLayoutRevision != character.LoadoutLayoutRevision) return Outcome(MagNetResult.Error, "stale-loadout-layout");
            var catalog = LoadoutCatalog;
            if (catalog == null) return Outcome(MagNetResult.Error, "loadout-catalog-unavailable");
            var rows = character.Loadouts?.Select(row => row.ToLoadout()).ToArray() ?? catalog.DefaultsForFaction(character.Faction);
            if (rows == null) return Outcome(MagNetResult.Error, "authored-faction-defaults-unavailable");
            if (rows.Any(row => !catalog.ValidForFaction(row, character.Faction))) return Outcome(MagNetResult.Error, "invalid-faction-loadout");
            if (!catalog.HasValidPrimary(rows[0])) return Outcome(MagNetResult.Error, "primary-loadout-required");
            if (!catalog.HasValidPrimary(rows[loadoutIndex])) return Outcome(MagNetResult.Error, "selected-loadout-primary-required");
            if (FindUnownedLoadoutSlot(character, catalog, rows[loadoutIndex].Items) is int unowned)
                return Outcome(MagNetResult.Error, NotOwned(rows[loadoutIndex].Items, unowned));
            character.LastUsedLoadout = loadoutIndex;
            session.Commit();
            return Outcome(MagNetResult.Success, "last-used-loadout-committed");
        }
        // Returns the first slot whose item the character does not own, or null when every item is owned.
        private int? FindUnownedLoadoutSlot(StoredCharacter character, MagLoadoutCatalog catalog, short[] items)
        {
            if (catalog.ApplicationId != 20231 && SkillCatalog == null) return null;
            var available = character.ToRecord().AvailableItems;
            for (int slot = 0; slot < items.Length; slot++)
            {
                short id = items[slot];
                if (id < 0 || (catalog.ApplicationId != 20231 && !SkillCatalog.RequiresPurchaseOwnership(id))) continue;
                if (catalog.ApplicationId == 20231 && catalog.IsStarterDefaultItem(character.Faction, slot, id)) continue;
                if (!catalog.TryOwnershipOrdinal(id, out int ordinal)) return slot;
                if ((available[ordinal >> 5] & (1u << (ordinal & 31))) != 0) continue;
                if (catalog.ApplicationId == 20231 && IsNeverAwardedEquipment(character.Faction, catalog, id, ordinal)) continue;
                return slot;
            }
            return null;
        }
        // Retail grants ownership only through creation grants and skill awards. Authored starter kits
        // also equip apparel, uniforms and other gear that no grant or skill ever awards, so ownership
        // cannot gate those items: no character could ever own them. Magazine weapons stay gated, and
        // without a retail skill catalog nothing can be shown to be outside every award.
        private bool IsNeverAwardedEquipment(byte faction, MagLoadoutCatalog catalog, short id, int ordinal) =>
            SkillCatalog?.ApplicationId == 20231 && !catalog.IsMagazineWeapon(id) &&
            !catalog.GrantsOrdinal(faction, ordinal) && !SkillCatalog.CanAward(faction, ordinal);
        private static string NotOwned(short[] items, int slot) =>
            "loadout-item-not-owned:slot=" + slot.ToString(System.Globalization.CultureInfo.InvariantCulture) +
            ":item=" + items[slot].ToString(System.Globalization.CultureInfo.InvariantCulture);
        private static bool ValidRewardKey(string value) => !string.IsNullOrEmpty(value) && value.Length <= 256 && value.All(c => c >= 33 && c <= 126);
        private static string MatchProgressKey(string account, uint characterId, string matchKey) => account + ":" + characterId.ToString(System.Globalization.CultureInfo.InvariantCulture) + ":" + matchKey;
        private static bool ValidKindUnits(string kind, int units) => MagProgressionCatalog.ValidKindUnits(kind, units);
        private static bool ValidLeadershipContext(string kind, GameplayLeadershipResult leadership) =>
            leadership == null || MagProgressionCatalog.IsFinalRewardKind(kind) && leadership.IsValid;

        private static bool ValidLeadershipSnapshot(GameplayLeadershipResult context, LeadershipRewardOutcome outcome)
        {
            if (outcome?.MatchLeadershipPoints is not uint matchPoints) return context == null;
            return context != null && matchPoints <= int.MaxValue && matchPoints <= outcome.LeadershipPoints &&
                (context.Role != 0 || matchPoints == 0);
        }
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
                if (progress == null || !ValidRewardKey(progress.MatchKey) || progress.Experience < 0 || progress.LastRewardAt < 0 ||
                    !float.IsFinite(progress.Fraction) || progress.Fraction < 0 || progress.Fraction >= 1 ||
                    progress.FragoPoints is int tally && (tally < 0 || tally > progress.Experience) ||
                    entry.Key != MatchProgressKey(progress.Account, progress.CharacterId, progress.MatchKey) ||
                    !document.Characters.Any(c => c.Account == progress.Account && c.Id == progress.CharacterId))
                    throw new InvalidDataException("Malformed match reward state.");
            }
            foreach (var entry in document.RewardReceipts)
            {
                var receipt = entry.Value; var snapshot = receipt?.Snapshot;
                if (!ValidRewardKey(entry.Key) || receipt == null || !ValidRewardKey(receipt.MatchKey) ||
                    !ValidKindUnits(receipt.Kind, receipt.Units) || snapshot == null || snapshot.Result != MagNetResult.Success ||
                    !ValidLeadershipContext(receipt.Kind, receipt.Leadership) || !ValidLeadershipSnapshot(receipt.Leadership, snapshot.Leadership) ||
                    !MagProgressionCatalog.ValidFragoContext(receipt.Kind, receipt.Units, receipt.Frago) ||
                    receipt.Frago.HasValue != snapshot.MatchFragoPoints.HasValue ||
                    snapshot.Reason != "reward-committed" || snapshot.Duplicate || snapshot.Experience < 0 || snapshot.ExperienceAwarded < 0 ||
                    snapshot.MatchExperience < snapshot.ExperienceAwarded || snapshot.Level < 1 || snapshot.PreviousLevel < 1 || snapshot.PreviousLevel > snapshot.Level ||
                    !float.IsFinite(snapshot.MatchFraction) || snapshot.MatchFraction < 0 || snapshot.MatchFraction >= 1 ||
                    !document.Characters.Any(c => c.Account == receipt.Account && c.Id == receipt.CharacterId && (c.Experience ?? 0) >= snapshot.Experience && (c.Level ?? 1) >= snapshot.Level) ||
                    !document.MatchRewardProgress.TryGetValue(MatchProgressKey(receipt.Account, receipt.CharacterId, receipt.MatchKey), out var progress) ||
                    progress.Experience < snapshot.MatchExperience ||
                    snapshot.MatchFragoPoints is int tally && (tally < 0 || tally > snapshot.MatchExperience ||
                        progress.FragoPoints is not int currentTally || tally > currentTally) ||
                    MagProgressionCatalog.IsFinalRewardKind(receipt.Kind) && (!progress.Finalized || progress.Experience != snapshot.MatchExperience))
                    throw new InvalidDataException("Malformed immutable reward receipt.");
            }
        }
        /// <summary>Commit authoritative action XP immediately as local policy. IDs must be stable
        /// across delivery retries; matchKey isolates native fractional XP between match lifetimes.</summary>
        public ProgressionRewardOutcome ApplyRewardForAccount(string account, uint characterId, string rewardId, string matchKey, string kind, int units,
            GameplayLeadershipResult leadership = null, bool? frago = null) =>
            // A PostgreSQL receipt-key race (the same reward id for another character) retries as a replay.
            Retrying(() => ApplyRewardForAccountOnce(account, characterId, rewardId, matchKey, kind, units, leadership, frago));
        private ProgressionRewardOutcome ApplyRewardForAccountOnce(string account, uint characterId, string rewardId, string matchKey, string kind, int units,
            GameplayLeadershipResult leadership, bool? frago)
        {
            ProgressionRewardOutcome Deny(string reason) => new ProgressionRewardOutcome { Result = MagNetResult.Error, Reason = reason };
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || !ValidRewardKey(rewardId) ||
                !ValidRewardKey(matchKey) || !ValidKindUnits(kind, units) || !ValidLeadershipContext(kind, leadership) ||
                !MagProgressionCatalog.ValidFragoContext(kind, units, frago)) return Deny("invalid-progression-reward");
            var catalog = ProgressionCatalog;
            if (catalog == null) return Deny("progression-catalog-unavailable");
            using var session = _backend.Open(new StoreScope(account, characterId, RewardId: rewardId, MatchKey: matchKey));
            var document = session.Document;
            // A deleted character still takes its rewards: they were earned before the player deleted it from the
            // character list, gameplay keeps resending a refused one, and a restore brings the progression back whole.
            var character = document.Characters.SingleOrDefault(c => c.Account == account && c.Id == characterId);
            if (character == null) return Deny("character-not-owned");
            document.RewardReceipts = document.RewardReceipts ?? new Dictionary<string, RewardReceipt>(StringComparer.Ordinal);
            document.MatchRewardProgress = document.MatchRewardProgress ?? new Dictionary<string, MatchRewardProgress>(StringComparer.Ordinal);
            if (document.RewardReceipts.TryGetValue(rewardId, out var existing))
            {
                if (existing.Account != account || existing.CharacterId != characterId || existing.MatchKey != matchKey || existing.Kind != kind || existing.Units != units || existing.Leadership != leadership || existing.Frago != frago)
                    return Deny("reward-receipt-mismatch");
                return existing.Snapshot.Copy(true); // Historical response stays exact after further actions or purchases.
            }
            if (frago == true && !catalog.CanApplyFragoBonus(kind)) return Deny("frago-bonus-unavailable");
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
            int? matchFragoPoints = null;
            if (frago == true)
            {
                if (!catalog.TryFragoBonus(awarded, out int bonus)) return Deny("invalid-authored-frago-bonus");
                if (bonus > int.MaxValue - awarded) return Deny("experience-overflow");
                int previousFragoPoints = progress.FragoPoints.GetValueOrDefault();
                if (bonus > int.MaxValue - previousFragoPoints) return Deny("frago-points-overflow");
                awarded += bonus;
                matchFragoPoints = previousFragoPoints + bonus;
            }
            int previousExperience = character.Experience ?? 0;
            if (awarded > int.MaxValue - previousExperience || awarded > int.MaxValue - progress.Experience) return Deny("experience-overflow");
            int experience = previousExperience + awarded;
            byte previousLevel = character.Level ?? 1; byte level = catalog.LevelForExperience(experience);
            byte points = catalog.PointsAfterGain(character.ToRecord().SkillPoints, previousLevel, level);
            uint? matchLeadershipPoints = null;
            if (leadership != null)
            {
                if (!catalog.TryLeadershipAward(leadership.Role, kind, out uint leadershipPoints)) return Deny("invalid-authored-leadership-reward");
                if (leadershipPoints > uint.MaxValue - (character.Leadership?.LeadershipPoints ?? 0)) return Deny("leadership-points-overflow");
                matchLeadershipPoints = leadershipPoints;
            }
            var outcome = new ProgressionRewardOutcome { Result = MagNetResult.Success, Reason = "reward-committed",
                Experience = experience, Level = level, PreviousLevel = previousLevel, SkillPoints = points,
                ExperienceAwarded = awarded, MatchExperience = progress.Experience + awarded, MatchFraction = fraction, MatchFragoPoints = matchFragoPoints,
                Leadership = UpdateLeadership(character, catalog, level, leadership?.Baseline, matchLeadershipPoints) };
            character.Experience = experience; character.Level = level; character.SkillPoints = points;
            progress.Experience = outcome.MatchExperience; progress.Fraction = fraction; progress.Finalized = final;
            progress.FragoPoints = matchFragoPoints ?? progress.FragoPoints;
            // A reward also revives an abandoned row that compaction has not yet evicted.
            progress.LastRewardAt = Clock().ToUnixTimeSeconds();
            // Row order records recency: the touched row moves last, so compaction evicts stale rows first.
            document.MatchRewardProgress = document.MatchRewardProgress.Where(e => e.Key != progressKey)
                .Append(new KeyValuePair<string, MatchRewardProgress>(progressKey, progress)).ToDictionary(e => e.Key, e => e.Value, StringComparer.Ordinal);
            document.RewardReceipts.Add(rewardId, new RewardReceipt { Account = account, CharacterId = characterId,
                MatchKey = matchKey, Kind = kind, Units = units, Snapshot = outcome.Copy(false), Leadership = leadership, Frago = frago });
            session.Commit(progressKey);
            return outcome.Copy(false);
        }
        private static LeadershipRewardOutcome UpdateLeadership(StoredCharacter character, MagProgressionCatalog catalog, byte characterLevel,
            GameplayLeadershipBaseline baseline = null, uint? matchPoints = null)
        {
            var previous = character.Leadership;
            if (!catalog.InitialLeadershipPoints.HasValue || previous?.LeadershipPoints == null && !matchPoints.HasValue) return null;
            uint previousPoints = previous?.LeadershipPoints ?? 0;
            uint points = checked(previousPoints + matchPoints.GetValueOrDefault());
            byte nativeLevel = catalog.LeadershipLevelForPoints(points, characterLevel);
            byte? floor = previous?.LeadershipPoints == null ? previous?.LeadershipLevel : previous.LeadershipLevelFloor;
            if (floor <= nativeLevel) floor = null;
            byte level = floor ?? nativeLevel;
            byte nominations = previous?.Nominations ?? baseline.Nominations;
            byte recency = previous?.MatchesSinceLastCommand ?? baseline.MatchesSinceLastCommand;
            character.Leadership = new StoredLeadershipRecord {
                LeadershipPoints = points, LeadershipLevel = level, LeadershipLevelFloor = floor,
                Nominations = nominations, MatchesSinceLastCommand = recency };
            bool changed = points != previousPoints || level != (previous?.LeadershipLevel ?? 1) ||
                nominations != (previous?.Nominations ?? 0) || recency != (previous?.MatchesSinceLastCommand ?? 0);
            return changed || matchPoints.HasValue ? new LeadershipRewardOutcome(points, level, nominations, recency, changed, matchPoints) : null;
        }
        public TrainingCompletionOutcome CompleteTrainingForAccount(string account, uint characterId)
        {
            TrainingCompletionOutcome Deny(string reason) => new TrainingCompletionOutcome { Result = MagNetResult.Error, Reason = reason };
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0) return Deny("invalid-training-identity");
            var rules = TrainingCompletionCatalog;
            if (rules == null) return Deny("training-catalog-unavailable");
            using var session = _backend.Open(new StoreScope(account, characterId));
            var document = session.Document;
            var character = Owned(document.Characters, account, characterId);
            if (character == null) return Deny("character-not-owned");
            if ((character.Flags & rules.CompletedFlagMask) != 0)
            {
                // A previous rename may have become visible before directory fsync failed.
                // Complete the durability barrier again before acknowledging this receipt.
                session.Commit();
                return new TrainingCompletionOutcome { Result = MagNetResult.Success, Reason = "training-already-completed", Record = character.ToRecord() };
            }
            int previousExperience = character.Experience ?? 0;
            byte previousLevel = character.Level ?? 1;
            if (previousLevel != rules.Progression.LevelForExperience(previousExperience)) return Deny("training-progression-inconsistent");
            if (rules.ExperienceBonus > int.MaxValue - previousExperience) return Deny("experience-overflow");
            int experience = previousExperience + rules.ExperienceBonus;
            byte level = rules.Progression.LevelForExperience(experience);
            if (TrainingLevel is byte minimum && level < minimum)
            {
                if (minimum > rules.Progression.MaximumLevel) return Deny("training-level-unsupported");
                // The bonus left the character below the minimum, so the threshold is above its XP.
                experience = rules.Progression.ThresholdForLevel(minimum); level = minimum;
            }
            byte points = rules.Progression.PointsAfterGain(character.ToRecord().SkillPoints, previousLevel, level);
            character.Flags |= rules.CompletedFlagMask;
            character.Experience = experience; character.Level = level; character.SkillPoints = points;
            UpdateLeadership(character, rules.Progression, level);
            session.Commit();
            // The flag is the durable receipt. A failed/uncertain save never returns a success
            // snapshot; after a rename the next request observes it and cannot reward twice.
            return new TrainingCompletionOutcome { Result = MagNetResult.Success, Reason = "training-completed", NewlyCompleted = true, Record = character.ToRecord() };
        }
        /// <summary>
        /// Operator maintenance: raise one character to a level exactly as earned progression would,
        /// setting XP to the level threshold and granting the skill points of each level gained.
        /// Runs under the store lock; with <paramref name="apply"/> it first copies the current store
        /// into <paramref name="backupDirectory"/>. Lowering is refused because spent points cannot be
        /// reconciled. This overload names the account's only live character; an account with several
        /// is refused (<c>ambiguous-character</c>) and needs the character id.
        /// </summary>
        public AdminLevelChange RaiseLevelForAdmin(string account, byte level, bool apply, string backupDirectory)
        {
            var live = ReadForAccount(account);
            if (live.Count > 1) return AdminLevelChange.Refused("ambiguous-character");
            return RaiseLevelForAdmin(account, live.Count == 1 ? live[0].Id : 0, level, apply, backupDirectory);
        }
        /// <summary>As <see cref="RaiseLevelForAdmin(string, byte, bool, string)"/>, for the account's live character <paramref name="characterId"/>.</summary>
        public AdminLevelChange RaiseLevelForAdmin(string account, uint characterId, byte level, bool apply, string backupDirectory)
        {
            var catalog = ProgressionCatalog;
            if (catalog == null) return AdminLevelChange.Refused("progression-catalog-unavailable");
            if (!MagDevelopmentAccountPolicy.ValidAccount(account)) return AdminLevelChange.Refused("invalid-account");
            if (level < 1 || level > catalog.MaximumLevel) return AdminLevelChange.Refused("level-out-of-range");
            using var session = _backend.Open(new StoreScope(account, characterId, CreateDirectory: false));
            var document = session.Document;
            var character = Owned(document.Characters, account, characterId);
            if (character == null) return AdminLevelChange.Refused("account-not-found");
            var before = character.ToRecord();
            if (level < before.Level) return AdminLevelChange.Refused("lowering-unsupported", before);
            var change = new AdminLevelChange { Account = account, CharacterId = character.Id, Name = character.Name, Slot = character.Slot,
                PreviousLevel = before.Level, PreviousExperience = (int)before.Experience, PreviousSkillPoints = before.SkillPoints,
                Level = level, Experience = level == before.Level ? (int)before.Experience : catalog.ThresholdForLevel(level),
                SkillPoints = catalog.PointsAfterGain(before.SkillPoints, before.Level, level) };
            if (change.Level == change.PreviousLevel) { change.Reason = "unchanged"; return change; }
            if (!apply) { change.Reason = "dry-run"; return change; }
            if (string.IsNullOrWhiteSpace(backupDirectory) || !Path.IsPathFullyQualified(backupDirectory))
                return AdminLevelChange.Refused("backup-directory-required", before);
            change.BackupPath = session.Backup(backupDirectory, "set-level", account, character.Id,
                level.ToString(System.Globalization.CultureInfo.InvariantCulture));
            character.Experience = change.Experience; character.Level = change.Level; character.SkillPoints = change.SkillPoints;
            UpdateLeadership(character, catalog, change.Level);
            session.Commit();
            change.Applied = true; change.Reason = "applied";
            return change;
        }
        /// <summary>
        /// The player deletes one of its characters from the character list (0x023C). Retail deletes it; this
        /// server keeps it instead, deliberately: the character leaves the list and frees its slot, but its
        /// row, kits, progression and reward history stay, marked with the time, so a misclick or a stolen
        /// account cannot destroy a character for good. Its name is moved aside (<c>alpha!0</c> becomes
        /// <c>alpha!0~7</c> for character 7, the original kept in <see cref="StoredCharacter.OriginalName"/>):
        /// the client names characters after their slot, so a new character in the same slot needs the name
        /// at once. Operators bring a deleted character back with <see cref="RestoreCharacterForAdmin"/> or
        /// remove it with <see cref="PurgeCharacterForAdmin"/>. The request names the character by id and
        /// name, which must both match a live character of the authenticated account.
        /// </summary>
        public CharacterCreationOutcome DeleteCharacterForAccount(string account, uint characterId, string name)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0 || !ValidName(name))
                return Outcome(MagNetResult.Error, "invalid-character-delete");
            using var session = _backend.Open(new StoreScope(account, characterId, Name: AsideName(name, characterId)));
            var all = session.Document.Characters;
            var character = Owned(all, account, characterId);
            if (character == null || character.Name != name) return Outcome(MagNetResult.Error, "character-not-owned");
            string aside = AsideName(name, characterId);
            if (all.Any(c => c != character && string.Equals(c.Name, aside, StringComparison.OrdinalIgnoreCase)))
                return Outcome(MagNetResult.Error, "character-aside-name-taken");
            session.Journal("character-delete", account, character.Id);
            character.DeletedAt = Clock().ToUnixTimeSeconds();
            character.OriginalName = character.Name; character.Name = aside;
            session.Commit(); // The deletion is durable before the session acknowledges it.
            return Outcome(MagNetResult.Success, "character-deleted");
        }
        /// <summary>A deleted character's stored name: its name and <c>~id</c>, the name shortened to fit 32 characters.</summary>
        internal static string AsideName(string name, uint characterId)
        {
            string suffix = "~" + characterId.ToString(System.Globalization.CultureInfo.InvariantCulture);
            return name[..Math.Min(name.Length, 32 - suffix.Length)] + suffix;
        }
        /// <summary>The account's deleted character named <paramref name="name"/> (any case): by its moved-aside
        /// name, else by the name it had. A name several deleted characters had is ambiguous.</summary>
        private static (StoredCharacter Character, string Refusal) DeletedCharacter(Document document, string account, string name)
        {
            bool Is(string value) => string.Equals(value, name, StringComparison.OrdinalIgnoreCase);
            var mine = document.Characters.Where(c => c.Account == account).ToList();
            var exact = mine.SingleOrDefault(c => Is(c.Name)); // names are unique in the store
            if (exact?.Deleted == true) return (exact, null);
            // A live character may hold the name again (a new one in the same slot); the deleted one had it.
            var had = mine.Where(c => c.Deleted && Is(c.OriginalName)).ToList();
            return had.Count switch
            {
                1 => (had[0], null),
                > 1 => (null, "ambiguous-character"),
                _ => (null, exact != null ? "character-not-deleted" : "account-not-found")
            };
        }
        /// <summary>The original name of the deleted character <paramref name="name"/> names, read ahead of the
        /// session so that the session also loads (and locks) whoever holds that name now.</summary>
        private string OriginalNameOf(string account, string name)
        {
            var exact = _backend.ReadForAccount(account, includeDeleted: true)
                .FirstOrDefault(c => c.Deleted && string.Equals(c.Name, name, StringComparison.OrdinalIgnoreCase));
            return exact != null ? exact.OriginalName : name;
        }
        /// <summary>
        /// Operator maintenance: bring a deleted character back into the lowest free slot of its account,
        /// with everything it had, and its original name when no other character holds it now (else it keeps
        /// its moved-aside name, which the client logs in with just as well). Refused when the account's
        /// slots are all taken. Dry run unless
        /// <paramref name="apply"/>; an applied restore is journalled (mag.admin_changes, or a copy of the
        /// JSON store in <paramref name="backupDirectory"/>).
        /// </summary>
        public AdminCharacterChange RestoreCharacterForAdmin(string account, string name, bool apply, string backupDirectory)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || !ValidName(name)) return AdminCharacterChange.Refused("account-not-found", account, name);
            // The scope also loads whoever holds the original name now, wherever it is.
            using var session = _backend.Open(new StoreScope(account, CreateDirectory: false, Name: OriginalNameOf(account, name) ?? name));
            var document = session.Document;
            var (character, refusal) = DeletedCharacter(document, account, name);
            if (refusal != null) return AdminCharacterChange.Refused(refusal, account, name);
            if (FreeSlot(document.Characters, account) is not byte slot) return AdminCharacterChange.Refused("character-slots-full", account, character.Name);
            string restored = character.OriginalName != null && !document.Characters.Any(c => c != character &&
                string.Equals(c.Name, character.OriginalName, StringComparison.OrdinalIgnoreCase)) ? character.OriginalName : character.Name;
            var change = new AdminCharacterChange { Account = account, CharacterId = character.Id, Name = restored, PreviousName = character.Name,
                Level = character.ToRecord().Level, Slot = slot, DeletedAt = character.DeletedAt, Reason = "dry-run" };
            if (!apply) return change;
            if (_backend is JsonBackend && (string.IsNullOrWhiteSpace(backupDirectory) || !Path.IsPathFullyQualified(backupDirectory)))
                return AdminCharacterChange.Refused("backup-directory-required", account, character.Name);
            change.BackupPath = session.Backup(backupDirectory, "character-restore", account, character.Id, character.Id.ToString(System.Globalization.CultureInfo.InvariantCulture));
            character.DeletedAt = null; character.Slot = slot; character.Name = restored; character.OriginalName = null;
            session.Commit();
            change.Applied = true; change.Reason = "applied";
            return change;
        }
        /// <summary>
        /// Operator maintenance: remove a deleted character for good, with its kits, match rows, reward
        /// receipts and match history. <paramref name="name"/> is its moved-aside name or the one it had. Only a deleted character can be purged. Dry
        /// run unless <paramref name="apply"/>; an applied purge keeps the character's last state in its
        /// journal (mag.admin_changes, or the copy of the JSON store in <paramref name="backupDirectory"/>).
        /// The account's voice sanctions stay: they belong to the account, not to a character.
        /// </summary>
        public AdminCharacterChange PurgeCharacterForAdmin(string account, string name, bool apply, string backupDirectory)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || !ValidName(name)) return AdminCharacterChange.Refused("account-not-found", account, name);
            using var session = _backend.Open(new StoreScope(account, CreateDirectory: false));
            var document = session.Document;
            var (character, refusal) = DeletedCharacter(document, account, name);
            if (refusal != null) return AdminCharacterChange.Refused(refusal, account, name);
            var change = new AdminCharacterChange { Account = account, CharacterId = character.Id, Name = character.OriginalName ?? character.Name,
                PreviousName = character.Name, Level = character.ToRecord().Level, Slot = character.Slot, DeletedAt = character.DeletedAt, Reason = "dry-run" };
            if (!apply) return change;
            if (_backend is JsonBackend && (string.IsNullOrWhiteSpace(backupDirectory) || !Path.IsPathFullyQualified(backupDirectory)))
                return AdminCharacterChange.Refused("backup-directory-required", account, character.Name);
            change.BackupPath = session.Backup(backupDirectory, "character-purge", account, character.Id, character.Id.ToString(System.Globalization.CultureInfo.InvariantCulture));
            document.Characters.Remove(character);
            // The JSON store holds the whole ledger; PostgreSQL's rows go with the character (ON DELETE CASCADE).
            if (document.MatchRewardProgress != null)
                foreach (var key in document.MatchRewardProgress.Where(e => e.Value.Account == account && e.Value.CharacterId == character.Id)
                    .Select(e => e.Key).ToList())
                    document.MatchRewardProgress.Remove(key);
            if (document.RewardReceipts != null)
                foreach (var key in document.RewardReceipts.Where(e => e.Value.Account == account && e.Value.CharacterId == character.Id)
                    .Select(e => e.Key).ToList())
                    document.RewardReceipts.Remove(key);
            session.Commit();
            change.Applied = true; change.Reason = "applied";
            return change;
        }
        /// <summary>
        /// Writes the store. <paramref name="keepHistoryToBound"/> (an export, see ExportTo) keeps all the
        /// settled history that fits the store's hard bounds instead of compacting it to the target.
        /// </summary>
        private void Commit(string path, List<StoredCharacter> all, Document document = null, string touchedProgress = null,
            bool keepHistoryToBound = false)
        {
            document = document ?? ReadDocument();
            document.Characters = all;
            long now = Clock().ToUnixTimeSeconds();
            document.SchemaVersion = document.SchemaVersion == 3 || ProgressionCatalog != null ? 3 : 2;
            if (document.SchemaVersion == 3)
            {
                document.RewardReceipts = document.RewardReceipts ?? new Dictionary<string, RewardReceipt>(StringComparer.Ordinal);
                document.MatchRewardProgress = document.MatchRewardProgress ?? new Dictionary<string, MatchRewardProgress>(StringComparer.Ordinal);
                foreach (var character in all) { character.Experience = character.Experience ?? 0; character.Level = character.Level ?? 1; }
                // On a reward commit to any row, every undated row in the store (written before rows were
                // dated, possibly for a match that will never reward again) starts its TTL now. A date in the
                // future, from a clock that moved back, is clamped so no row can postpone its expiry
                // indefinitely. Other saves leave the ledger as it is; an undated row is never compactable.
                if (touchedProgress != null)
                    foreach (var row in document.MatchRewardProgress.Values)
                        if (!(row.LastRewardAt <= now)) row.LastRewardAt = now;
            }
            var contents = JsonSerializer.SerializeToUtf8Bytes(document, JsonOptions);
            string compaction = null;
            long expiredBefore = now - (long)OpenMatchRowTtl.TotalSeconds;
            if (document.SchemaVersion == 3 && (contents.Length > (keepHistoryToBound ? MaximumStoreBytes : LedgerCompactionTriggerBytes) ||
                LedgerOverCount(document)))
                contents = CompactRewardLedger(document, touchedProgress, expiredBefore, contents,
                    keepHistoryToBound ? MaximumStoreBytes : LedgerCompactionTargetBytes, out compaction);
            if (contents.Length > MaximumStoreBytes || LedgerOverCount(document))
                throw new InvalidDataException(DescribeOverflow(document, expiredBefore, contents.Length));
            _committer.WriteAtomically(path, contents);
            if (compaction != null) LedgerMaintenance?.Invoke(compaction);
        }
        /// <summary>
        /// A row is compactable once it can no longer need its state: it is finalized (its match result was
        /// credited, so it accepts no further award), or it is open but abandoned, with no reward for longer
        /// than OpenMatchRowTtl. Action XP is credited to the character as each reward commits, so dropping
        /// an abandoned row loses only its sub-point fraction and its replay receipts. The row that the
        /// current commit touches is never compactable.
        /// </summary>
        private static bool IsCompactable(MatchRewardProgress row, long expiredBefore) =>
            row.Finalized || row.LastRewardAt < expiredBefore;
        /// <summary>More receipts or match rows than a reader accepts. A live store never gets there (an
        /// award is refused at the bound); an export of a larger PostgreSQL ledger can.</summary>
        private static bool LedgerOverCount(Document document) => document.SchemaVersion == 3 &&
            (document.RewardReceipts.Count > MaximumRewardReceipts || document.MatchRewardProgress.Count > MaximumRewardReceipts);
        /// <summary>
        /// Evicts settled and abandoned history so the store keeps accepting rewards. Compactable rows are
        /// dropped together with their receipts, least recently rewarded first, until the store is at the
        /// compaction target. Active open rows are never evicted, nor is the row this commit touches. The
        /// most recently rewarded compactable rows stay for redelivery after a lost acknowledgement and are
        /// evicted only when the store would otherwise exceed its hard bound. Either way, rows are evicted
        /// until the receipt and match-row counts are within what a reader accepts.
        /// </summary>
        private static byte[] CompactRewardLedger(Document document, string touchedProgress, long expiredBefore, byte[] contents,
            int targetBytes, out string summary)
        {
            int beforeBytes = contents.Length, beforeReceipts = document.RewardReceipts.Count, beforeRows = document.MatchRewardProgress.Count;
            var receiptsByRow = document.RewardReceipts
                .GroupBy(e => MatchProgressKey(e.Value.Account, e.Value.CharacterId, e.Value.MatchKey), StringComparer.Ordinal)
                .ToDictionary(g => g.Key, g => g.Select(e => e.Key).ToList(), StringComparer.Ordinal);
            // Stable: rows rewarded in the same second keep their row order, which also records recency.
            var settled = document.MatchRewardProgress.Where(e => e.Key != touchedProgress && IsCompactable(e.Value, expiredBefore))
                .OrderBy(e => e.Value.LastRewardAt).Select(e => e.Key).ToList();
            int retainedFrom = Math.Max(0, settled.Count - RetainedFinalizedMatchRows), next = 0, abandoned = 0;
            long Evict(string row)
            {
                var receipts = receiptsByRow.TryGetValue(row, out var ids) ? ids : new List<string>();
                long size = JsonSerializer.SerializeToUtf8Bytes(new { row = document.MatchRewardProgress[row],
                    receipts = receipts.ToDictionary(id => id, id => document.RewardReceipts[id], StringComparer.Ordinal) }, JsonOptions).Length;
                foreach (var id in receipts) document.RewardReceipts.Remove(id);
                if (!document.MatchRewardProgress[row].Finalized) abandoned++;
                document.MatchRewardProgress.Remove(row);
                return size;
            }
            foreach (var (limit, goal) in new[] { (retainedFrom, targetBytes), (settled.Count, MaximumStoreBytes) })
                while ((contents.Length > goal || LedgerOverCount(document)) && next < limit)
                {
                    long estimate = contents.Length;
                    while ((estimate > goal || LedgerOverCount(document)) && next < limit) estimate -= Evict(settled[next++]);
                    // Rebuilt without free slots, so rows added later still serialize last.
                    document.RewardReceipts = document.RewardReceipts.ToDictionary(e => e.Key, e => e.Value, StringComparer.Ordinal);
                    document.MatchRewardProgress = document.MatchRewardProgress.ToDictionary(e => e.Key, e => e.Value, StringComparer.Ordinal);
                    contents = JsonSerializer.SerializeToUtf8Bytes(document, JsonOptions);
                }
            summary = next == 0 ? null : $"Character store reward ledger compacted: evicted {next} match rows " +
                $"({next - abandoned} finalized, {abandoned} abandoned open; {beforeReceipts - document.RewardReceipts.Count} receipts), " +
                $"{beforeBytes} -> {contents.Length} bytes, {document.MatchRewardProgress.Count} of {beforeRows} match rows kept";
            return contents;
        }
        private static string DescribeOverflow(Document document, long expiredBefore, int bytes)
        {
            if (document.SchemaVersion != 3) return $"Character store exceeds local bound: {bytes} of {MaximumStoreBytes} bytes.";
            if (bytes <= MaximumStoreBytes)
                return $"Character store exceeds local bound: {document.RewardReceipts.Count} reward receipts and " +
                    $"{document.MatchRewardProgress.Count} match rows (at most {MaximumRewardReceipts} each) after evicting settled matches.";
            var open = document.MatchRewardProgress.Where(e => !IsCompactable(e.Value, expiredBefore)).Select(e => e.Key).ToHashSet(StringComparer.Ordinal);
            int openReceipts = document.RewardReceipts.Values.Count(r => open.Contains(MatchProgressKey(r.Account, r.CharacterId, r.MatchKey)));
            return $"Character store exceeds local bound: {bytes} of {MaximumStoreBytes} bytes after evicting settled matches; " +
                $"{open.Count} open match rows hold {openReceipts} of {document.RewardReceipts.Count} reward receipts " +
                $"and {document.MatchRewardProgress.Count - open.Count} finalized or abandoned rows remain.";
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
