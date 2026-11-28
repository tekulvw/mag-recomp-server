using System;
using System.Buffers.Binary;
using System.Collections.Generic;
using System.Formats.Asn1;
using System.Globalization;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using Org.BouncyCastle.Asn1;
using Org.BouncyCastle.Asn1.X509;
using Org.BouncyCastle.Crypto.Parameters;
using Org.BouncyCastle.Security;

namespace Server.Medius
{
    /// <summary>Identity exposed only after complete Sony NP verification.</summary>
    public sealed class MagPsnTicket
    {
        public string Subject { get; internal set; }
        public string OnlineId { get; internal set; }
        public string ServiceId { get; internal set; }
        /// <summary>Exact signed issue time for admission decisions.</summary>
        public long IssuedAtMilliseconds { get; internal set; }
        /// <summary>Exact signed expiry for admission and proof expiry caps.</summary>
        public long ExpiresAtMilliseconds { get; internal set; }
        /// <summary>Ceiling Unix seconds for replay-cache retention only; never an admission or proof deadline.</summary>
        public long ExpiresAt { get; internal set; }
        public byte[] ReplayNonce { get; internal set; }
    }

    /// <summary>
    /// Strict NP 2.1 release-network verifier. No keys or game service IDs are built in.
    /// Supports the fixed-width, empty-entitlement 248-byte structure only; new variants
    /// require evidence and an explicit parser change, never heuristic field searches.
    /// Public format references (AGPL-3.0 upstream, consulted as protocol documentation;
    /// this parser is independently implemented, no upstream code is incorporated):
    /// https://github.com/LBPUnion/ProjectLighthouse/blob/ac9dc7bcbf87bf5478d3f53cd7e4d2567cb7414e/Documentation/Tickets.md
    /// https://github.com/LBPUnion/ProjectLighthouse/blob/ac9dc7bcbf87bf5478d3f53cd7e4d2567cb7414e/ProjectLighthouse/Tickets/Parser/TicketParser21.cs
    /// https://github.com/LBPUnion/ProjectLighthouse/blob/ac9dc7bcbf87bf5478d3f53cd7e4d2567cb7414e/ProjectLighthouse/Tickets/Signature/PsnSignatureVerifier.cs
    /// These describe LBP tickets; they do not establish MAG's issuer/key/service support.
    /// </summary>
    public sealed class MagPsnTicketVerifier
    {
        public const int MaximumTicketBytes = 248;
        public const int MaximumLifetimeSeconds = 43200;
        private const int SignatureOffset = 192;
        private readonly Dictionary<string, ECPublicKeyParameters> _keys = new(StringComparer.Ordinal);
        private readonly HashSet<string> _services;

        public MagPsnTicketVerifier(IReadOnlyDictionary<string, byte[]> keys, IReadOnlyCollection<string> services)
        {
            if (keys == null || keys.Count == 0) throw new ArgumentException("Sony verification requires explicit public keys.", nameof(keys));
            foreach (var entry in keys)
            {
                if (entry.Key == null || entry.Key.Length != 8 || !entry.Key.All(c => c >= '0' && c <= '9' || c >= 'a' && c <= 'f'))
                    throw new ArgumentException("Sony key IDs must be eight lowercase hexadecimal characters.", nameof(keys));
                try
                {
                    // DER, named secp192r1 EC public key only. Reject explicit curves, private keys,
                    // trailing ASN.1, and keys accepted by a platform-specific permissive importer.
                    if (entry.Value == null || entry.Value.Length > 256) throw new ArgumentException();
                    var obj = Asn1Object.FromByteArray(entry.Value);
                    var info = SubjectPublicKeyInfo.GetInstance(obj);
                    if (!entry.Value.AsSpan().SequenceEqual(info.GetDerEncoded()) ||
                        info.AlgorithmID.Algorithm.Id != "1.2.840.10045.2.1" ||
                        !(info.AlgorithmID.Parameters is DerObjectIdentifier curve) || curve.Id != "1.2.840.10045.3.1.1" ||
                        !(PublicKeyFactory.CreateKey(info) is ECPublicKeyParameters publicKey) || publicKey.IsPrivate)
                        throw new ArgumentException();
                    _keys.Add(entry.Key, publicKey);
                }
                catch (Exception e) when (e is not OutOfMemoryException)
                {
                    // Configuration errors contain no key bytes or imported ASN.1 error details.
                    throw new ArgumentException("Sony keys must be DER SPKI named secp192r1 EC public keys.", nameof(keys));
                }
            }
            if (services == null || services.Count == 0 || services.Any(service => !ValidService(service)))
                throw new ArgumentException("Sony verification requires explicit ASCII service IDs of 1 to 24 characters.", nameof(services));
            _services = new HashSet<string>(services, StringComparer.Ordinal);
        }

