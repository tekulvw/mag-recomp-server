using System;
using System.Linq;
using System.Text;
using RT.Models.MAG;

namespace Server.Medius
{
    internal static class MagDevelopmentAccountPolicy
    {
        public static bool HasZeroRsa(MagPluginClientMessage message) => message.RsaWords.All(word => word == 0);
        public static bool ValidAccount(string value) => value != null && value.Length >= 3 && value.Length <= 16 &&
            value.All(c => c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '-' || c == '_');
        // A 36-byte NP identity is either all zero or bound to the session's account: a NUL-padded
        // 16-byte online ID equal to the account ignoring ASCII case, then zero terminator, padding,
        // opt and reserved bytes. This is a local consistency rule, not NP identity verification.
        public static bool ValidNpId(byte[] npId, string account)
        {
            if (npId == null || npId.Length != 36) return false;
            if (npId.All(value => value == 0)) return true;
            if (!ValidAccount(account) || npId.Skip(16).Any(value => value != 0)) return false;
            int length = Array.IndexOf(npId, (byte)0, 0, 16);
            if (length < 0) length = 16;
            if (npId.Skip(length).Take(16 - length).Any(value => value != 0) || npId.Take(length).Any(value => value > 127)) return false;
            return Encoding.ASCII.GetString(npId, 0, length).ToLowerInvariant() == account;
        }
        public static string Account(MagNpAuthenticateRequest request)
        {
            // This opt-in local identity policy is not a PSN ticket validator.
            if (!request.LastMessage || !HasZeroRsa(request) || request.FullTicketSize < 3 || request.FullTicketSize > 16 ||
                request.TicketChunkSize != request.FullTicketSize || request.TicketChunk == null ||
                request.TicketChunk.Length < request.TicketChunkSize || request.TicketChunk.Skip(request.TicketChunkSize).Any(value => value != 0)) return null;
            var account = Encoding.ASCII.GetString(request.TicketChunk, 0, request.TicketChunkSize).ToLowerInvariant();
            return ValidAccount(account) ? account : null;
        }
        public static bool Accepts(MagNpAuthenticateRequest request) => Account(request) != null;
    }
}
