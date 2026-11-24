using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using RT.Models.MAG;

namespace Server.Medius
{
    // Verifies portal game tickets (docs/portal-ticket.md) carried in the NP authenticate message.
    // Off by default: with MAG_PORTAL_TICKETS unset or "off", nothing here is constructed.

    public enum MagPortalTicketMode { Off, Required }

    /// <summary>Why a ticket exchange stopped. Only these classes are logged; never ticket bytes.</summary>
    public enum MagPortalTicketReason
    {
        Accepted, Pending, Malformed, UnknownKey, BadSignature, BadLifetime, NotYetValid, Expired,
        WrongAudience, BadIdentity, Replayed, ReplayCacheFull, BadChunk, BadHeader, TooManyAttempts, AccountSignedIn
    }

    public static class MagPortalTicketReasons
    {
        /// <summary>Snake-case class, matching the "expect" values of portal/tests/ticket-vectors.json.</summary>
        public static string Code(this MagPortalTicketReason reason) => reason switch
        {
            MagPortalTicketReason.Accepted => "accepted",
            MagPortalTicketReason.Pending => "pending",
            MagPortalTicketReason.Malformed => "malformed",
            MagPortalTicketReason.UnknownKey => "unknown_key",
            MagPortalTicketReason.BadSignature => "bad_signature",
            MagPortalTicketReason.BadLifetime => "bad_lifetime",
            MagPortalTicketReason.NotYetValid => "not_yet_valid",
            MagPortalTicketReason.Expired => "expired",
            MagPortalTicketReason.WrongAudience => "wrong_audience",
            MagPortalTicketReason.BadIdentity => "bad_identity",
            MagPortalTicketReason.Replayed => "replayed",
            MagPortalTicketReason.ReplayCacheFull => "replay_cache_full",
            MagPortalTicketReason.BadChunk => "bad_chunk",
            MagPortalTicketReason.BadHeader => "bad_header",
            MagPortalTicketReason.TooManyAttempts => "too_many_attempts",
            MagPortalTicketReason.AccountSignedIn => "account_signed_in",
            _ => "unknown"
        };
    }

    /// <summary>Environment configuration for portal tickets, read once at startup.</summary>
    public sealed class MagPortalTicketSettings
    {
        public const string ModeVariable = "MAG_PORTAL_TICKETS";
        public const string KeysVariable = "MAG_PORTAL_TICKET_KEYS";
        public const string AudienceVariable = "MAG_PORTAL_TICKET_AUDIENCE";
        private const string P256Oid = "1.2.840.10045.3.1.7";

        public MagPortalTicketMode Mode { get; private set; }
        /// <summary>Key ID to DER SubjectPublicKeyInfo of a P-256 public key.</summary>
        public IReadOnlyDictionary<byte, byte[]> Keys { get; private set; } = new Dictionary<byte, byte[]>();
        public string Audience { get; private set; }

        public static MagPortalTicketSettings FromEnvironment() => Load(Environment.GetEnvironmentVariable);

        /// <summary>
        /// Mode "off" (or unset) ignores the other variables. Mode "required" needs at least one
        /// key as comma-separated <c>id:base64-SPKI</c> entries and the audience host name.
        /// </summary>
        public static MagPortalTicketSettings Load(Func<string, string> read)
        {
            string mode = read(ModeVariable);
            if (string.IsNullOrEmpty(mode) || mode == "off") return new MagPortalTicketSettings { Mode = MagPortalTicketMode.Off };
            if (mode != "required") throw new ArgumentException(ModeVariable + " must be \"off\" or \"required\".");
            return new MagPortalTicketSettings
            {
                Mode = MagPortalTicketMode.Required,
                Keys = ParseKeys(read(KeysVariable)),
                Audience = ParseAudience(read(AudienceVariable))
            };
        }

        public static bool ValidAudience(string value) => value != null && value.Length >= 1 && value.Length <= 63 &&
            value.All(c => c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '.' || c == '-');

