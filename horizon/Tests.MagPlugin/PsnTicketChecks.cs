using System.Buffers.Binary;
using System.Security.Cryptography;
using System.Text;
using Org.BouncyCastle.Asn1.X9;
using Org.BouncyCastle.Crypto;
using Org.BouncyCastle.Crypto.Generators;
using Org.BouncyCastle.Crypto.Parameters;
using Org.BouncyCastle.Security;
using Org.BouncyCastle.X509;
using Server.Medius;

// Synthetic NP 2.1 tickets only. Private test keys exist only for this process.
internal static class PsnTicketChecks
{
    const long Now = 1_800_000_000;
    const string Service = "TEST00-SYNTHETIC_00";
    const int SignatureOffset = 192;

    public static void Run()
    {
        MillisecondBoundaries();
        var generator = new ECKeyPairGenerator();
        generator.Init(new ECKeyGenerationParameters(ECNamedCurveTable.GetOid("secp192r1"), new SecureRandom()));
        var pair = generator.GenerateKeyPair();
        var spki = SubjectPublicKeyInfoFactory.CreateSubjectPublicKeyInfo(pair.Public).GetDerEncoded();
        var keys = new Dictionary<string, byte[]> { ["1234abcd"] = spki };
        var verifier = new MagPsnTicketVerifier(keys, new[] { Service });
        var integrationFixture = Ticket(Now, 42, "Other_Name", Service, out var integrationKeys);
        var integrationVerifier = new MagPsnTicketVerifier(integrationKeys, new[] { Service });
        Expect(integrationVerifier, integrationFixture, MagPortalTicketReason.Accepted);
        integrationVerifier.Verify(integrationFixture, Now, out var integrationIdentity);
        Check(integrationIdentity.Subject == "256:42" && integrationIdentity.OnlineId == "Other_Name", "integration fixture carries requested identity");
        byte[] valid = Build(pair.Private);
        Expect(verifier, valid, MagPortalTicketReason.Accepted);
        verifier.Verify(valid, Now, out var ticket);
        Check(ticket.Subject == "256:18446744073709551615", "unsigned, provider-qualified subject");
        Check(ticket.OnlineId == "Synthetic_Name" && ticket.ServiceId == Service, "signed online ID and service");
        Check(ticket.ExpiresAt == Now + 600 && ticket.ReplayNonce.Length == 16, "expiry seconds and replay nonce bounds");
        Check(ticket.ReplayNonce.SequenceEqual(SHA256.HashData(valid.AsSpan(0, SignatureOffset)).AsSpan(0, 16).ToArray()), "replay digest covers authenticated prefix");
        verifier.Verify(Build(pair.Private), Now, out var resigned);
        Check(ticket.ReplayNonce.SequenceEqual(resigned.ReplayNonce), "ECDSA signature variation cannot evade replay identity");
        keys.Clear(); spki[0] ^= 1; // Configuration is snapshotted, not retained by reference.
        Expect(verifier, valid, MagPortalTicketReason.Accepted);
        Expect(verifier, null, MagPortalTicketReason.Malformed);
        for (int length = 0; length < valid.Length; length++)
            Expect(verifier, valid.AsSpan(0, length).ToArray(), MagPortalTicketReason.Malformed);
        Expect(verifier, valid.Concat(new byte[1]).ToArray(), MagPortalTicketReason.Malformed);
        Expect(verifier, Encoding.ASCII.GetBytes("MAGT synthetic portal ticket"), MagPortalTicketReason.Malformed);

        // Each mutation is signed unless signature coverage itself is under test.
        foreach (int offset in new[] { 0, 1, 2, 3, 7, 8, 9, 11, 12, 15, 36, 39, 44, 47, 56, 59, 68, 71, 80, 83, 116, 119, 124, 127, 132, 135, 160, 163, 168, 171, 172, 175, 176, 177, 179, 180, 183, 188, 191 })
            Expect(verifier, Build(pair.Private, b => b[offset] ^= 1), MagPortalTicketReason.Malformed);
        Expect(verifier, Build(pair.Private, b => b[184] ^= 1), MagPortalTicketReason.UnknownKey);
        Expect(verifier, Build(pair.Private, b => BinaryPrimitives.WriteUInt32BigEndian(b.AsSpan(40), 0x33333333)), MagPortalTicketReason.BadIdentity);
        Expect(verifier, Build(pair.Private, b => BinaryPrimitives.WriteUInt32BigEndian(b.AsSpan(40), 257)), MagPortalTicketReason.BadIdentity);
        Expect(verifier, Build(pair.Private, b => BinaryPrimitives.WriteUInt64BigEndian(b.AsSpan(72), 0)), MagPortalTicketReason.BadIdentity);
        foreach (Action<byte[]> mutate in new Action<byte[]>[] {
            b => b[84] = 0, b => b[84] = (byte)'1', b => b[84] = 0xff,
            b => b[90] = (byte)' ', b => b[100] = (byte)'x', b => b[115] = 1 })
            Expect(verifier, Build(pair.Private, mutate), MagPortalTicketReason.BadIdentity);
        Expect(verifier, Build(pair.Private, b => b[136] = (byte)'X'), MagPortalTicketReason.WrongAudience);
        Expect(verifier, Build(pair.Private, b => b[159] = 1), MagPortalTicketReason.Malformed);
        Expect(verifier, Build(pair.Private, b => SetTime(b, 48, (ulong)(Now + 1) * 1000)), MagPortalTicketReason.NotYetValid);
        Expect(verifier, Build(pair.Private, b => SetTime(b, 60, (ulong)Now * 1000)), MagPortalTicketReason.Expired);
        Expect(verifier, Build(pair.Private, b => SetTime(b, 60, (ulong)(Now - 61) * 1000)), MagPortalTicketReason.BadLifetime);
        Expect(verifier, Build(pair.Private, b => SetTime(b, 48, ulong.MaxValue)), MagPortalTicketReason.BadLifetime);
        Expect(verifier, Build(pair.Private, b => SetTime(b, 60, ulong.MaxValue)), MagPortalTicketReason.BadLifetime);
        Expect(verifier, Build(pair.Private, b => SetTime(b, 60, (ulong)(Now + 43201) * 1000)), MagPortalTicketReason.BadLifetime);
        // At a partial final second the exported replay expiry rounds up, while validation remains millisecond-exact.
        var partial = Build(pair.Private, b => SetTime(b, 60, (ulong)Now * 1000 + 1));
        Expect(verifier, partial, MagPortalTicketReason.Accepted);
        verifier.Verify(partial, Now, out var partialTicket);
        Check(partialTicket.ExpiresAt == Now + 1, "replay entries cannot expire before the signed millisecond expiry");
        Expect(verifier, valid, MagPortalTicketReason.Malformed, long.MaxValue);
        Expect(verifier, valid, MagPortalTicketReason.Malformed, -1);
        foreach (int offset in new[] { 20, 40, 48, 60, 72, 84, 136, 164, SignatureOffset + 8 })
        {
            var forged = valid.ToArray(); forged[offset] ^= 1;
            Check(verifier.Verify(forged, Now, out var rejected) != MagPortalTicketReason.Accepted && rejected == null, "forged signed field rejected");
        }
        var badSignature = valid.ToArray(); badSignature[SignatureOffset] = 0x31;
        Expect(verifier, badSignature, MagPortalTicketReason.BadSignature);
        var badPadding = valid.ToArray(); badPadding[^1] = 0x7f;
        Check(verifier.Verify(badPadding, Now, out _) != MagPortalTicketReason.Accepted, "nonzero signature trailer rejected");
        byte[] publicKey = SubjectPublicKeyInfoFactory.CreateSubjectPublicKeyInfo(pair.Public).GetDerEncoded();
        Throws(() => new MagPsnTicketVerifier(new Dictionary<string, byte[]>(), new[] { Service }));
        Throws(() => new MagPsnTicketVerifier(new Dictionary<string, byte[]> { ["1234ABCD"] = publicKey }, new[] { Service }));
        Throws(() => new MagPsnTicketVerifier(new Dictionary<string, byte[]> { ["1234abcd"] = publicKey.Concat(new byte[1]).ToArray() }, new[] { Service }));
        Throws(() => new MagPsnTicketVerifier(new Dictionary<string, byte[]> { ["1234abcd"] = publicKey }, Array.Empty<string>()));
        Throws(() => new MagPsnTicketVerifier(new Dictionary<string, byte[]> { ["1234abcd"] = publicKey }, new[] { "TEST\0hidden" }));
        generator.Init(new ECKeyGenerationParameters(ECNamedCurveTable.GetOid("secp256r1"), new SecureRandom()));
        Throws(() => new MagPsnTicketVerifier(new Dictionary<string, byte[]> { ["1234abcd"] = SubjectPublicKeyInfoFactory.CreateSubjectPublicKeyInfo(generator.GenerateKeyPair().Public).GetDerEncoded() }, new[] { Service }));
    }

