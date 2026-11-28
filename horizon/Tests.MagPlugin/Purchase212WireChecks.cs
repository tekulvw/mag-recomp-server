using RT.Models.MAG;

// Invented ordinals and records; no original catalog, account or captured bytes.
internal static class Purchase212WireChecks
{
    private static void Check(bool value, string message) {if (!value) throw new Exception(message);}
    private static void Reject(Action action)
    {try {action();} catch (ArgumentException) {return;} throw new Exception("Conflicting purchase profile accepted");}

    public static void Run()
    {
        foreach (short id in new short[] {short.MinValue, -1, 0, 127, 128, 159, 160, short.MaxValue})
        {
            // Plain type + signed16; no common RSA/transaction header.
            byte[] expected = {0x02, 0x68, unchecked((byte)(id >> 8)), unchecked((byte)id)};
            var request = new MagPurchaseSkill212Request {SkillId = id};
            Check(MagMessageCodec.SerializeForProtocol(request, 20231, 1793).SequenceEqual(expected), "Signed16 purchase bytes changed");
            Check(MagMessageCodec.TryParseForProtocol(expected, 20231, 1793, out var parsed) &&
                parsed is MagPurchaseSkill212Request p && p.SkillId == id, "Signed16 purchase ordinal lost");
            Check(!MagMessageCodec.TryParseClientForProtocol(expected, 20231, 1793, out _), "Unqualified purchase entered live dispatch");
            for (int end = 0; end < expected.Length; end++)
                Check(!MagMessageCodec.TryParseForProtocol(expected[..end], 20231, 1793, out _), "Truncated purchase accepted");
            Check(!MagMessageCodec.TryParseForProtocol(expected.Concat(new byte[] {0}).ToArray(), 20231, 1793, out _), "Trailing purchase byte accepted");
            Reject(() => MagMessageCodec.SerializeForProtocol(request, 20231, 1740));
            Reject(() => MagMessageCodec.SerializeForApplication(request, 20231));
            Reject(() => MagMessageCodec.Serialize(request));
        }
        Reject(() => MagMessageCodec.SerializeForProtocol(new MagPurchaseSkillRequest {SkillId = 127}, 20231, 1793));
        var oldWire = MagMessageCodec.SerializeForProtocol(new MagPurchaseSkillRequest {SkillId = 127}, 20231, 1740);
        Check(oldWire.SequenceEqual(new byte[] {0x02, 0x50, 0x7f}), "Legacy purchase ordinal widened");
        Check(MagMessageCodec.TryParseClientForProtocol(oldWire, 20231, 1740, out var old) && old is MagPurchaseSkillRequest {SkillId: 127},
            "Legacy purchase dispatch changed");
        Check(!MagMessageCodec.TryParseForProtocol(new byte[] {0x02, 0x68, 0, 1}, 20230, 1793, out _), "Wrong app accepted purchase");
        Check(!MagMessageCodec.TryParseForProtocol(new byte[] {0x02, 0x68, 0, 1}, 20231, 1794, out _), "Unknown profile guessed purchase");

        var record = new MagCharacterRecord {Faction = 0, SkillSpecialty = 0, Level = 0, LeadershipLevel = 0, LastZonesMux = 0,
            Loadouts = Enumerable.Range(0, 5).Select(_ => new MagLoadout {UpdateId = 0}).ToArray(),
            Patch212Unknown16 = new ushort[] {0x1234,0,0x5678,0,0,0xabcd},
            Patch212Unknown32 = new uint[] {0x89abcdef,0,1,2,uint.MaxValue}};
        // The independently checked synthetic record has1967bits plus one zero pad.
        // Prefix its valid bits with the response success bit: now exactly246bytes.
        byte[] recordBytes = new byte[214].Concat(Convert.FromHexString(
            "24680000acf000000001579b13579bde000000000000000200000005fffffffe")).ToArray();
        foreach (bool success in new[] {false, true})
        {
            var expected = new byte[248]; expected[0] = 0x02; expected[1] = 0x69;
            for (int i = 0; i < recordBytes.Length; i++)
                expected[i + 2] = (byte)((recordBytes[i] >> 1) | (i == 0 ? 0 : recordBytes[i - 1] << 7));
            if (success) expected[2] |= 0x80;
            var wire = MagMessageCodec.SerializeForProtocol(new MagPurchaseSkillResponse {Success = success, Record = record}, 20231, 1793);
            Check(wire.SequenceEqual(expected), "Purchase response record is conditional or misaligned");
            Check(MagMessageCodec.TryParseForProtocol(wire, 20231, 1793, out var message) &&
                message is MagPurchaseSkillResponse p && p.Success == success &&
                p.Record.Patch212Unknown16.SequenceEqual(record.Patch212Unknown16) &&
                p.Record.Patch212Unknown32.SequenceEqual(record.Patch212Unknown32), "Purchase response lost its1793 record");
            Check(!MagMessageCodec.TryParseForProtocol(wire[..^1], 20231, 1793, out _), "Truncated response record accepted");
            Check(!MagMessageCodec.TryParseForProtocol(new byte[] {0x02, 0x69, success ? (byte)0x80 : (byte)0}, 20231, 1793, out _),
                "Purchase response accepted without its unconditional record");
            Check(!MagMessageCodec.TryParseClientForProtocol(wire, 20231, 1793, out _), "Server response entered client dispatch");
        }
    }
}
