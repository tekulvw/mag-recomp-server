using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;

namespace Server.Medius
{
    /// <summary>One account's character as the operator tools report it.</summary>
    public sealed class MagAdminAccount
    {
        public string Account { get; init; }
        public uint CharacterId { get; init; }
        public string Name { get; init; }
        public int Faction { get; init; }
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
    public enum MagAdminRefusal { None, NotFound, Online, SessionUnknown, LowerLevel, SameLevel, LevelOutOfRange, NoProgressionCatalog, Failed }

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
        public string Code => Refusal switch
        {
            MagAdminRefusal.NotFound => "not_found",
            MagAdminRefusal.Online => "online",
            MagAdminRefusal.SessionUnknown => "session_unknown",
            MagAdminRefusal.LowerLevel => "lower_level",
            MagAdminRefusal.SameLevel => "same_level",
            MagAdminRefusal.LevelOutOfRange => "level_out_of_range",
            MagAdminRefusal.NoProgressionCatalog => "no_progression_catalog",
            MagAdminRefusal.Failed => "failed",
            _ => null
        };
    }

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

        public static IReadOnlyList<MagAdminAccount> ListAccounts(LocalCharacterStore store, string markerDirectory, DateTime now)
        {
            var active = MagActiveAccounts.Read(markerDirectory, now);
            return store.ReadAll().OrderBy(c => c.Id).Select(c => Describe(store, c, active)).ToArray();
        }

        /// <summary>One account's character, or null when the account has none.</summary>
        public static MagAdminAccount Show(LocalCharacterStore store, string account, string markerDirectory, DateTime now)
        {
            var character = store.ReadAll().SingleOrDefault(x => x.Account == account);
            return character == null ? null : Describe(store, character, MagActiveAccounts.Read(markerDirectory, now));
        }

        private static MagAdminAccount Describe(LocalCharacterStore store, StoredCharacter c, IReadOnlyDictionary<string, DateTime> active)
        {
            var r = c.ToRecord();
            var catalog = store.ProgressionCatalog;
            bool next = catalog != null && r.Level < catalog.MaximumLevel;
            return new MagAdminAccount
            {
                Account = c.Account, CharacterId = c.Id, Name = c.Name, Faction = c.Faction,
                Level = r.Level, Experience = r.Experience, SkillPoints = r.SkillPoints,
                Session = Session(active, c.Account), HasProgressionCatalog = catalog != null,
                NextLevel = next ? (byte)(r.Level + 1) : null,
                NextLevelExperience = next ? catalog.ThresholdForLevel((byte)(r.Level + 1)) : null
            };
        }

        /// <summary>
        /// Raise <paramref name="account"/> to <paramref name="level"/> as earned progression would.
        /// Writing (<paramref name="apply"/>) needs a session marker and no live lease for the account;
        /// a dry run proceeds with <see cref="MagAdminSetLevelResult.OnlineWarning"/> instead.
        /// </summary>
        public static MagAdminSetLevelResult SetLevel(LocalCharacterStore store, string account, int level, bool apply,
            string backupDirectory, string markerDirectory, DateTime now)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || !store.ReadAll().Any(c => c.Account == account))
                return Refuse(MagAdminRefusal.NotFound, $"account-not-found: account {account} has no character");
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
            var change = store.RaiseLevelForAdmin(account, (byte)level, apply, backupDirectory);
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
                "account-not-found" or "invalid-account" => Refuse(MagAdminRefusal.NotFound, $"account-not-found: account {account} has no character"),
                _ => Refuse(MagAdminRefusal.Failed, change.Reason, change)
            };
        }

        private static MagAdminSetLevelResult Refuse(MagAdminRefusal refusal, string message, AdminLevelChange change = null) =>
            new MagAdminSetLevelResult { Refusal = refusal, Message = message, Change = change };
    }
}