    public static void MillisecondBoundaries()
    {
        const long issued = 1_799_999_999_100;
        const long expires = 1_800_000_000_501;
        var bytes = TicketAtMilliseconds(issued, expires, 42, "Fractional_Name", Service, out var keys);
        var verifier = new MagPsnTicketVerifier(keys, new[] { Service });
        // Rounding the clock down to seconds would incorrectly accept this expired ticket.
        ExpectMilliseconds(verifier, bytes, expires, MagPortalTicketReason.Expired);
        ExpectMilliseconds(verifier, bytes, expires + 1, MagPortalTicketReason.Expired);
        ExpectMilliseconds(verifier, bytes, issued - 1, MagPortalTicketReason.NotYetValid);
        ExpectMilliseconds(verifier, bytes, issued, MagPortalTicketReason.Accepted);
        ExpectMilliseconds(verifier, bytes, expires - 1, MagPortalTicketReason.Accepted);
        verifier.VerifyAtMilliseconds(bytes, expires - 1, out var ticket);
        Check(ticket.IssuedAtMilliseconds == issued && ticket.ExpiresAtMilliseconds == expires,
            "admission and proof caps retain exact signed millisecond timestamps");
        Check(ticket.ExpiresAt == 1_800_000_001, "only replay retention rounds expiry upward");
        ExpectMilliseconds(verifier, bytes, -1, MagPortalTicketReason.Malformed);
        ExpectMilliseconds(verifier, bytes, long.MaxValue, MagPortalTicketReason.Expired);
        // The compatibility seconds API denotes exactly the supplied integral second.
        Expect(verifier, bytes, MagPortalTicketReason.NotYetValid, 1_799_999_999);
        Expect(verifier, bytes, MagPortalTicketReason.Accepted, 1_800_000_000);
        Expect(verifier, bytes, MagPortalTicketReason.Expired, 1_800_000_001);
    }