        private static string ParseAudience(string value)
        {
            if (string.IsNullOrEmpty(value))
                throw new ArgumentException(AudienceVariable + " is required when " + ModeVariable + "=required.");
            if (!ValidAudience(value))
                throw new ArgumentException(AudienceVariable + " must be 1 to 63 lowercase ASCII letters, digits, '.' or '-'.");
            return value;
        }

        private static IReadOnlyDictionary<byte, byte[]> ParseKeys(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
                throw new ArgumentException(KeysVariable + " is required when " + ModeVariable + "=required.");
            var keys = new Dictionary<byte, byte[]>();
            foreach (string raw in value.Split(','))
            {
                string entry = raw.Trim();
                int colon = entry.IndexOf(':');
                if (colon <= 0 || !byte.TryParse(entry.Substring(0, colon), NumberStyles.None, CultureInfo.InvariantCulture, out byte id))
                    throw new ArgumentException(KeysVariable + " entries must be <key ID 0-255>:<base64 SPKI>, separated by commas.");
                if (keys.ContainsKey(id)) throw new ArgumentException(KeysVariable + " lists key ID " + id + " more than once.");
                string encoded = entry.Substring(colon + 1).Trim();
                var buffer = new byte[encoded.Length];
                if (encoded.Length == 0 || !Convert.TryFromBase64String(encoded, buffer, out int length))
                    throw new ArgumentException(KeysVariable + " key ID " + id + " is not valid base64.");
                byte[] spki = buffer.AsSpan(0, length).ToArray();
                if (!IsP256PublicKey(spki)) throw new ArgumentException(KeysVariable + " key ID " + id + " is not a P-256 SubjectPublicKeyInfo.");
                keys.Add(id, spki);
            }
            return keys;
        }

        internal static bool IsP256PublicKey(byte[] spki)
        {
            try
            {
                using var ecdsa = ECDsa.Create();
                ecdsa.ImportSubjectPublicKeyInfo(spki, out int read);
                return read == spki.Length && ecdsa.KeySize == 256 && ecdsa.ExportParameters(false).Curve.Oid?.Value == P256Oid;
            }
            catch (CryptographicException) { return false; }
        }
    }

    /// <summary>The verified fields of a version 1 ticket.</summary>
    public sealed class MagPortalTicket
    {
        public byte KeyId { get; internal set; }
        public long IssuedAt { get; internal set; }
        public long ExpiresAt { get; internal set; }
        public uint AccountId { get; internal set; }
        public byte[] Nonce { get; internal set; }
        public string Audience { get; internal set; }
        /// <summary>The name as the player chose it; accounts use its lowercase form.</summary>
        public string PlayerName { get; internal set; }
    }

    /// <summary>Stateless checks 1 to 5 of docs/portal-ticket.md. Nonce use is checked by the caller.</summary>
    public sealed class MagPortalTicketVerifier
    {
        public const int SignatureBytes = 64;
        public const int MaximumSkewSeconds = 60;
        /// <summary>
        /// 12 hours, as the portal allows: a launcher reconnects with the ticket it fetched at launch,
        /// so a short lifetime turns a dropped connection into a refused sign-in.
        /// </summary>
        public const int MaximumLifetimeSeconds = 43200;
        private static readonly byte[] Magic = Encoding.ASCII.GetBytes("MAGT");
        private readonly IReadOnlyDictionary<byte, byte[]> _keys;
        private readonly byte[] _audience;

        public MagPortalTicketVerifier(IReadOnlyDictionary<byte, byte[]> keys, string audience)
        {
            if (keys == null || keys.Count == 0 || keys.Values.Any(key => !MagPortalTicketSettings.IsP256PublicKey(key)))
                throw new ArgumentException("Portal ticket verification needs at least one P-256 key.", nameof(keys));
            if (!MagPortalTicketSettings.ValidAudience(audience)) throw new ArgumentException("Invalid portal ticket audience.", nameof(audience));
            _keys = keys.ToDictionary(kv => kv.Key, kv => kv.Value.ToArray());
            _audience = Encoding.ASCII.GetBytes(audience);
        }