        /// <summary>Compatibility API; the supplied clock denotes an exact integral Unix second.</summary>
        public MagPortalTicketReason Verify(byte[] bytes, long nowUnixSeconds, out MagPsnTicket ticket)
        {
            ticket = null;
            if (nowUnixSeconds < 0 || nowUnixSeconds > long.MaxValue / 1000)
                return MagPortalTicketReason.Malformed;
            return VerifyAtMilliseconds(bytes, checked(nowUnixSeconds * 1000), out ticket);
        }

        /// <summary>Production admission API: compares the clock with the exact signed millisecond times.</summary>
        public MagPortalTicketReason VerifyAtMilliseconds(byte[] bytes, long nowUnixMilliseconds, out MagPsnTicket ticket)
        {
            ticket = null;
            if (bytes == null || bytes.Length != MaximumTicketBytes || nowUnixMilliseconds < 0)
                return MagPortalTicketReason.Malformed;
            ReadOnlySpan<byte> data = bytes;
            if (!data.Slice(0, 4).SequenceEqual(new byte[] { 0x21, 1, 0, 0 }) || U32(data, 4) != 240 ||
                !Header(data, 8, 0x3000, 164) || !Header(data, 12, 8, 20) ||
                !Header(data, 36, 1, 4) || !Header(data, 44, 7, 8) || !Header(data, 56, 7, 8) ||
                !Header(data, 68, 2, 8) || !Header(data, 80, 4, 32) ||
                !Header(data, 116, 8, 4) || !Header(data, 124, 8, 4) || !Header(data, 132, 8, 24) ||
                !Header(data, 160, 1, 4) || !Header(data, 168, 0, 0) || !Header(data, 172, 0, 0) ||
                !Header(data, 176, 0x3002, 68) || !Header(data, 180, 8, 4) || !Header(data, 188, 8, 56))
                return MagPortalTicketReason.Malformed;
            string keyId = Convert.ToHexString(data.Slice(184, 4)).ToLowerInvariant();
            if (!_keys.TryGetValue(keyId, out var key)) return MagPortalTicketReason.UnknownKey;
            if (!VerifySignature(bytes, key)) return MagPortalTicketReason.BadSignature;

            // No identity escapes on any rejection. Time fields are unsigned milliseconds.
            ulong issued = U64(data, 48), expires = U64(data, 60);
            if (issued > long.MaxValue || expires > long.MaxValue || expires <= issued || expires - issued > MaximumLifetimeSeconds * 1000UL)
                return MagPortalTicketReason.BadLifetime;
            ulong now = (ulong)nowUnixMilliseconds;
            if (issued > now) return MagPortalTicketReason.NotYetValid;
            if (expires <= now) return MagPortalTicketReason.Expired;
            if (U32(data, 40) != 256 || U64(data, 72) == 0 || !ReadOnlineId(data.Slice(84, 32), out string onlineId))
                return MagPortalTicketReason.BadIdentity;
            if (!ReadPaddedAscii(data.Slice(136, 24), out string service) || !ValidService(service))
                return MagPortalTicketReason.Malformed;
            if (!_services.Contains(service)) return MagPortalTicketReason.WrongAudience;

            ticket = new MagPsnTicket
            {
                Subject = "256:" + U64(data, 72).ToString(CultureInfo.InvariantCulture),
                OnlineId = onlineId, ServiceId = service,
                IssuedAtMilliseconds = (long)issued, ExpiresAtMilliseconds = (long)expires,
                // Round upward for replay-cache retention; expiry validation above uses exact milliseconds.
                ExpiresAt = (long)(expires / 1000 + (expires % 1000 == 0 ? 0UL : 1UL)),
                // Hash signed content rather than the malleable ECDSA encoding. Re-signing or
                // replacing s by n-s must not give the same authenticated ticket a new replay key.
                ReplayNonce = SHA256.HashData(data.Slice(0, SignatureOffset)).AsSpan(0, 16).ToArray()
            };
            return MagPortalTicketReason.Accepted;
        }

