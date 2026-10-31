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