        public MagPortalTicketReason Verify(byte[] bytes, long nowUnixSeconds, out MagPortalTicket ticket) =>
            Verify(bytes, nowUnixSeconds, out ticket, out _);

        /// <summary>
        /// As <see cref="Verify(byte[], long, out MagPortalTicket)"/>; <paramref name="signed"/> also
        /// holds the fields of a ticket whose signature verified when a later check refuses it, for
        /// staff logs only. Its <see cref="MagPortalTicket.PlayerName"/> is null unless the name
        /// follows the name rule.
        /// </summary>
        public MagPortalTicketReason Verify(byte[] bytes, long nowUnixSeconds, out MagPortalTicket ticket, out MagPortalTicket signed)
        {
            ticket = null;
            signed = null;
            // 1. Exact length, magic, version and field length ranges.
            if (bytes == null || bytes.Length < 43) return MagPortalTicketReason.Malformed;
            int audienceLength = bytes[42];
            if (audienceLength < 1 || audienceLength > 63 || bytes.Length < 44 + audienceLength) return MagPortalTicketReason.Malformed;
            int nameLength = bytes[43 + audienceLength];
            if (nameLength < 3 || nameLength > 15) return MagPortalTicketReason.Malformed;
            int signedLength = 44 + audienceLength + nameLength;
            if (bytes.Length != signedLength + SignatureBytes || !bytes.AsSpan(0, 4).SequenceEqual(Magic) || bytes[4] != 1)
                return MagPortalTicketReason.Malformed;

            // 2. Known key and a valid signature (either s form) over the signed prefix.
            if (!_keys.TryGetValue(bytes[5], out var spki)) return MagPortalTicketReason.UnknownKey;
            bool valid;
            using (var ecdsa = ECDsa.Create())
            {
                ecdsa.ImportSubjectPublicKeyInfo(spki, out _);
                valid = ecdsa.VerifyData(bytes.AsSpan(0, signedLength), bytes.AsSpan(signedLength, SignatureBytes),
                    HashAlgorithmName.SHA256, DSASignatureFormat.IeeeP1363FixedFieldConcatenation);
            }
            if (!valid) return MagPortalTicketReason.BadSignature;

            // The portal signed these fields; checks 3 to 5 decide whether they are acceptable now.
            ulong issued = ReadUInt64(bytes, 6), expires = ReadUInt64(bytes, 14);
            uint accountId = (uint)(bytes[22] << 24 | bytes[23] << 16 | bytes[24] << 8 | bytes[25]);
            var name = bytes.AsSpan(44 + audienceLength, nameLength);
            bool nameValid = IsLetter(name[0]) && name.ToArray().All(c => IsLetter(c) || c >= '0' && c <= '9' || c == '-' || c == '_');
            signed = new MagPortalTicket
            {
                KeyId = bytes[5], IssuedAt = (long)Math.Min(issued, long.MaxValue), ExpiresAt = (long)Math.Min(expires, long.MaxValue),
                AccountId = accountId, Nonce = bytes.AsSpan(26, 16).ToArray(), Audience = Encoding.ASCII.GetString(bytes, 43, audienceLength),
                PlayerName = nameValid ? Encoding.ASCII.GetString(name) : null
            };

            // 3. Time window.
            if (issued > long.MaxValue || expires > long.MaxValue || expires <= issued || expires - issued > MaximumLifetimeSeconds)
                return MagPortalTicketReason.BadLifetime;
            if ((long)issued > nowUnixSeconds + MaximumSkewSeconds) return MagPortalTicketReason.NotYetValid;
            if ((long)expires <= nowUnixSeconds) return MagPortalTicketReason.Expired;

            // 4. Audience equals this server.
            if (!bytes.AsSpan(43, audienceLength).SequenceEqual(_audience)) return MagPortalTicketReason.WrongAudience;

            // 5. Account ID and name rule.
            if (accountId == 0 || !nameValid) return MagPortalTicketReason.BadIdentity;

            ticket = signed;
            return MagPortalTicketReason.Accepted;
        }