        private static bool VerifySignature(byte[] bytes, ECPublicKeyParameters key)
        {
            try
            {
                // secp192r1 signatures fit a short-form DER sequence in a 56-byte field.
                // Accept only canonical DER and zero fill after its exact end.
                if (bytes[SignatureOffset] != 0x30 || bytes[SignatureOffset + 1] > 54) return false;
                int length = bytes[SignatureOffset + 1] + 2;
                if (length < 8 || bytes.AsSpan(SignatureOffset + length).IndexOfAnyExcept((byte)0) >= 0) return false;
                var reader = new AsnReader(bytes.AsMemory(SignatureOffset, length), AsnEncodingRules.DER);
                var sequence = reader.ReadSequence();
                if (sequence.ReadInteger().Sign <= 0 || sequence.ReadInteger().Sign <= 0) return false;
                sequence.ThrowIfNotEmpty(); reader.ThrowIfNotEmpty();
                var signer = SignerUtilities.GetSigner("SHA-1withECDSA");
                signer.Init(false, key);
                signer.BlockUpdate(bytes, 0, SignatureOffset);
                return signer.VerifySignature(bytes.AsSpan(SignatureOffset, length).ToArray());
            }
            catch (Exception e) when (e is not OutOfMemoryException) { return false; }
        }

        private static bool ValidService(string service) => service != null && service.Length >= 1 && service.Length <= 24 &&
            service.All(c => c >= 'A' && c <= 'Z' || c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '-' || c == '_');

        private static bool ReadOnlineId(ReadOnlySpan<byte> field, out string value)
        {
            if (!ReadPaddedAscii(field, out value) || value.Length < 3 || value.Length > 16) return false;
            return IsLetter(value[0]) && value.All(c => IsLetter(c) || c >= '0' && c <= '9' || c == '-' || c == '_');
        }
        private static bool IsLetter(char c) => c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z';
        private static bool ReadPaddedAscii(ReadOnlySpan<byte> field, out string value)
        {
            value = null;
            int zero = field.IndexOf((byte)0);
            int length = zero < 0 ? field.Length : zero;
            if (length == 0 || zero >= 0 && field.Slice(zero).IndexOfAnyExcept((byte)0) >= 0) return false;
            for (int i = 0; i < length; i++) if (field[i] < 0x21 || field[i] > 0x7e) return false;
            value = Encoding.ASCII.GetString(field.Slice(0, length));
            return true;
        }
        private static bool Header(ReadOnlySpan<byte> data, int offset, ushort type, ushort length) =>
            BinaryPrimitives.ReadUInt16BigEndian(data.Slice(offset, 2)) == type && BinaryPrimitives.ReadUInt16BigEndian(data.Slice(offset + 2, 2)) == length;
        private static uint U32(ReadOnlySpan<byte> data, int offset) => BinaryPrimitives.ReadUInt32BigEndian(data.Slice(offset, 4));
        private static ulong U64(ReadOnlySpan<byte> data, int offset) => BinaryPrimitives.ReadUInt64BigEndian(data.Slice(offset, 8));
    }
}
