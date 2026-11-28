using System;
using System.Linq;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>The result of a skill respec; <see cref="Record"/> is the character's record after a committed one.</summary>
    public sealed class SkillRespecOutcome
    {
        public MagNetResult Result { get; init; }
        public string Reason { get; init; }
        public uint Fee { get; init; }
        public int RefundedSkillPoints { get; init; }
        public MagCharacterRecord Record { get; init; }
    }

    public sealed partial class LocalCharacterStore
    {
        /// <summary>
        /// A retail skill respec (0x0254) for the session's character. It costs the authored fee for the
        /// character's respec count (the table's last fee once the count runs past it), and an exact balance is
        /// enough. It refunds every owned skill at its authored cost (unspent points capped at 255), clears those
        /// skills and the equipment they awarded, rebuilds the five kits from the faction's authored defaults,
        /// counts the respec (capped at 255) and debits the fee. Starter (tier 0) skills cost nothing and stay
        /// granted. The balance spent is the stored one raised to <see cref="RespecPointsFloor"/>, which then tops
        /// the remainder back up whenever the record is next built. See docs/skill-respec.md.
        /// </summary>
        public SkillRespecOutcome RespecSkillsForAccount(string account, uint characterId)
        {
            if (!MagDevelopmentAccountPolicy.ValidAccount(account) || characterId == 0) return Refused("invalid-skill-respec");
            var skills = SkillCatalog;
            var armory = LoadoutCatalog;
            if (skills == null || armory == null) return Refused("skill-catalog-unavailable");
            using var session = _backend.Open(new StoreScope(account, characterId));
            var character = Owned(session.Document.Characters, account, characterId);
            if (character == null) return Refused("character-not-owned");
            byte count = character.RespecCount ?? 0;
            if (!skills.TryRespecFee(count, out uint fee)) return Refused("respec-fees-unavailable");
            uint balance = Math.Max(character.RespecPoints ?? 0, RespecPointsFloor ?? 0);
            if (balance < fee) return Refused("insufficient-respec-points", fee);
            var defaults = armory.DefaultsForFaction(character.Faction);
            if (defaults == null) return Refused("authored-faction-defaults-unavailable", fee);

            var before = character.ToRecord();
            var starter = armory.GrantsForFaction(character.Faction)?.Skills ?? new uint[5];
            // Starter skills are granted again at once and cost nothing, so only the others are refunded.
            var owned = new uint[5];
            for (int word = 0; word < 5; word++) owned[word] = before.Skills[word] & ~starter[word];
            var (refund, equipment) = skills.RespecRefund(character.Faction, owned);

            character.SkillPoints = (byte)Math.Min(255, before.SkillPoints + refund);
            character.OwnedSkills = character.OwnedSkills == null ? null : character.OwnedSkills.Select((bits, word) => bits & ~owned[word]).ToArray();
            if (character.UnlockedItems != null)
            {
                var unlocked = (uint[])character.UnlockedItems.Clone();
                foreach (int item in equipment) unlocked[item >> 5] &= ~(1u << (item & 31));
                character.UnlockedItems = unlocked;
            }
            character.Loadouts = defaults.Select(StoredLoadout.FromLoadout).ToArray();
            // Kit positions do not move, so a pending selection still names the same kit; only a selected kit
            // left without a primary falls back to the first, which the authored defaults always fill.
            if (!armory.HasValidPrimary(character.Loadouts[character.LastUsedLoadout].ToLoadout())) character.LastUsedLoadout = 0;
            character.RespecCount = (byte)Math.Min(255, count + 1);
            character.RespecPoints = balance - fee;
            session.Commit(); // The client imports the record only after the durable commit.
            return new SkillRespecOutcome
            {
                Result = MagNetResult.Success, Reason = "skill-respec-committed", Fee = fee, RefundedSkillPoints = refund,
                Record = character.ToRecord()
            };
        }

        private static SkillRespecOutcome Refused(string reason, uint fee = 0) =>
            new SkillRespecOutcome { Result = MagNetResult.Error, Reason = reason, Fee = fee };
    }
}