        private static bool IsLetter(byte c) => c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z';
        private static ulong ReadUInt64(byte[] bytes, int offset)
        {
            ulong value = 0;
            for (int i = 0; i < 8; i++) value = value << 8 | bytes[offset + i];
            return value;
        }
    }

    /// <summary>
    /// Uses of accepted nonces, each kept until its ticket expires. The game reuses one ticket for
    /// several sign-ins, so a nonce may be used <see cref="MaximumUses"/> times. Process-local and
    /// thread-safe; MAPS and MAS run as separate processes, so the limits apply per service.
    /// Memory is bounded by the capacity: when full of unexpired nonces it refuses new tickets
    /// rather than forgetting old ones, and one account can hold only
    /// <see cref="MaximumNoncesPerAccount"/> of them, so no single account can fill it.
    /// </summary>
    public sealed class MagPortalNonceCache
    {
        /// <summary>
        /// Sign-ins one ticket may open on one service. The game sends the ticket it fetched at launch
        /// to MAPS, MAS and the statistics login, and again on every reconnect, for up to the 12-hour
        /// ticket lifetime. This count only stops unbounded reuse: the main protection against a
        /// shared or leaked ticket is that it never opens a second live session for an account that
        /// is already signed in (<see cref="MagPortalLiveSessions"/>).
        /// </summary>
        public const int MaximumUses = 32;

        /// <summary>
        /// Nonces held at once, at about 50 bytes each (some 3 MB when full). Every launch fetches one
        /// ticket, so a few thousand tickets in a 12-hour window leave room to spare.
        /// </summary>
        public const int DefaultCapacity = 65536;

        /// <summary>
        /// Unexpired nonces one account may hold: a fresh ticket every six minutes for 12 hours. Above
        /// it, that account's new tickets are refused until its old ones expire; other accounts are
        /// unaffected. It takes 512 accounts doing this to fill <see cref="DefaultCapacity"/>.
        /// </summary>
        public const int MaximumNoncesPerAccount = 128;

        /// <summary>Expired nonces are swept at least this often, and at most once a second.</summary>
        public const int SweepIntervalSeconds = 60;

        private readonly object _gate = new object();
        private readonly Dictionary<(ulong, ulong), (long Expiry, int Uses, uint Account)> _entries =
            new Dictionary<(ulong, ulong), (long, int, uint)>();
        private readonly Dictionary<uint, int> _perAccount = new Dictionary<uint, int>();
        private readonly int _capacity;
        private long _lastSweep = long.MinValue;

        public MagPortalNonceCache(int capacity = DefaultCapacity)
        {
            if (capacity < 1) throw new ArgumentOutOfRangeException(nameof(capacity));
            _capacity = capacity;
        }

        public int Count { get { lock (_gate) return _entries.Count; } }

        /// <summary>Nonces held for <paramref name="accountId"/>, including expired ones not yet swept.</summary>
        public int CountFor(uint accountId) { lock (_gate) return _perAccount.TryGetValue(accountId, out int n) ? n : 0; }

        /// <summary>
        /// Spends one use of the nonce: Replayed once it has been used <see cref="MaximumUses"/> times,
        /// ReplayCacheFull when a new nonce does not fit.
        /// </summary>
        public MagPortalTicketReason TryUse(byte[] nonce, uint accountId, long expiresAt, long nowUnixSeconds)
        {
            if (nonce == null || nonce.Length != 16) throw new ArgumentException("A nonce is 16 bytes.", nameof(nonce));
            var key = (BitConverter.ToUInt64(nonce, 0), BitConverter.ToUInt64(nonce, 8));
            lock (_gate)
            {
                if (_entries.TryGetValue(key, out var kept))
                {
                    if (kept.Expiry > nowUnixSeconds)
                    {
                        if (kept.Uses >= MaximumUses) return MagPortalTicketReason.Replayed;
                        _entries[key] = (Math.Max(kept.Expiry, expiresAt), kept.Uses + 1, kept.Account);
                        return MagPortalTicketReason.Accepted;
                    }
                    Remove(key, kept.Account);
                }
                bool crowded = _entries.Count >= _capacity || CountLocked(accountId) >= MaximumNoncesPerAccount;
                if (nowUnixSeconds > _lastSweep && (crowded || nowUnixSeconds >= _lastSweep + SweepIntervalSeconds)) Sweep(nowUnixSeconds);
                if (_entries.Count >= _capacity || CountLocked(accountId) >= MaximumNoncesPerAccount) return MagPortalTicketReason.ReplayCacheFull;
                _entries[key] = (expiresAt, 1, accountId);
                _perAccount[accountId] = CountLocked(accountId) + 1;
                return MagPortalTicketReason.Accepted;
            }
        }

