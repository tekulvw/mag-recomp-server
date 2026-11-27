using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;

namespace Server.Medius
{
    /// <summary>One character as the operator tools report it.</summary>
    public sealed class MagAdminAccount
    {
        public string Account { get; init; }
        public uint CharacterId { get; init; }
        public string Name { get; init; }
        public int Faction { get; init; }
        /// <summary>The character's slot on its account (for a deleted character, the slot it last held).</summary>
        public byte Slot { get; init; }
        /// <summary>When the player deleted it; null while it is live.</summary>
        public DateTimeOffset? DeletedAt { get; init; }
        public bool Deleted => DeletedAt != null;
        public byte Level { get; init; }
        public uint Experience { get; init; }
        public byte SkillPoints { get; init; }
        /// <summary><c>online</c>, <c>offline</c> or <c>unknown</c> (no session marker).</summary>
        public string Session { get; init; }
        /// <summary>False when the store has no progression catalog; then there is no next level.</summary>
        public bool HasProgressionCatalog { get; init; }
        /// <summary>Next level and its XP threshold; null at the maximum level or without a catalog.</summary>
        public byte? NextLevel { get; init; }
        public int? NextLevelExperience { get; init; }
    }

    /// <summary>Why a level change was not made. <see cref="None"/> means planned (dry run) or applied.</summary>
    public enum MagAdminRefusal
    {
        None, NotFound, Online, SessionUnknown, LowerLevel, SameLevel, LevelOutOfRange, NoProgressionCatalog, Failed,
        /// <summary>The account has several live characters and the request named none of them.</summary>
        Ambiguous,
        /// <summary>A restore found every slot of the account taken.</summary>
        SlotsFull,
        /// <summary>A restore or purge named a live character; only deleted ones can be.</summary>
        NotDeleted
    }

    public sealed class MagAdminSetLevelResult
    {
        public MagAdminRefusal Refusal { get; init; }
        /// <summary>Operator-readable explanation of a refusal, starting with the store's reason word.</summary>
        public string Message { get; init; }
        /// <summary>A dry run for an account that currently holds a live login.</summary>
        public bool OnlineWarning { get; init; }
        /// <summary>The planned or applied change; for <see cref="MagAdminRefusal.SameLevel"/> it describes the current state.</summary>
        public AdminLevelChange Change { get; init; }

        /// <summary>Stable API code for a refusal (<c>online</c>, <c>lower_level</c>, …); null for none.</summary>
        public string Code => CodeOf(Refusal);

        public static string CodeOf(MagAdminRefusal refusal) => refusal switch
        {
            MagAdminRefusal.NotFound => "not_found",
            MagAdminRefusal.Online => "online",
            MagAdminRefusal.SessionUnknown => "session_unknown",
            MagAdminRefusal.LowerLevel => "lower_level",
            MagAdminRefusal.SameLevel => "same_level",
            MagAdminRefusal.LevelOutOfRange => "level_out_of_range",
            MagAdminRefusal.NoProgressionCatalog => "no_progression_catalog",
            MagAdminRefusal.Failed => "failed",
            MagAdminRefusal.Ambiguous => "ambiguous_character",
            MagAdminRefusal.SlotsFull => "slots_full",
            MagAdminRefusal.NotDeleted => "not_deleted",
            _ => null
        };
    }

    /// <summary>A planned (dry run), applied or refused restore or purge of a deleted character.</summary>
    public sealed class MagAdminCharacterResult
    {
        public MagAdminRefusal Refusal { get; init; }
        public string Message { get; init; }
        public AdminCharacterChange Change { get; init; }
        public string Code => MagAdminSetLevelResult.CodeOf(Refusal);
    }

    /// <summary>The live character an operator request names, or why it names none.</summary>
    public sealed record MagAdminTarget(StoredCharacter Character, MagAdminRefusal Refusal, string Message);

    /// <summary>
    /// Account and character maintenance shared by <c>mag-admin</c> and the internal admin API.
    /// The rules live here and in <see cref="LocalCharacterStore.RaiseLevelForAdmin"/>; callers only
    /// format the results.
    /// </summary>
    public static class MagAdminOperations
    {
        public const string MarkerDirectoryVariable = "MAG_ACTIVE_ACCOUNTS_DIR";

        /// <summary>Directory holding the session (lease) markers each medius process publishes.</summary>
        public static string MarkerDirectory() => Environment.GetEnvironmentVariable(MarkerDirectoryVariable) ?? "/tmp";

        /// <summary>Backups of the store are written to <c>.maintenance-backups</c> beside it.</summary>
        public static string BackupDirectory(string storePath) =>
            Path.Combine(Path.GetDirectoryName(Path.GetFullPath(storePath)), ".maintenance-backups");

