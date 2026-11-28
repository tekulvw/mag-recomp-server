using System.Numerics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Medius;

// Portal ticket checks. The vectors are the portal's own synthetic test vectors (a copy of
// portal/tests/ticket-vectors.json); every other ticket and key is generated here in memory.
internal static class PortalTicketChecks
{
    static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    static void Throws(Func<object> action, string needle)
    {
        try { action(); }
        catch (ArgumentException e)
        {
            Check(e.Message.Contains(needle), "Wrong configuration error: " + e.Message);
            return;
        }
        throw new Exception("Configuration unexpectedly accepted; expected an error naming " + needle);
    }

    static JsonElement LoadVectors()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "portal-ticket-vectors.json");
        return JsonDocument.Parse(File.ReadAllBytes(path)).RootElement;
    }

    static DateTime At(long unix) => DateTime.UnixEpoch.AddSeconds(unix);

    // Ticket version 1 layout from docs/portal-ticket.md.
    static byte[] Unsigned(byte keyId, long issued, long expires, uint accountId, byte[] nonce, string audience, string name,
        string magic = "MAGT", byte version = 1)
    {
        var bytes = new List<byte>();
        bytes.AddRange(Encoding.ASCII.GetBytes(magic));
        bytes.Add(version);
        bytes.Add(keyId);
        for (int shift = 56; shift >= 0; shift -= 8) bytes.Add((byte)((ulong)issued >> shift));
        for (int shift = 56; shift >= 0; shift -= 8) bytes.Add((byte)((ulong)expires >> shift));
        for (int shift = 24; shift >= 0; shift -= 8) bytes.Add((byte)(accountId >> shift));
        bytes.AddRange(nonce);
        bytes.Add((byte)audience.Length);
        bytes.AddRange(Encoding.ASCII.GetBytes(audience));
        bytes.Add((byte)name.Length);
        bytes.AddRange(Encoding.ASCII.GetBytes(name));
        return bytes.ToArray();
    }

    static byte[] Sign(ECDsa key, byte[] unsigned) =>
        unsigned.Concat(key.SignData(unsigned, HashAlgorithmName.SHA256, DSASignatureFormat.IeeeP1363FixedFieldConcatenation)).ToArray();

    static byte[] Nonce(byte seed) => Enumerable.Range(0, 16).Select(i => (byte)(seed + i)).ToArray();

    static MagPortalTicketSettings Settings(string audience, params (byte id, byte[] spki)[] keys) =>
        MagPortalTicketSettings.Load(name => name switch
        {
            MagPortalTicketSettings.ModeVariable => "required",
            MagPortalTicketSettings.KeysVariable => string.Join(",", keys.Select(k => k.id + ":" + Convert.ToBase64String(k.spki))),
            MagPortalTicketSettings.AudienceVariable => audience,
            _ => null
        });

    static MagNpAuthenticateRequest Chunk(byte[] data, int fullSize, bool last, bool retail = false)
    {
        var request = retail ? new MagRetailNpAuthenticateRequest() : new MagNpAuthenticateRequest();
        request.FullTicketSize = (ushort)fullSize;
        request.TicketChunkSize = (ushort)data.Length;
        request.LastMessage = last;
        data.CopyTo(request.TicketChunk, 0);
        return request;
    }

    static MagPortalAuthentication Send(MagPortalTicketPolicy policy, byte[] ticket) =>
        policy.Authenticate(new MagPortalTicketAssembly(), Chunk(ticket, ticket.Length, true));

    /// <summary>Every portal vector, rebuilt from its fields, plus the high-s form of the first.</summary>
    public static void Vectors()
    {
        var root = LoadVectors();
        var key = root.GetProperty("key");
        byte keyId = (byte)key.GetProperty("key_id").GetInt32();
        byte[] spki = Convert.FromBase64String(key.GetProperty("spki_base64").GetString());
        var tickets = new List<byte[]>();
        foreach (var vector in root.GetProperty("valid").EnumerateArray())
        {
            byte[] unsigned = Unsigned((byte)vector.GetProperty("key_id").GetInt32(), vector.GetProperty("issued_at").GetInt64(),
                vector.GetProperty("expires_at").GetInt64(), vector.GetProperty("account_id").GetUInt32(),
                Convert.FromHexString(vector.GetProperty("nonce_hex").GetString()), vector.GetProperty("audience").GetString(),
                vector.GetProperty("player_name").GetString());
            byte[] ticket = unsigned.Concat(Convert.FromHexString(vector.GetProperty("signature_hex").GetString())).ToArray();
            Check(ticket.Length == vector.GetProperty("length").GetInt32(), "Rebuilt vector length");
            Check(Convert.ToHexString(SHA256.HashData(ticket)).ToLowerInvariant() == vector.GetProperty("ticket_sha256").GetString(),
                "Rebuilt vector hash does not match the portal's");
            long at = vector.GetProperty("verify_at").GetInt64();
            var policy = new MagPortalTicketPolicy(Settings(vector.GetProperty("audience").GetString(), (keyId, spki)), () => At(at));
            var result = Send(policy, ticket);
            Check(result.Accepted, "Valid vector rejected: " + result.Reason.Code());
            Check(result.Account == vector.GetProperty("player_name").GetString().ToLowerInvariant() &&
                result.AccountId == vector.GetProperty("account_id").GetUInt32(), "Valid vector bound the wrong identity");
            tickets.Add(ticket);
        }

        foreach (var vector in root.GetProperty("invalid").EnumerateArray())
        {
            byte[] ticket = tickets[vector.GetProperty("base").GetInt32()].ToArray();
            var mutation = vector.GetProperty("mutation");
            if (mutation.ValueKind == JsonValueKind.Object)
            {
                if (mutation.TryGetProperty("xor_byte", out var xor))
                    ticket[xor.GetProperty("offset").GetInt32()] ^= (byte)xor.GetProperty("value").GetInt32();
                else if (mutation.TryGetProperty("set_byte", out var set))
                    ticket[set.GetProperty("offset").GetInt32()] = (byte)set.GetProperty("value").GetInt32();
                else if (mutation.TryGetProperty("truncate", out var truncate))
                    ticket = ticket.Take(truncate.GetInt32()).ToArray();
                else throw new Exception("Unknown vector mutation in " + vector.GetProperty("case").GetString());
            }
            long at = vector.GetProperty("verify_at").GetInt64();
            var policy = new MagPortalTicketPolicy(Settings(vector.GetProperty("audience").GetString(), (keyId, spki)), () => At(at));
            var result = Send(policy, ticket);
            Check(!result.Accepted && result.Account == null && result.Reason.Code() == vector.GetProperty("expect").GetString(),
                "Invalid vector '" + vector.GetProperty("case").GetString() + "' gave " + result.Reason.Code());
        }

        // Either s form verifies: replace s with n - s in the first vector.
        var n = BigInteger.Parse("0FFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551", System.Globalization.NumberStyles.HexNumber);
        byte[] high = tickets[0].ToArray();
        var s = new BigInteger(high.AsSpan(high.Length - 32), isUnsigned: true, isBigEndian: true);
        byte[] flipped = (n - s).ToByteArray(isUnsigned: true, isBigEndian: true);
        Check(flipped.Length <= 32, "High-s form overflowed");
        Array.Clear(high, high.Length - 32, 32);
        flipped.CopyTo(high, high.Length - flipped.Length);
        Check(!high.SequenceEqual(tickets[0]), "High-s form equals the original");
        var first = root.GetProperty("valid")[0];
        var highPolicy = new MagPortalTicketPolicy(Settings(first.GetProperty("audience").GetString(), (keyId, spki)),
            () => At(first.GetProperty("verify_at").GetInt64()));
        Check(Send(highPolicy, high).Accepted, "High-s signature rejected");
    }

    /// <summary>Rules the vectors do not cover, with freshly generated keys.</summary>
    public static void Rules()
    {
        using var one = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        using var two = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        using var stranger = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        const long now = 1_900_000_000;
        const string audience = "game.example.test";
        var settings = Settings(audience, (1, one.ExportSubjectPublicKeyInfo()), (7, two.ExportSubjectPublicKeyInfo()));
        byte seed = 0;
        MagPortalTicketReason Verify(byte[] ticket) =>
            Send(new MagPortalTicketPolicy(settings, () => At(now)), ticket).Reason;
        byte[] Ticket(ECDsa key, byte id = 1, long issued = now - 10, long expires = now + 590, uint account = 9, string name = "Bravo", string magic = "MAGT", byte version = 1) =>
            Sign(key, Unsigned(id, issued, expires, account, Nonce(seed += 16), audience, name, magic, version));

        Check(Verify(Ticket(one)) == MagPortalTicketReason.Accepted, "Baseline ticket rejected");
        Check(Verify(Ticket(two, 7)) == MagPortalTicketReason.Accepted, "Second configured key (rotation) rejected");
        Check(Verify(Ticket(two, 1)) == MagPortalTicketReason.BadSignature, "Ticket verified under another key ID's key");
        Check(Verify(Ticket(stranger)) == MagPortalTicketReason.BadSignature, "Unconfigured signer accepted");
        Check(Verify(Ticket(one, 2)) == MagPortalTicketReason.UnknownKey, "Unknown key ID accepted");
        Check(Verify(Ticket(one, magic: "MAGX")) == MagPortalTicketReason.Malformed, "Bad magic accepted");
        Check(Verify(Ticket(one, version: 2)) == MagPortalTicketReason.Malformed, "Unknown version accepted");
        Check(Verify(Ticket(one, name: "Ab")) == MagPortalTicketReason.Malformed, "Two-character name accepted");
        Check(Verify(Ticket(one, name: "Abcdefghijklmnop")) == MagPortalTicketReason.Malformed, "Sixteen-character name accepted");
        Check(Verify(Ticket(one).Concat(new byte[] { 0 }).ToArray()) == MagPortalTicketReason.Malformed, "Trailing byte accepted");
        var emptyAudience = Sign(one, Unsigned(1, now, now + 60, 9, Nonce(200), "", "Bravo"));
        Check(Verify(emptyAudience) == MagPortalTicketReason.Malformed, "Empty audience accepted");
        Check(MagPortalTicketVerifier.MaximumLifetimeSeconds == 12 * 3600, "Ticket lifetime is not 12 hours");
        Check(Verify(Ticket(one, issued: now - 100, expires: now - 100 + 43201)) == MagPortalTicketReason.BadLifetime, "Lifetime over 12 h accepted");
        Check(Verify(Ticket(one, issued: now - 100, expires: now - 100 + 43200)) == MagPortalTicketReason.Accepted, "Lifetime of exactly 12 h rejected");
        Check(Verify(Ticket(one, issued: now - 43199, expires: now + 1)) == MagPortalTicketReason.Accepted, "12 h ticket rejected in its last second");
        Check(Verify(Ticket(one, issued: now - 43200, expires: now)) == MagPortalTicketReason.Expired, "12 h ticket accepted at its expiry");
        Check(Verify(Ticket(one, issued: now + 5, expires: now + 5)) == MagPortalTicketReason.BadLifetime, "Expiry equal to issue accepted");
        Check(Verify(Ticket(one, issued: now + 60, expires: now + 600)) == MagPortalTicketReason.Accepted, "60 s future skew rejected");
        Check(Verify(Ticket(one, issued: now + 61, expires: now + 600)) == MagPortalTicketReason.NotYetValid, "61 s future skew accepted");
        Check(Verify(Ticket(one, issued: now - 600, expires: now)) == MagPortalTicketReason.Expired, "Ticket accepted at its expiry");
        Check(Verify(Ticket(one, account: 0)) == MagPortalTicketReason.BadIdentity, "Account ID 0 accepted");
        Check(Verify(Ticket(one, name: "9bravo")) == MagPortalTicketReason.BadIdentity, "Name starting with a digit accepted");
        Check(Verify(Ticket(one, name: "Bra vo")) == MagPortalTicketReason.BadIdentity, "Name with a space accepted");
        var other = Sign(one, Unsigned(1, now, now + 60, 9, Nonce(220), "game.example.tes", "Bravo"));
        Check(Verify(other) == MagPortalTicketReason.WrongAudience, "Audience prefix accepted");

        var policy = new MagPortalTicketPolicy(settings, () => At(now));
        var signed = Ticket(one);
        var nonzeroRsa = Chunk(signed, signed.Length, true);
        nonzeroRsa.RsaWords[3] = 1;
        Check(policy.Authenticate(new MagPortalTicketAssembly(), nonzeroRsa).Reason == MagPortalTicketReason.BadHeader, "Non-zero RSA words accepted");
    }

    public static void Reassembly()
    {
        using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        const long now = 1_900_000_000;
        var settings = Settings("game.example.test", (1, key.ExportSubjectPublicKeyInfo()));
        byte seed = 0;
        byte[] Ticket() => Sign(key, Unsigned(1, now, now + 600, 77, Nonce(seed += 16), "game.example.test", "Charlie_9"));
        var policy = new MagPortalTicketPolicy(settings, () => At(now));

        // Three chunks that add up exactly.
        var ticket = Ticket();
        var assembly = new MagPortalTicketAssembly();
        Check(policy.Authenticate(assembly, Chunk(ticket[..40], ticket.Length, false)).Reason == MagPortalTicketReason.Pending, "First chunk not pending");
        Check(assembly.InProgress, "First chunk not kept");
        Check(policy.Authenticate(assembly, Chunk(ticket[40..100], ticket.Length, false)).Reason == MagPortalTicketReason.Pending, "Second chunk not pending");
        var done = policy.Authenticate(assembly, Chunk(ticket[100..], ticket.Length, true));
        Check(done.Accepted && done.Account == "charlie_9" && done.AccountId == 77 && !assembly.InProgress, "Three-chunk ticket rejected");

        MagPortalTicketReason Run(params MagNpAuthenticateRequest[] chunks)
        {
            var fresh = new MagPortalTicketAssembly();
            var reason = MagPortalTicketReason.Pending;
            foreach (var chunk in chunks)
            {
                reason = policy.Authenticate(fresh, chunk).Reason;
                if (reason != MagPortalTicketReason.Pending) break;
            }
            Check(!fresh.InProgress, "Finished exchange left a partial ticket");
            return reason;
        }
        ticket = Ticket();
        int full = ticket.Length;
        var bad = new Dictionary<string, MagNpAuthenticateRequest[]>
        {
            ["full size changes"] = new[] { Chunk(ticket[..40], full, false), Chunk(ticket[40..], full + 1, true) },
            ["chunks overflow"] = new[] { Chunk(ticket[..100], full, false), Chunk(ticket[50..], full, true) },
            ["last flag too early"] = new[] { Chunk(ticket[..40], full, false), Chunk(ticket[40..100], full, true) },
            ["full without last flag"] = new[] { Chunk(ticket, full, false) },
            ["empty chunk"] = new[] { Chunk(Array.Empty<byte>(), full, false) },
            ["too large"] = new[] { Chunk(new byte[900], 2049, false) },
            ["too many chunks"] = Enumerable.Range(0, 9).Select(i => Chunk(new byte[] { 1 }, 20, false)).ToArray(),
        };
        foreach (var (name, chunks) in bad)
            Check(Run(chunks) == MagPortalTicketReason.BadChunk, "Bad chunk sequence accepted: " + name);
        var dirty = Chunk(ticket, full, true);
        dirty.TicketChunk[full] = 1;
        Check(Run(dirty) == MagPortalTicketReason.BadChunk, "Data after the chunk size accepted");
        Check(Run(Chunk(ticket, full, true)) == MagPortalTicketReason.Accepted, "Untouched ticket rejected after bad sequences");

        // A rejected exchange discards its partial ticket; a session gets three failed attempts.
        assembly = new MagPortalTicketAssembly();
        ticket = Ticket();
        policy.Authenticate(assembly, Chunk(ticket[..40], ticket.Length, false));
        Check(policy.Authenticate(assembly, Chunk(ticket[40..], ticket.Length + 5, true)).Reason == MagPortalTicketReason.BadChunk, "Changed size accepted");
        Check(policy.Authenticate(assembly, Chunk(ticket, ticket.Length, true)).Accepted, "Clean retry rejected after a discarded chunk");
        assembly = new MagPortalTicketAssembly();
        for (int i = 0; i < MagPortalTicketPolicy.MaximumFailuresPerSession; i++)
            Check(policy.Authenticate(assembly, Chunk(Encoding.ASCII.GetBytes("synthetic"), 9, true)).Reason == MagPortalTicketReason.Malformed, "Plain name not malformed");
        Check(policy.Authenticate(assembly, Chunk(Ticket(), Ticket().Length, true)).Reason == MagPortalTicketReason.TooManyAttempts, "Unlimited attempts");
    }

    public static void Replay()
    {
        using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        long now = 1_900_000_000;
        var settings = Settings("game.example.test", (1, key.ExportSubjectPublicKeyInfo()));
        byte[] Ticket(byte seed, long expires) => Sign(key, Unsigned(1, now, expires, 5, Nonce(seed), "game.example.test", "Delta"));
        var policy = new MagPortalTicketPolicy(settings, () => At(now));
        var ticket = Ticket(1, now + 600);
        // The game reuses one ticket for its sign-ins and reconnects: 32 uses per service, never more.
        Check(MagPortalNonceCache.MaximumUses == 32, "Use limit is not 32");
        for (int use = 1; use <= MagPortalNonceCache.MaximumUses; use++)
            Check(Send(policy, ticket).Accepted, "Use " + use + " rejected");
        Check(Send(policy, ticket).Reason == MagPortalTicketReason.Replayed, "Use past the limit accepted");
        // A 12-hour ticket keeps its uses for its whole life, and its nonce is remembered that long.
        var longPolicy = new MagPortalTicketPolicy(settings, () => At(now + 43199));
        Check(Send(longPolicy, Ticket(2, now + 43200)).Accepted, "12 h ticket rejected near its end");
        var spent = Ticket(3, now + 43200);
        for (int use = 1; use <= MagPortalNonceCache.MaximumUses; use++) Check(Send(policy, spent).Accepted, "12 h ticket use " + use + " rejected");
        var nearEnd = new MagPortalTicketPolicy(settings, () => At(now + 43199), policy.Nonces);
        Check(Send(nearEnd, spent).Reason == MagPortalTicketReason.Replayed, "12 h ticket's nonce forgotten before it expired");
        Check(Send(policy, Ticket(1, now + 300)).Reason == MagPortalTicketReason.Replayed, "Nonce reuse in a new ticket accepted");
        Check(Send(new MagPortalTicketPolicy(settings, () => At(now)), ticket).Accepted, "Nonces leaked between independent caches");

        // Bounded: a full cache of live nonces refuses new tickets, and frees expired ones.
        var small = new MagPortalTicketPolicy(settings, () => At(now), new MagPortalNonceCache(1));
        Check(Send(small, Ticket(40, now + 60)).Accepted, "First ticket in a small cache rejected");
        Check(Send(small, Ticket(41, now + 600)).Reason == MagPortalTicketReason.ReplayCacheFull, "Full cache forgot a live nonce");
        now += 61;
        var later = Ticket(42, now + 600);
        Check(Send(small, later).Accepted && small.Nonces.Count == 1, "Expired nonce not purged");

        // One account cannot fill the cache: past its share its own new tickets are refused until they
        // expire, other accounts still sign in, and the cache never grows past its capacity.
        var cache = new MagPortalNonceCache(MagPortalNonceCache.MaximumNoncesPerAccount + 2);
        byte[] Unique(int i) => BitConverter.GetBytes((long)i).Concat(BitConverter.GetBytes(~(long)i)).ToArray();
        for (int i = 0; i < MagPortalNonceCache.MaximumNoncesPerAccount; i++)
            Check(cache.TryUse(Unique(i), 7, now + 43200, now) == MagPortalTicketReason.Accepted, "Account share refused early at " + i);
        Check(cache.TryUse(Unique(100000), 7, now + 43200, now) == MagPortalTicketReason.ReplayCacheFull, "Account held more than its share");
        Check(cache.CountFor(7) == MagPortalNonceCache.MaximumNoncesPerAccount, "Refused nonce counted for its account");
        Check(cache.TryUse(Unique(100001), 8, now + 43200, now) == MagPortalTicketReason.Accepted &&
            cache.TryUse(Unique(100002), 9, now + 600, now) == MagPortalTicketReason.Accepted, "Another account refused while one is at its share");
        Check(cache.TryUse(Unique(100003), 10, now + 43200, now) == MagPortalTicketReason.ReplayCacheFull &&
            cache.Count == MagPortalNonceCache.MaximumNoncesPerAccount + 2, "Cache grew past its capacity");
        // Account 9's nonce expires; the next sign-in after that sweeps it out and takes its place.
        Check(cache.TryUse(Unique(100004), 10, now + 43200, now + 601) == MagPortalTicketReason.Accepted &&
            cache.CountFor(9) == 0 && cache.Count == MagPortalNonceCache.MaximumNoncesPerAccount + 2, "Expired nonce not swept when full");
        // At expiry the account's share comes back.
        Check(cache.TryUse(Unique(100005), 7, now + 2 * 43200, now + 43200) == MagPortalTicketReason.Accepted &&
            cache.CountFor(7) == 1, "Account share not returned after its tickets expired");
        Check(MagPortalNonceCache.DefaultCapacity / MagPortalNonceCache.MaximumNoncesPerAccount >= 512, "Too few accounts fill the default cache");

        // Thread-safe: exactly the use limit of many concurrent uses of a nonce wins.
        var shared = new MagPortalNonceCache();
        int accepted = 0;
        Parallel.For(0, 4 * MagPortalNonceCache.MaximumUses, _ =>
        {
            if (shared.TryUse(Nonce(90), 5, now + 600, now) == MagPortalTicketReason.Accepted) Interlocked.Increment(ref accepted);
        });
        Check(accepted == MagPortalNonceCache.MaximumUses, "Concurrent nonce use accepted " + accepted + " times");
    }

    sealed class Owner : IMagPortalSession
    {
        public bool Live = true;
        public bool IsPortalSessionLive(DateTime now) => Live;
    }

    /// <summary>A ticket never opens a second live session for its account on one service.</summary>
    public static void LiveSessions()
    {
        using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        const long now = 1_900_000_000;
        var settings = Settings("game.example.test", (1, key.ExportSubjectPublicKeyInfo()));
        byte[] Ticket(byte seed, string name) => Sign(key, Unsigned(1, now, now + 600, 5, Nonce(seed), "game.example.test", name));
        var policy = new MagPortalTicketPolicy(settings, () => At(now));
        var sessionNow = At(now);
        MagPortalAuthentication Use(byte[] ticket, Owner owner) =>
            policy.Authenticate(new MagPortalTicketAssembly(), Chunk(ticket, ticket.Length, true), owner, sessionNow);

        var ticket = Ticket(1, "Foxtrot");
        var first = new Owner();
        Check(Use(ticket, first).Accepted, "First session rejected");
        var second = new Owner();
        Check(Use(ticket, second).Reason == MagPortalTicketReason.AccountSignedIn, "Second live session for the account opened");
        Check(Use(Ticket(2, "Foxtrot"), second).Reason == MagPortalTicketReason.AccountSignedIn, "A different ticket opened a second live session");
        Check(Use(Ticket(3, "Golf"), second).Accepted, "Another account refused while the first is signed in");
        Check(Use(ticket, first).Accepted, "The holding session could not sign in again");

        // Ending (released on disconnect) or going idle frees the account; refusals spent no uses.
        policy.Sessions.Release(first);
        var third = new Owner();
        Check(Use(ticket, third).Accepted, "Account not freed after its session ended");
        for (int use = 4; use <= MagPortalNonceCache.MaximumUses; use++) Check(Use(ticket, third).Accepted, "Holder's use " + use + " rejected");
        third.Live = false;
        var fourth = new Owner();
        Check(Use(ticket, fourth).Reason == MagPortalTicketReason.Replayed, "Use of the ticket past its limit accepted");
        Check(Use(Ticket(4, "Foxtrot"), fourth).Accepted, "Idle session kept its account");
        Check(Use(Ticket(5, "Foxtrot"), new Owner()).Reason == MagPortalTicketReason.AccountSignedIn, "Refused claim released the holder");
    }

    public static void Configuration()
    {
        using var p256 = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        using var p384 = ECDsa.Create(ECCurve.NamedCurves.nistP384);
        using var rsa = RSA.Create(2048);
        string good = Convert.ToBase64String(p256.ExportSubjectPublicKeyInfo());
        MagPortalTicketSettings Load(string mode, string keys, string audience) => MagPortalTicketSettings.Load(name => name switch
        {
            MagPortalTicketSettings.ModeVariable => mode,
            MagPortalTicketSettings.KeysVariable => keys,
            MagPortalTicketSettings.AudienceVariable => audience,
            _ => null
        });

        // Off by default; off ignores the other variables, so existing deployments are unchanged.
        foreach (var off in new[] { Load(null, null, null), Load("", "junk", "UPPER"), Load("off", "junk", null) })
            Check(off.Mode == MagPortalTicketMode.Off && MagPortalTicketPolicy.FromSettings(off) == null, "Off mode built a policy");

        var two = Load("required", " 1:" + good + " , 200:" + good, "play.example.test");
        Check(two.Mode == MagPortalTicketMode.Required && two.Keys.Count == 2 && two.Keys.ContainsKey(200) && two.Audience == "play.example.test",
            "Two-key configuration");
        Check(MagPortalTicketPolicy.FromSettings(two) != null, "Required mode built no policy");

        Throws(() => Load("on", "1:" + good, "a"), MagPortalTicketSettings.ModeVariable);
        Throws(() => Load("Required", "1:" + good, "a"), MagPortalTicketSettings.ModeVariable);
        Throws(() => Load("required", null, "a"), MagPortalTicketSettings.KeysVariable);
        Throws(() => Load("required", " ", "a"), MagPortalTicketSettings.KeysVariable);
        Throws(() => Load("required", "1:" + good, null), MagPortalTicketSettings.AudienceVariable);
        foreach (string audience in new[] { "Play.example.test", "play_example", new string('a', 64), "play.example.test/" })
            Throws(() => Load("required", "1:" + good, audience), MagPortalTicketSettings.AudienceVariable);
        foreach (string keys in new[] { good, "1=" + good, "256:" + good, "-1:" + good, "x:" + good, "1:", "1:not base64!",
            "1:" + good + ",1:" + good, "1:" + good + ",", "1:" + Convert.ToBase64String(p384.ExportSubjectPublicKeyInfo()),
            "1:" + Convert.ToBase64String(rsa.ExportSubjectPublicKeyInfo()), "1:" + Convert.ToBase64String(p256.ExportSubjectPublicKeyInfo().Concat(new byte[] { 0 }).ToArray()) })
            Throws(() => Load("required", keys, "play.example.test"), MagPortalTicketSettings.KeysVariable);
        // Errors name the variable, never the key material.
        try { Load("required", "1:" + good + "x", "a"); }
        catch (ArgumentException e) { Check(!e.Message.Contains(good[..20]), "Configuration error echoed key material"); }
    }

    // ---- Sessions: unchanged off mode, and required mode on the wire.

    static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Wire(Func<RT_MSG_CLIENT_APP_TO_PLUGIN, IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP>> session,
        MagMessage message, int app)
    {
        byte[] payload = MagMessageCodec.SerializeForApplication(message, app);
        using var stream = new MemoryStream();
        using (var writer = new BinaryWriter(stream, Encoding.ASCII, true))
        { writer.Write((ushort)payload.Length); writer.Write((byte)0); writer.Write(payload); }
        stream.Position = 0;
        var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN();
        using var reader = new MessageReader(stream);
        envelope.Deserialize(reader);
        return session(envelope);
    }

    static bool Succeeded(IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> replies, int app) =>
        replies.Count == 1 && MagMessageCodec.TryParseForApplication(replies[0].Payload, app, out var parsed) &&
        parsed is MagNpAuthenticateResponse response && response.Success;

    /// <summary>One well-formed failure reply of the request's own message type, echoing its transaction.</summary>
    static bool Refused(IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> replies, int app, int transactionId = 0) =>
        replies.Count == 1 && MagMessageCodec.TryParseForApplication(replies[0].Payload, app, out var parsed) &&
        parsed is MagNpAuthenticateResponse response && !response.Success && response.TransactionId == transactionId &&
        (app == 20231) == (response is MagRetailNpAuthenticateResponse);

    static MagNpAuthenticateRequest Plain(int app)
    {
        var plain = Chunk(Encoding.ASCII.GetBytes("Synthetic"), 9, true, app == 20231);
        return plain;
    }

    public static void Sessions()
    {
        using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        const long now = 1_900_000_000;
        var settings = Settings("game.example.test", (1, key.ExportSubjectPublicKeyInfo()));
        byte seed = 0;
        byte[] Ticket() => Sign(key, Unsigned(1, now - 5, now + 600, 1234, Nonce(seed += 16), "game.example.test", "Echo_Ranger-07"));
        int previousApp = Server.Medius.Program.Settings.MagApplicationId;
        var clock = At(now);
        try
        {
            foreach (int app in new[] { 20230, 20231 })
            {
                Server.Medius.Program.Settings.MagApplicationId = app;
                bool retail = app == 20231;
                MagMasSession Mas(bool development, MagPortalTicketPolicy policy)
                {
                    var mas = new MagMasSession { DevelopmentAuthEnabled = development, PortalTickets = policy };
                    Wire(e => mas.HandleMany(true, app, e, clock), new MagHello(), app);
                    Wire(e => mas.HandleMany(true, app, e, clock), new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, app);
                    Check(mas.State == MagMasSession.SessionState.ProtocolAcknowledged, "MAS handshake");
                    return mas;
                }

                // Off: the development name policy is unchanged and no portal state is touched.
                var off = Mas(true, null);
                Check(Succeeded(Wire(e => off.HandleMany(true, app, e, clock), Plain(app), app), app) &&
                    off.AuthenticatedAccount == "synthetic" && off.PortalAccountId == 0 && off.PortalTicketStatus == "inactive",
                    "Off mode development sign-in changed");
                var disabled = Mas(false, null);
                Check(Wire(e => disabled.HandleMany(true, app, e, clock), Plain(app), app).Count == 0 &&
                    disabled.State == MagMasSession.SessionState.ProtocolAcknowledged && disabled.PortalTicketRefusal == null,
                    "Off mode signed in without development auth, or answered the refusal");

                // Required: the plain name is refused even with development auth on; a ticket signs in.
                var policy = new MagPortalTicketPolicy(settings, () => clock);
                var mas = Mas(true, policy);
                Check(Refused(Wire(e => mas.HandleMany(true, app, e, clock), Plain(app), app), app) &&
                    mas.State == MagMasSession.SessionState.ProtocolAcknowledged && mas.AuthenticatedAccount == null &&
                    mas.PortalTicketStatus == "malformed" && mas.PortalTicketRefusal == "portal ticket refused: reason=malformed",
                    "Required mode accepted the plain development name");
                var ticket = Ticket();
                Check(Wire(e => mas.HandleMany(true, app, e, clock), Chunk(ticket[..60], ticket.Length, false, retail), app).Count == 0 &&
                    mas.PortalTicketStatus == "pending" && mas.PortalTicketRefusal == null, "First wire chunk answered");
                Check(Succeeded(Wire(e => mas.HandleMany(true, app, e, clock), Chunk(ticket[60..], ticket.Length, true, retail), app), app) &&
                    mas.State == MagMasSession.SessionState.LocalDevelopmentAuthenticated && mas.AuthenticatedAccount == "echo_ranger-07" &&
                    mas.PortalAccountId == 1234 && mas.PortalTicketStatus == "accepted", "Required mode ticket sign-in");
                var second = Mas(false, policy);
                var signedInRequest = Chunk(ticket, ticket.Length, true, retail);
                signedInRequest.TransactionId = 41;
                Check(Refused(Wire(e => second.HandleMany(true, app, e, clock), signedInRequest, app), app, 41) &&
                    second.PortalTicketStatus == "account_signed_in" && second.AuthenticatedAccount == null &&
                    second.PortalTicketRefusal.StartsWith("portal ticket refused: reason=account_signed_in account=echo_ranger-07 account_id=1234 "),
                    "Ticket opened a second live MAS session");
                mas.EndPortalSession();
                Check(Succeeded(Wire(e => second.HandleMany(true, app, e, clock), Chunk(ticket, ticket.Length, true, retail), app), app) &&
                    second.AuthenticatedAccount == "echo_ranger-07", "Account not freed after the first MAS session ended");
                second.EndPortalSession();

                // Without development auth the portal still opens the account stack: character creation works,
                // and the NP ID's online ID must match the ticket's name ignoring case.
                var store = Path.Combine(Path.GetTempPath(), "mag-portal-" + Guid.NewGuid().ToString("N"));
                Directory.CreateDirectory(store);
                try
                {
                    var created = Mas(false, policy);
                    created.CharacterStorageEnabled = true;
                    created.CharacterStore = new LocalCharacterStore(Path.Combine(store, "characters.json"));
                    var fresh = Ticket();
                    Check(Succeeded(Wire(e => created.HandleMany(true, app, e, clock), Chunk(fresh, fresh.Length, true, retail), app), app), "Ticket sign-in for creation");
                    byte[] npId = new byte[36];
                    Encoding.ASCII.GetBytes("Echo_Ranger-07").CopyTo(npId, 0);
                    var reply = Wire(e => created.HandleMany(true, app, e, clock),
                        new MagCreateCharacterRequest { AccountName = "Echo", Faction = 1, HeadId = 2, VoiceId = 3, UniverseId = 26, NpId = npId }, app);
                    Check(reply.Count == 1 && created.StorageStatus == "local-profile-committed" &&
                        created.CharacterStore.ReadForAccount("echo_ranger-07").Single().NpId.SequenceEqual(npId),
                        "Portal session with a matching NP ID could not create its character: " + created.StorageStatus);
                    Encoding.ASCII.GetBytes("Echo_Ranger-08").CopyTo(npId, 0);
                    var mismatch = created.CharacterStore.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Other", Faction = 1, HeadId = 2, VoiceId = 3, UniverseId = 26, NpId = npId },
                        created.AuthenticatedAccount);
                    Check(mismatch.Reason == "np-id-account-mismatch", "NP ID for another name accepted: " + mismatch.Reason);
                    created.EndPortalSession();
                }
                finally { Directory.Delete(store, true); }

                // An idle session past its timeout no longer holds the account.
                var holder = Mas(false, policy);
                holder.PortalLiveTimeoutSeconds = 30;
                var held = Ticket();
                Check(Succeeded(Wire(e => holder.HandleMany(true, app, e, clock), Chunk(held, held.Length, true, retail), app), app), "Holder sign-in");
                clock = clock.AddSeconds(31);
                var successor = Mas(false, policy);
                Check(Succeeded(Wire(e => successor.HandleMany(true, app, e, clock), Chunk(held, held.Length, true, retail), app), app),
                    "Idle MAS session kept its account");

                // MAPS: same policy, its own state machine.
                var maps = new MagMapsSession { DevelopmentAuthEnabled = true, PortalTickets = new MagPortalTicketPolicy(settings, () => clock) };
                Wire(e => maps.HandleMany(true, app, e, clock), new MagHello(), app);
                if (!retail) Wire(e => maps.HandleMany(true, app, e, clock), new MagUniverseListRequest(), app);
                Check(Refused(Wire(e => maps.HandleMany(true, app, e, clock), Plain(app), app), app) && maps.AuthenticatedAccount == null,
                    "MAPS required mode accepted the plain name");
                // An expired ticket: a failure reply right away, and a staff log naming the account and its expiry.
                var expired = Sign(key, Unsigned(1, now - 700, now - 100, 1234, Nonce(250), "game.example.test", "Echo_Ranger-07"));
                var expiredRequest = Chunk(expired, expired.Length, true, retail);
                expiredRequest.TransactionId = 7;
                Check(Refused(Wire(e => maps.HandleMany(true, app, e, clock), expiredRequest, app), app, 7) &&
                    maps.PortalTicketStatus == "expired" && maps.AuthenticatedAccount == null, "MAPS expired ticket not refused with a reply");
                string expiredAt = DateTime.UnixEpoch.AddSeconds(now - 100).ToString("yyyy-MM-dd'T'HH:mm:ss'Z'");
                string issuedAt = DateTime.UnixEpoch.AddSeconds(now - 700).ToString("yyyy-MM-dd'T'HH:mm:ss'Z'");
                long age = (long)(clock - At(now - 700)).TotalSeconds;
                Check(maps.PortalTicketRefusal == "portal ticket refused: reason=expired account=echo_ranger-07 account_id=1234 issued_at=" +
                    issuedAt + " expires_at=" + expiredAt + " age_s=" + age, "Expired refusal log: " + maps.PortalTicketRefusal);
                Check(!maps.PortalTicketRefusal.Contains(Convert.ToHexString(Nonce(250)), StringComparison.OrdinalIgnoreCase) &&
                    !maps.PortalTicketRefusal.Contains(Convert.ToBase64String(expired)), "Refusal log carries ticket bytes");
                // An unverified signature names no account; the log line is cleared by the next message.
                using (var stranger = ECDsa.Create(ECCurve.NamedCurves.nistP256))
                {
                    var forged = Sign(stranger, Unsigned(1, now, now + 600, 1234, Nonce(251), "game.example.test", "Echo_Ranger-07"));
                    Check(Refused(Wire(e => maps.HandleMany(true, app, e, clock), Chunk(forged, forged.Length, true, retail), app), app) &&
                        maps.PortalTicketRefusal == "portal ticket refused: reason=bad_signature", "Forged ticket log: " + maps.PortalTicketRefusal);
                }
                // Three failures used up the session's attempts: further tickets are ignored without a reply.
                var ignored = Ticket();
                Check(Wire(e => maps.HandleMany(true, app, e, clock), Chunk(ignored, ignored.Length, true, retail), app).Count == 0 &&
                    maps.PortalTicketStatus == "too_many_attempts" && maps.AuthenticatedAccount == null, "Attempts past the limit answered");
                maps = new MagMapsSession { DevelopmentAuthEnabled = true, PortalTickets = maps.PortalTickets };
                Wire(e => maps.HandleMany(true, app, e, clock), new MagHello(), app);
                if (!retail) Wire(e => maps.HandleMany(true, app, e, clock), new MagUniverseListRequest(), app);
                // A bad non-final chunk gets no reply: the client is still sending, and its final chunk is answered.
                var partial = Ticket();
                Check(Wire(e => maps.HandleMany(true, app, e, clock), Chunk(partial[..40], partial.Length + 1, false, retail), app).Count == 0, "First chunk answered");
                Check(Wire(e => maps.HandleMany(true, app, e, clock), Chunk(partial[40..], partial.Length, false, retail), app).Count == 0 &&
                    maps.PortalTicketStatus == "bad_chunk", "Bad non-final chunk answered");
                var mapsTicket = Ticket();
                Check(Succeeded(Wire(e => maps.HandleMany(true, app, e, clock), Chunk(mapsTicket, mapsTicket.Length, true, retail), app), app) &&
                    maps.AuthenticatedAccount == "echo_ranger-07" && maps.PortalAccountId == 1234, "MAPS ticket sign-in");
                var mapsOff = new MagMapsSession { DevelopmentAuthEnabled = true };
                Wire(e => mapsOff.HandleMany(true, app, e, clock), new MagHello(), app);
                if (!retail) Wire(e => mapsOff.HandleMany(true, app, e, clock), new MagUniverseListRequest(), app);
                Check(Succeeded(Wire(e => mapsOff.HandleMany(true, app, e, clock), Plain(app), app), app) && mapsOff.AuthenticatedAccount == "synthetic",
                    "MAPS off mode development sign-in changed");
            }
            Check(MagPortalTicketPolicy.IsTicketMessage(20231, 0x0034) && MagPortalTicketPolicy.IsTicketMessage(20230, 0x0031) &&
                !MagPortalTicketPolicy.IsTicketMessage(20231, 0x0031), "Ticket message IDs for log redaction");
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previousApp; }
    }
    public static void RenamedSessions()
    {
        using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        const long now = 1_900_000_000;
        var clock = At(now);
        var settings = Settings("game.example.test", (1, key.ExportSubjectPublicKeyInfo()));
        IReadOnlyDictionary<uint, string> rows = new Dictionary<uint, string> { [17] = "oldpublic", [18] = "otherowner", [19] = "service_owner" };
        using var cache = new MagPortalAccountCache(_ => Task.FromResult(rows), () => clock);
        cache.RefreshAsync().GetAwaiter().GetResult();
        byte seed = 0;
        byte[] Ticket(uint id, string name) => Sign(key, Unsigned(1, now - 5, now + 600, id, Nonce(++seed), "game.example.test", name));
        var policy = new MagPortalTicketPolicy(settings, () => clock, accounts: cache);
        MagPortalAuthentication Use(uint id, string name, Owner owner = null)
        {
            byte[] bytes = Ticket(id, name);
            return policy.Authenticate(new MagPortalTicketAssembly(), Chunk(bytes, bytes.Length, true), owner, clock);
        }
        Check(Use(17, "OldPublic").Account == "oldpublic" && Use(17, "NewPublic").Account == "oldpublic", "Old/new signed names changed immutable owner");
        Check(Use(18, "NewPublic").Account == "otherowner", "Same public name moved another ID to first owner's characters");
        Check(Use(88, "NewPublic").Reason == MagPortalTicketReason.AccountUnavailable, "Name-based fallback accepted unknown numeric ID");
        Check(Use(19, "ServicePublic").Account == "service_owner", "Service ticket refused");
        var held = new Owner();
        Check(Use(17, "OldPublic", held).Accepted && Use(17, "NewPublic", new Owner()).Reason == MagPortalTicketReason.AccountSignedIn,
            "Renamed ticket bypassed stable live-session binding");
        held.Live = false;
        int previousApp = Server.Medius.Program.Settings.MagApplicationId;
        string directory = Path.Combine(Path.GetTempPath(), "mag-renamed-wire-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            foreach (int app in new[] { 20230, 20231 })
            {
                Server.Medius.Program.Settings.MagApplicationId = app;
                var store = new LocalCharacterStore(Path.Combine(directory, app + ".json"));
                var oldNp = new byte[36]; Encoding.ASCII.GetBytes("OldPublic").CopyTo(oldNp, 0);
                Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "OldPublic!0", Faction = 1, HeadId = 2, VoiceId = 3, UniverseId = 26, NpId = oldNp }, "oldpublic", "OldPublic").Result == MagNetResult.Success, "Old character setup");
                var registry = new MagLoginRegistry();
                var mas = new MagMasSession { PortalTickets = policy, CharacterStore = store, CharacterStorageEnabled = true, AccountLoginEnabled = true, LoginRegistry = registry };
                Wire(e => mas.HandleMany(true, app, e, clock), new MagHello(), app);
                Wire(e => mas.HandleMany(true, app, e, clock), new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, app);
                var bytes = Ticket(17, "NewPublic");
                Check(Succeeded(Wire(e => mas.HandleMany(true, app, e, clock), Chunk(bytes, bytes.Length, true, app == 20231), app), app) && mas.AuthenticatedAccount == "oldpublic" && mas.AuthenticatedPublicName == "NewPublic", "MAS signed rename identity");
                var newNp = new byte[36]; Encoding.ASCII.GetBytes("NewPublic").CopyTo(newNp, 0);
                Wire(e => mas.HandleMany(true, app, e, clock), new MagCreateCharacterRequest { AccountName = "NewPublic!1", Faction = 1, HeadId = 2, VoiceId = 3, UniverseId = 26, NpId = newNp }, app);
                Check(mas.StorageStatus == "local-profile-committed" && store.ReadForAccount("oldpublic").Count == 2 && store.ReadForAccount("newpublic").Count == 0, "MAS creation used public name as owner");
                var replies = Wire(e => mas.HandleMany(true, app, e, clock), app == 20231
                    ? new MagRetailAccountLoginRequest { AccountName = "OldPublic!0", VersionMajor = 3, VersionMinor = 3, VersionBuild = 1, DnasSignature = "" }
                    : new MagAccountLoginRequest { AccountName = "OldPublic!0", VersionMajor = 3, VersionMinor = 3, VersionBuild = 1, DnasSignature = "" }, app);
                Check(replies.Count == 1 && MagMessageCodec.TryParseForApplication(replies[0].Payload, app, out var parsed) && parsed is MagAccountLoginResponse,
                    "MAS did not issue selected character credentials");
                MagMessageCodec.TryParseForApplication(replies[0].Payload, app, out var message);
                var login = (MagAccountLoginResponse)message;
                Check(login.Result == 1, "Existing stored character login denied after rename");
                string Ascii(byte[] field) => Encoding.ASCII.GetString(field).TrimEnd('\0');
                var mls = new MagMlsSession { CharacterStore = store };
                Check(mls.TryBind(new RT_MSG_CLIENT_CONNECT_TCP { AppId = app, TargetWorldId = 1, AccessToken = Ascii(login.AccessKey), SessionKey = Ascii(login.SessionKey) }, registry, "renamed-" + app, clock) &&
                    mls.AuthenticatedAccount == "oldpublic" && mls.AuthenticatedPublicName == "NewPublic", "Credentials lost public/owner separation");
                Check(mls.Accept(clock) && mls.CompleteTransport(clock), "MLS transport setup");
                Wire(e => mls.HandleMany(e, clock), new MagHello(), app);
                Wire(e => mls.HandleMany(e, clock), new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, app);
                replies = Wire(e => mls.HandleMany(e, clock), new MagCharacterDataRequest { QueryKind = 1, AccountName = "NewPublic" }, app);
                Check(replies.Count == 1 && MagMessageCodec.TryParseForApplication(replies[0].Payload, app, out parsed) && parsed is MagCharacterDataResponse card && card.Success &&
                    StoredCharacter.DisplayNameFor(card.Record.Name, card.Record.NpId) == "NewPublic" && card.Record.AccountId == login.AccountId, "MLS card missed authenticated public name");
                var old = store.ReadForAccount("oldpublic").Single(c => c.Name == "OldPublic!0");
                Check(old.NpId.SequenceEqual(oldNp) && old.Name == "OldPublic!0", "Display response rewrote historical character identity");
                mas.EndPortalSession();
                var maps = new MagMapsSession { PortalTickets = new MagPortalTicketPolicy(settings, () => clock, accounts: cache) };
                Wire(e => maps.HandleMany(true, app, e, clock), new MagHello(), app);
                if (app != 20231) Wire(e => maps.HandleMany(true, app, e, clock), new MagUniverseListRequest(), app);
                bytes = Ticket(17, "NewPublic");
                Check(Succeeded(Wire(e => maps.HandleMany(true, app, e, clock), Chunk(bytes, bytes.Length, true, app == 20231), app), app) &&
                    maps.AuthenticatedAccount == "oldpublic" && maps.AuthenticatedPublicName == "NewPublic", "MAPS identity separation");
                maps.EndPortalSession();
            }
        }
        finally { Directory.Delete(directory, true); Server.Medius.Program.Settings.MagApplicationId = previousApp; }
        rows = new Dictionary<uint, string> { [19] = "service_owner" };
        cache.RefreshAsync().GetAwaiter().GetResult();
        Check(Use(17, "OldPublic").Reason == MagPortalTicketReason.AccountUnavailable, "Banned/deleted identity still admitted after refresh");
        clock = clock.AddSeconds(5);
        Check(Use(19, "ServicePublic").Reason == MagPortalTicketReason.AccountUnavailable, "Stale lookup admitted signed ticket");
    }

}
