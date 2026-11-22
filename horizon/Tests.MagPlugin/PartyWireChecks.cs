using RT.Models.MAG;

static class PartyWireChecks
{
    static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    public static void Run()
    {
        var invite = Convert.FromHexString("001A010203041122334455667788");
        Check(MagMessageCodec.TryParseClientForApplication(invite, 20231, out var parsed),
            "Retail compact invite refused");
        Check(MagMessageCodec.SerializeForApplication(parsed, 20231).SequenceEqual(invite),
            "Retail invite did not retain the compact body");
        Check(!MagMessageCodec.TryParseClientForApplication(invite, 20230, out _),
            "Prototype accepted a retail invite");

        foreach (var hex in new[] { "001B00010203041122334455667788", "001C01020304" })
        {
            var request = Convert.FromHexString(hex);
            Check(MagMessageCodec.TryParseClientForApplication(request, 20231, out parsed) &&
                MagMessageCodec.SerializeForApplication(parsed, 20231).SequenceEqual(request),
                "Retail party request body changed on round trip");
            Check(!MagMessageCodec.TryParseClientForApplication(request[..^1], 20231, out _) &&
                !MagMessageCodec.TryParseClientForApplication(request.Append((byte)0).ToArray(), 20231, out _),
                "Wrong compact party request length accepted");
        }

        foreach (var hex in new[] { "001F01020304", "002001020304", "0021000255667788112233440102030402FF80" })
        {
            var update = Convert.FromHexString(hex);
            Check(MagMessageCodec.TryParseForApplication(update, 20231, out parsed) &&
                MagMessageCodec.SerializeForApplication(parsed, 20231).SequenceEqual(update),
                "Retail party event body changed on round trip");
            Check(!MagMessageCodec.TryParseClientForApplication(update, 20231, out _),
                "Client ingress accepted a server-owned party event");
        }
        Check(!MagMessageCodec.TryParseForApplication(Convert.FromHexString("0021000255667788112233440102030402FF81"), 20231, out _),
            "Nonzero party roster padding accepted");
    }
}