    static void ExpectMilliseconds(MagPsnTicketVerifier verifier, byte[] bytes, long now, MagPortalTicketReason expected)
    {
        var actual = verifier.VerifyAtMilliseconds(bytes, now, out var ticket);
        Check(actual == expected, $"Sony NP millisecond boundary: expected {expected}, got {actual}");
        Check((actual == MagPortalTicketReason.Accepted) == (ticket != null), "millisecond refusal exposes no identity");
    }

    // Integration tests use a fresh, memory-only key per fixture. No shared signing secret.
    public static byte[] Ticket(long now, ulong userId, string onlineId, string serviceId, out IReadOnlyDictionary<string, byte[]> keys) =>
        TicketAtMilliseconds(checked((now - 60) * 1000), checked((now + 600) * 1000), userId, onlineId, serviceId, out keys);

    public static byte[] TicketAtMilliseconds(long issuedAtMilliseconds, long expiresAtMilliseconds, ulong userId,
        string onlineId, string serviceId, out IReadOnlyDictionary<string, byte[]> keys)
    {
        var generator = new ECKeyPairGenerator();
        generator.Init(new ECKeyGenerationParameters(ECNamedCurveTable.GetOid("secp192r1"), new SecureRandom()));
        var pair = generator.GenerateKeyPair();
        keys = new Dictionary<string, byte[]> { ["1234abcd"] = SubjectPublicKeyInfoFactory.CreateSubjectPublicKeyInfo(pair.Public).GetDerEncoded() };
        return Build(pair.Private, bytes =>
        {
            SetTime(bytes, 48, checked((ulong)issuedAtMilliseconds));
            SetTime(bytes, 60, checked((ulong)expiresAtMilliseconds));
            BinaryPrimitives.WriteUInt64BigEndian(bytes.AsSpan(72), userId);
            bytes.AsSpan(84, 32).Clear(); Encoding.ASCII.GetBytes(onlineId).CopyTo(bytes, 84);
            bytes.AsSpan(136, 24).Clear(); Encoding.ASCII.GetBytes(serviceId).CopyTo(bytes, 136);
        });
    }

