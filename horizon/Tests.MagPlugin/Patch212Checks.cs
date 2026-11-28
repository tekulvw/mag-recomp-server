using RT.Models.MAG;

// Hand-built synthetic packets. No captures or extracted game data.
internal static class Patch212Checks
{
    private static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    public static void Request()
    {
        // ID 01f5, count 2, levels 1 and uint.MaxValue, build B, language en.
        byte[] wire = Convert.FromHexString("01f5000200000001ffffffff0001420002656e");
        Check(MagPatch212Codec.TryParseRequest(wire, out var request) &&
            request.InstalledLevels.SequenceEqual(new uint[] { 1, uint.MaxValue }) &&
            request.BuildString == "B" && request.Language == "en", "2.12 vector request misread");
        // Legacy profile must not accept this vector packet.
        Check(!MagMessageCodec.TryParseClientForApplication(wire, 20231, out _), "2.12 packet leaked into 1740 codec");
        Check(MagPatch212Codec.TryParseRequest(Convert.FromHexString("01f5000000000000"), out var empty) &&
            empty.InstalledLevels.Length == 0 && empty.BuildString == "" && empty.Language == "", "Empty vector or strings rejected");
        for (int length = 0; length < wire.Length; length++)
            Check(!MagPatch212Codec.TryParseRequest(wire[..length], out var failed) && failed == null,
                "Truncated request accepted at " + length);
        Check(!MagPatch212Codec.TryParseRequest(wire.Append((byte)0).ToArray(), out _), "Extra zero byte accepted");
        Check(!MagPatch212Codec.TryParseRequest(wire.Append((byte)1).ToArray(), out _), "Extra nonzero byte accepted");
        foreach (string hex in new[] {
            "01f5ffff00000000", // Count cannot fit available payload.
            "01f5000300000001ffffffff0001420002656e", // Count eats string fields.
            "01f500000001ff0000", // Unsupported non-ASCII string.
            "01f500001001", // String exceeds shared bound.
            "01f6000000000000", // A response must not parse as a request.
            "01f500000000000000000001420002656e" // 1740 two-level request, not a vector.
        }) Check(!MagPatch212Codec.TryParseRequest(Convert.FromHexString(hex), out _), "Malformed/wrong-profile request accepted: " + hex);
        Check(!MagPatch212Codec.TryParseRequest(null, out _), "Null request accepted");
        Check(!MagPatch212Codec.TryParseRequest(new byte[10238], out _), "Oversized request accepted");
    }

    public static void Completion()
    {
        var replies = MagPatch212Codec.NoDownloadReply();
        Check(replies.Count == 2, "2.12 needs settings and a separate terminal URL");
        byte[] settings = Convert.FromHexString(
            "01f6" + "ffffffffffffffffffffffff" + "00002000" + "00" +
            "ffffffffffffffffffffffffffffffffffffffffffffffff");
        Check(replies[0].SequenceEqual(settings), "Settings contain a legacy count or wrong fields");
        Check(replies[1].SequenceEqual(Convert.FromHexString("01f70000000000000080")),
            "Empty file/size or MSB terminal flag changed");
        foreach (var reply in replies)
            Check(!MagPatch212Codec.TryParseRequest(reply, out _) &&
                !MagMessageCodec.TryParseClientForApplication(reply, 20231, out _), "Server reply accepted at client ingress");
        // Callers must not share mutable payload buffers across sessions.
        replies[1][^1] = 0;
        Check(MagPatch212Codec.NoDownloadReply()[1][^1] == 0x80, "Reply buffers shared across calls");
    }
}