        private int CountLocked(uint accountId) => _perAccount.TryGetValue(accountId, out int n) ? n : 0;

        private void Remove((ulong, ulong) key, uint accountId)
        {
            _entries.Remove(key);
            int left = CountLocked(accountId) - 1;
            if (left > 0) _perAccount[accountId] = left; else _perAccount.Remove(accountId);
        }

        private void Sweep(long nowUnixSeconds)
        {
            _lastSweep = nowUnixSeconds;
            foreach (var stale in _entries.Where(kv => kv.Value.Expiry <= nowUnixSeconds).ToArray()) Remove(stale.Key, stale.Value.Account);
        }
    }

    /// <summary>A MAPS or MAS session that can hold a portal account.</summary>
    public interface IMagPortalSession
    {
        /// <summary>True while the session is connected, signed in and not idle past its timeout at <paramref name="now"/>.</summary>
        bool IsPortalSessionLive(DateTime now);
    }

    /// <summary>
    /// Which session holds each ticket-authenticated account on this service. A ticket never opens
    /// a second live session for an account; a session that ended or went idle frees the account.
    /// </summary>
    public sealed class MagPortalLiveSessions
    {
        private readonly object _gate = new object();
        private readonly Dictionary<string, IMagPortalSession> _owners = new Dictionary<string, IMagPortalSession>(StringComparer.Ordinal);

        public int Count { get { lock (_gate) return _owners.Count; } }

        public bool TryClaim(string account, IMagPortalSession owner, DateTime now)
        {
            lock (_gate)
            {
                if (_owners.TryGetValue(account, out var current) && !ReferenceEquals(current, owner) && current.IsPortalSessionLive(now)) return false;
                if (_owners.Count >= 4096)
                    foreach (var stale in _owners.Where(kv => !kv.Value.IsPortalSessionLive(now)).Select(kv => kv.Key).ToArray()) _owners.Remove(stale);
                _owners[account] = owner;
                return true;
            }
        }

        public void Release(IMagPortalSession owner)
        {
            if (owner == null) return;
            lock (_gate)
                foreach (var key in _owners.Where(kv => ReferenceEquals(kv.Value, owner)).Select(kv => kv.Key).ToArray()) _owners.Remove(key);
        }
    }

    /// <summary>
    /// Per-session reassembly of a ticket sent in NP authenticate chunks. Every chunk repeats the
    /// same full size; chunk sizes must add up to it exactly, with the last flag on the final one.
    /// Any inconsistency discards the partial ticket.
    /// </summary>
    public sealed class MagPortalTicketAssembly
    {
        public const int MaximumTicketBytes = 2048;
        public const int MaximumChunks = 8;
        private byte[] _buffer;
        private int _received;
        private int _chunks;

        public int Failures { get; internal set; }
        public bool InProgress => _buffer != null;

        public void Reset() { _buffer = null; _received = 0; _chunks = 0; }

        /// <summary>Returns Pending, Accepted with the complete ticket, or BadChunk.</summary>
        public MagPortalTicketReason Offer(MagNpAuthenticateRequest request, out byte[] ticket)
        {
            ticket = null;
            int size = request.TicketChunkSize, full = request.FullTicketSize;
            var chunk = request.TicketChunk;
            bool consistent = chunk != null && size >= 1 && size <= chunk.Length && full >= 1 && full <= MaximumTicketBytes &&
                chunk.AsSpan(size).IndexOfAnyExcept((byte)0) < 0 && (_buffer == null || _buffer.Length == full) &&
                _received + size <= full && (request.LastMessage ? _received + size == full : _received + size < full) &&
                _chunks < MaximumChunks;
            if (!consistent) { Reset(); return MagPortalTicketReason.BadChunk; }
            _buffer ??= new byte[full];
            chunk.AsSpan(0, size).CopyTo(_buffer.AsSpan(_received));
            _received += size; _chunks++;
            if (!request.LastMessage) return MagPortalTicketReason.Pending;
            ticket = _buffer;
            Reset();
            return MagPortalTicketReason.Accepted;
        }
    }