    static void Expect(MagPsnTicketVerifier verifier, byte[] bytes, MagPortalTicketReason expected, long now = Now)
    {
        var actual = verifier.Verify(bytes, now, out var ticket);
        Check(actual == expected, $"Sony NP ticket: expected {expected}, got {actual}");
        Check((actual == MagPortalTicketReason.Accepted) == (ticket != null), "refusal must expose no identity");
    }
    static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    static void Throws(Action action)
    {
        try { action(); } catch (ArgumentException) { return; }
        throw new Exception("Invalid verifier configuration was accepted");
    }
    static void SetTime(byte[] bytes, int offset, ulong value) => BinaryPrimitives.WriteUInt64BigEndian(bytes.AsSpan(offset), value);
    static byte[] Build(AsymmetricKeyParameter key, Action<byte[]> mutate = null)
    {
        using var stream = new MemoryStream();
        void U16(int value) { stream.WriteByte((byte)(value >> 8)); stream.WriteByte((byte)value); }
        void Field(int type, byte[] value) { U16(type); U16(value.Length); stream.Write(value); }
        byte[] Number(ulong value, int length)
        {
            var data = new byte[length]; for (int i = length - 1; i >= 0; i--) { data[i] = (byte)value; value >>= 8; } return data;
        }
        byte[] Padded(string value, int length) { var data = new byte[length]; Encoding.ASCII.GetBytes(value).CopyTo(data, 0); return data; }
        stream.Write(new byte[] { 0x21, 1, 0, 0, 0, 0, 0, 240 });
        U16(0x3000); U16(164);
        Field(8, new byte[20]); Field(1, Number(256, 4));
        Field(7, Number((ulong)(Now - 60) * 1000, 8)); Field(7, Number((ulong)(Now + 600) * 1000, 8));
        Field(2, Number(ulong.MaxValue, 8)); Field(4, Padded("Synthetic_Name", 32));
        Field(8, Padded("us", 4)); Field(8, Padded("un", 4)); Field(8, Padded(Service, 24));
        Field(1, new byte[4]); Field(0, Array.Empty<byte>()); Field(0, Array.Empty<byte>());
        U16(0x3002); U16(68); Field(8, new byte[] { 0x12, 0x34, 0xab, 0xcd });
        U16(8); U16(56);
        byte[] prefix = stream.ToArray();
        mutate?.Invoke(prefix);
        var signer = SignerUtilities.GetSigner("SHA-1withECDSA"); signer.Init(true, key); signer.BlockUpdate(prefix, 0, prefix.Length);
        byte[] signature = signer.GenerateSignature();
        var result = new byte[248]; prefix.CopyTo(result, 0); signature.CopyTo(result, SignatureOffset); return result;
    }
}
