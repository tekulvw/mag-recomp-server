using System;
using System.Linq;
using RT.Models;

namespace Server.Medius
{
    public static class MagConsoleLinkMessages
    {
        /// <summary>
        /// Existing SCERT system-message framing, delivered only on the verified console's channel.
        /// Stock MAG display is a hardware acceptance gate, not established by serialization tests.
        /// </summary>
        public static RT_MSG_SERVER_SYSTEM_MESSAGE Create(string code)
        {
            if (code == null || code.Length != 32 || code.Any(c => !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f')))
                throw new ArgumentException("Invalid console proof code.");
            return new RT_MSG_SERVER_SYSTEM_MESSAGE
            {
                Severity = 0, EncodingType = 1, LanguageType = 2, EndOfMessage = true, RedactMessage = true,
                Message = "Link this console at your MAG portal's /account/console page. Code: " + code + ". Expires within five minutes."
            };
        }
    }
}