    public sealed class MagPortalAuthentication
    {
        public MagPortalTicketReason Reason { get; internal set; }
        public bool Accepted => Reason == MagPortalTicketReason.Accepted;
        /// <summary>True when the exchange ended without a sign-in (not accepted, not waiting for chunks).</summary>
        public bool Refused => Reason != MagPortalTicketReason.Accepted && Reason != MagPortalTicketReason.Pending;
        /// <summary>Lowercase player name: the key of existing local characters.</summary>
        public string Account { get; internal set; }
        public uint AccountId { get; internal set; }
        /// <summary>
        /// For a refused ticket whose signature verified: its signed fields, for staff logs. Null when
        /// the ticket was accepted, or refused before its signature could be trusted.
        /// </summary>
        public MagPortalTicket RefusedTicket { get; internal set; }
        /// <summary>The verifier's clock, Unix seconds, when the ticket was checked; 0 before that.</summary>
        public long CheckedAt { get; internal set; }

        /// <summary>
        /// One log line naming why a ticket was refused and, when the portal signed it, whose it was
        /// and when it expires. Never carries ticket bytes or the nonce. Null unless <see cref="Refused"/>.
        /// </summary>
        public string RefusalLog()
        {
            if (!Refused) return null;
            var line = new StringBuilder("portal ticket refused: reason=").Append(Reason.Code());
            var t = RefusedTicket;
            if (t != null)
            {
                line.Append(" account=").Append(t.PlayerName == null ? "invalid-name" : t.PlayerName.ToLowerInvariant())
                    .Append(" account_id=").Append(t.AccountId.ToString(CultureInfo.InvariantCulture))
                    .Append(" issued_at=").Append(Iso(t.IssuedAt)).Append(" expires_at=").Append(Iso(t.ExpiresAt))
                    .Append(" age_s=").Append((CheckedAt - t.IssuedAt).ToString(CultureInfo.InvariantCulture));
            }
            return line.ToString();
        }