        public static string Session(IReadOnlyDictionary<string, DateTime> active, string account) =>
            active == null ? "unknown" : active.ContainsKey(account) ? "online" : "offline";

        /// <summary>Every character, deleted ones included: by account, live before deleted, then by slot.</summary>
        public static IReadOnlyList<MagAdminAccount> ListAccounts(LocalCharacterStore store, string markerDirectory, DateTime now)
        {
            var active = MagActiveAccounts.Read(markerDirectory, now);
            return Ordered(store.ReadEveryCharacter()).Select(c => Describe(store, c, active)).ToArray();
        }

        static IEnumerable<StoredCharacter> Ordered(IEnumerable<StoredCharacter> characters) => characters
            .OrderBy(c => c.Account, StringComparer.Ordinal).ThenBy(c => c.Deleted).ThenBy(c => c.Slot).ThenBy(c => c.Id);

        /// <summary>The account's live character in <paramref name="slot"/>, else its lowest live slot; null when it has none.</summary>
        public static MagAdminAccount Show(LocalCharacterStore store, string account, string markerDirectory, DateTime now, int? slot = null)
        {
            var live = store.ReadForAccount(account);
            var character = slot is int wanted ? live.SingleOrDefault(c => c.Slot == wanted) : live.FirstOrDefault();
            return character == null ? null : Describe(store, character, MagActiveAccounts.Read(markerDirectory, now));
        }

