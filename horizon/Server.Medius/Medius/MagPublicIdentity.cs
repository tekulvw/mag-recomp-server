using System.Linq;
using System.Text;
using RT.Models.MAG;

namespace Server.Medius
{
    /// <summary>Display-only authenticated identity; never a character owner or mutation key.</summary>
    public static class MagPublicIdentity
    {
        // Keep legacy development identities (up to 16 bytes) valid as well as portal names.
        public static bool ValidName(string name) => name != null && name.Length >= 3 && name.Length <= 16 &&
            name.All(c => c >= 'A' && c <= 'Z' || c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '_' || c == '-');
        public static string DisplayName(StoredCharacter character, string publicName) => ValidName(publicName) ? publicName : character.DisplayName();
        public static MagCharacterRecord DisplayRecord(StoredCharacter character, string publicName) =>
            DisplayRecord(character.ToRecord(), publicName);
        public static MagCharacterRecord DisplayRecord(MagCharacterRecord record, string publicName)
        {
            if (record == null || !ValidName(publicName)) return record;
            var npId = new byte[36];
            Encoding.ASCII.GetBytes(publicName).CopyTo(npId, 0);
            return record.WithNpId(npId);
        }

    }
}