        private static string Iso(long unixSeconds) =>
            unixSeconds >= 0 && unixSeconds <= 253402300799
                ? DateTime.UnixEpoch.AddSeconds(unixSeconds).ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture)
                : unixSeconds.ToString(CultureInfo.InvariantCulture);
    }

    /// <summary>Required-mode sign-in: reassemble, verify, then spend the nonce.</summary>
    public sealed class MagPortalTicketPolicy
    {
        public const int MaximumFailuresPerSession = 3;
        private readonly MagPortalTicketVerifier _verifier;
        private readonly Func<DateTime> _clock;

        public MagPortalTicketSettings Settings { get; }
        public MagPortalNonceCache Nonces { get; }
        public MagPortalLiveSessions Sessions { get; } = new MagPortalLiveSessions();

        public MagPortalTicketPolicy(MagPortalTicketSettings settings, Func<DateTime> clock = null, MagPortalNonceCache nonces = null)
        {
            if (settings == null || settings.Mode != MagPortalTicketMode.Required) throw new ArgumentException("Portal ticket policy needs required mode.", nameof(settings));
            Settings = settings;
            _verifier = new MagPortalTicketVerifier(settings.Keys, settings.Audience);
            _clock = clock ?? (() => DateTime.UtcNow);
            Nonces = nonces ?? new MagPortalNonceCache();
        }

        /// <summary>Null when tickets are off, so sessions keep their unchanged development path.</summary>
        public static MagPortalTicketPolicy FromSettings(MagPortalTicketSettings settings) =>
            settings.Mode == MagPortalTicketMode.Required ? new MagPortalTicketPolicy(settings) : null;

        /// <summary>The application's NP authenticate message ID; used to keep its payload out of logs.</summary>
        public static bool IsTicketMessage(int applicationId, int messageId) => messageId == (applicationId == 20231 ? 0x0034 : 0x0031);

        /// <summary>
        /// Verifies a chunk. On a complete valid ticket, <paramref name="owner"/> (when given) claims the
        /// account unless another live session on this service holds it, then one nonce use is spent.
        /// <paramref name="sessionNow"/> is the sessions' activity clock, used for liveness only.
        /// </summary>
        public MagPortalAuthentication Authenticate(MagPortalTicketAssembly assembly, MagNpAuthenticateRequest request,
            IMagPortalSession owner = null, DateTime sessionNow = default)
        {
            if (assembly.Failures >= MaximumFailuresPerSession) return Fail(assembly, MagPortalTicketReason.TooManyAttempts, false);
            // The plain development payload and the portal ticket share this header rule.
            if (!MagDevelopmentAccountPolicy.HasZeroRsa(request)) return Fail(assembly, MagPortalTicketReason.BadHeader, true);
            var reason = assembly.Offer(request, out var bytes);
            if (reason == MagPortalTicketReason.Pending) return new MagPortalAuthentication { Reason = reason };
            if (reason != MagPortalTicketReason.Accepted) return Fail(assembly, reason, true);
            long now = UnixSeconds(_clock());
            reason = _verifier.Verify(bytes, now, out var ticket, out var signed);
            if (reason != MagPortalTicketReason.Accepted) return Fail(assembly, reason, true, signed, now);
            string account = ticket.PlayerName.ToLowerInvariant();
            // Claim first so a refused second session spends none of the ticket's uses.
            if (owner != null && !Sessions.TryClaim(account, owner, sessionNow)) return Fail(assembly, MagPortalTicketReason.AccountSignedIn, true, ticket, now);
            reason = Nonces.TryUse(ticket.Nonce, ticket.AccountId, ticket.ExpiresAt, now);
            if (reason != MagPortalTicketReason.Accepted)
            {
                if (owner != null) Sessions.Release(owner);
                return Fail(assembly, reason, true, ticket, now);
            }
            return new MagPortalAuthentication { Reason = reason, Account = account, AccountId = ticket.AccountId, CheckedAt = now };
        }

        /// <summary>
        /// The reply to an NP authenticate message that did not sign in: a well-formed failure (the
        /// success bit clear), so the client ends its sign-in at once instead of waiting for a reply
        /// that never comes. Null when no reply is due: while chunks are still arriving, for a
        /// non-final chunk, and once a session has used up its attempts (those are ignored).
        /// </summary>
        public static MagNpAuthenticateResponse RefusalReply(MagNpAuthenticateRequest request, MagPortalAuthentication outcome)
        {
            if (request == null || outcome == null || !outcome.Refused || !request.LastMessage ||
                outcome.Reason == MagPortalTicketReason.TooManyAttempts) return null;
            MagNpAuthenticateResponse reply = request is MagRetailNpAuthenticateRequest ? new MagRetailNpAuthenticateResponse() : new MagNpAuthenticateResponse();
            reply.TransactionId = request.TransactionId;
            reply.Success = false;
            return reply;
        }

        private static MagPortalAuthentication Fail(MagPortalTicketAssembly assembly, MagPortalTicketReason reason, bool count,
            MagPortalTicket signed = null, long checkedAt = 0)
        {
            assembly.Reset();
            if (count) assembly.Failures++;
            return new MagPortalAuthentication { Reason = reason, RefusedTicket = signed, CheckedAt = checkedAt };
        }

        internal static long UnixSeconds(DateTime time)
        {
            var utc = time.Kind == DateTimeKind.Local ? time.ToUniversalTime() : DateTime.SpecifyKind(time, DateTimeKind.Utc);
            return (long)Math.Floor((utc - DateTime.UnixEpoch).TotalSeconds);
        }
    }
}