        /// <summary>Every character of the account, deleted ones included, live first and by slot.</summary>
        public static IReadOnlyList<MagAdminAccount> Characters(LocalCharacterStore store, string account, string markerDirectory, DateTime now)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account)) return Array.Empty<MagAdminAccount>();
            var active = MagActiveAccounts.Read(markerDirectory, now);
            return Ordered(store.ReadEveryCharacter().Where(c => c.Account == account)).Select(c => Describe(store, c, active)).ToArray();
        }

        /// <summary>
        /// The live character an operator names. With <paramref name="slot"/>, <paramref name="target"/> is an
        /// account and the character is the one in that slot. Without it, an account with one live character
        /// names that one (as before slots), an account with several is ambiguous, and anything else is
        /// looked up as a character name (any case).
        /// </summary>
        public static MagAdminTarget Resolve(LocalCharacterStore store, string target, int? slot)
        {
            MagAdminTarget Missing() => new MagAdminTarget(null, MagAdminRefusal.NotFound,
                slot is int n ? $"account-not-found: account {target} has no character in slot {n}" : $"account-not-found: no account or character {target}");
            if (string.IsNullOrEmpty(target)) return Missing();
            var live = store.ReadForAccount(target);
            if (slot is int wanted)
                return live.SingleOrDefault(c => c.Slot == wanted) is { } inSlot ? new MagAdminTarget(inSlot, MagAdminRefusal.None, null) : Missing();
            if (live.Count == 1) return new MagAdminTarget(live[0], MagAdminRefusal.None, null);
            if (live.Count > 1)
                return new MagAdminTarget(null, MagAdminRefusal.Ambiguous, $"ambiguous-character: account {target} has {live.Count} characters (" +
                    string.Join(", ", live.Select(c => $"slot {c.Slot} {c.Name}")) + "); name one, or give its slot");
            var named = store.ReadAll().SingleOrDefault(c => string.Equals(c.Name, target, StringComparison.OrdinalIgnoreCase));
            return named != null ? new MagAdminTarget(named, MagAdminRefusal.None, null) : Missing();
        }

        private static MagAdminAccount Describe(LocalCharacterStore store, StoredCharacter c, IReadOnlyDictionary<string, DateTime> active)
        {
            var r = c.ToRecord();
            var catalog = store.ProgressionCatalog;
            bool next = catalog != null && r.Level < catalog.MaximumLevel;
            return new MagAdminAccount
            {
                Account = c.Account, CharacterId = c.Id, Name = c.Name, Faction = c.Faction, Slot = c.Slot,
                DeletedAt = c.DeletedAt is long deleted ? DateTimeOffset.FromUnixTimeSeconds(deleted) : null,
                Level = r.Level, Experience = r.Experience, SkillPoints = r.SkillPoints,
                Session = Session(active, c.Account), HasProgressionCatalog = catalog != null,
                NextLevel = next ? (byte)(r.Level + 1) : null,
                NextLevelExperience = next ? catalog.ThresholdForLevel((byte)(r.Level + 1)) : null
            };
        }

        /// <summary>
        /// Raise the character <paramref name="target"/> names (see <see cref="Resolve"/>) to <paramref name="level"/>
        /// as earned progression would. Writing (<paramref name="apply"/>) needs a session marker and no live
        /// lease for the character's account; a dry run proceeds with <see cref="MagAdminSetLevelResult.OnlineWarning"/> instead.
        /// </summary>
        public static MagAdminSetLevelResult SetLevel(LocalCharacterStore store, string target, int level, bool apply,
            string backupDirectory, string markerDirectory, DateTime now, int? slot = null)
        {
            var resolved = Resolve(store, target, slot);
            if (resolved.Refusal != MagAdminRefusal.None) return Refuse(resolved.Refusal, resolved.Message);
            string account = resolved.Character.Account;
            var active = MagActiveAccounts.Read(markerDirectory, now);
            if (apply && active == null)
                return Refuse(MagAdminRefusal.SessionUnknown, $"no session marker in {markerDirectory}; cannot confirm {account} is logged out");
            bool online = active != null && active.ContainsKey(account);
            if (online && apply)
                return Refuse(MagAdminRefusal.Online, $"{account} has a live login (lease until {active[account]:O}); log it out and retry");
            if (level < byte.MinValue || level > byte.MaxValue)
                return store.ProgressionCatalog == null
                    ? Refuse(MagAdminRefusal.NoProgressionCatalog, "progression-catalog-unavailable: the store has no progression catalog")
                    : Refuse(MagAdminRefusal.LevelOutOfRange, $"level-out-of-range: level must be from 1 to {store.ProgressionCatalog.MaximumLevel}");
            var change = store.RaiseLevelForAdmin(account, resolved.Character.Id, (byte)level, apply, backupDirectory);
            if (!change.Refusal && change.Reason == "unchanged")
                return new MagAdminSetLevelResult { Refusal = MagAdminRefusal.SameLevel, OnlineWarning = online, Change = change,
                    Message = $"unchanged: {account} is already level {change.Level}" };
            if (!change.Refusal)
                return new MagAdminSetLevelResult { Refusal = MagAdminRefusal.None, OnlineWarning = online, Change = change };
            return change.Reason switch
            {
                "progression-catalog-unavailable" => Refuse(MagAdminRefusal.NoProgressionCatalog, "progression-catalog-unavailable: the store has no progression catalog"),
                "level-out-of-range" => Refuse(MagAdminRefusal.LevelOutOfRange, $"level-out-of-range: level must be from 1 to {store.ProgressionCatalog.MaximumLevel}"),
                "lowering-unsupported" => Refuse(MagAdminRefusal.LowerLevel,
                    $"lowering-unsupported: {account} is level {change.PreviousLevel}; lowering is refused because skill points may already be spent", change),
                "account-not-found" or "invalid-account" => Refuse(MagAdminRefusal.NotFound, $"account-not-found: account {account} has no such character"),
                _ => Refuse(MagAdminRefusal.Failed, change.Reason, change)
            };
        }

        private static MagAdminSetLevelResult Refuse(MagAdminRefusal refusal, string message, AdminLevelChange change = null) =>
            new MagAdminSetLevelResult { Refusal = refusal, Message = message, Change = change };

        /// <summary>Bring the account's deleted character <paramref name="name"/> back into its lowest free slot.</summary>
        public static MagAdminCharacterResult Restore(LocalCharacterStore store, string account, string name, bool apply, string backupDirectory) =>
            Describe(store.RestoreCharacterForAdmin(account, name, apply, backupDirectory), account, name);

        /// <summary>Remove the account's deleted character <paramref name="name"/> for good, releasing its name.</summary>
        public static MagAdminCharacterResult Purge(LocalCharacterStore store, string account, string name, bool apply, string backupDirectory) =>
            Describe(store.PurgeCharacterForAdmin(account, name, apply, backupDirectory), account, name);

        static MagAdminCharacterResult Describe(AdminCharacterChange change, string account, string name)
        {
            if (!change.Refusal) return new MagAdminCharacterResult { Refusal = MagAdminRefusal.None, Change = change };
            var (refusal, message) = change.Reason switch
            {
                "account-not-found" => (MagAdminRefusal.NotFound, $"account-not-found: account {account} has no character named {name}"),
                "character-not-deleted" => (MagAdminRefusal.NotDeleted, $"character-not-deleted: {change.Name ?? name} of {account} is not deleted"),
                "ambiguous-character" => (MagAdminRefusal.Ambiguous,
                    $"ambiguous-character: several deleted characters of {account} were named {name}; give the deleted name (<name>~<id>)"),
                "character-slots-full" => (MagAdminRefusal.SlotsFull,
                    $"character-slots-full: every slot of {account} holds a live character; the player deletes one first"),
                _ => (MagAdminRefusal.Failed, change.Reason)
            };
            return new MagAdminCharacterResult { Refusal = refusal, Message = message, Change = change };
        }
    }
}
