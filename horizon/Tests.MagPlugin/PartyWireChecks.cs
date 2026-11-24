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
        ClientFieldOrder();
        AddedMessages();
    }

    // Fixtures are built by hand from the documented field order, not by the codec under test.
    static void ClientFieldOrder()
    {
        // 0x1A: party id, invitee, inviter.
        Check(MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("001A000000050000001300000011"), 20231, out var parsed) &&
            parsed is MagRetailPartyInvite { PartyId: 5, Invitee: 0x13, Inviter: 0x11 }, "Invite fields misread");
        // 0x1B as the client writes it: response, inviter, responder (self), party id.
        var answer = Convert.FromHexString("001B00000000110000001300000005");
        Check(MagMessageCodec.TryParseClientForApplication(answer, 20231, out parsed) &&
            parsed is MagRetailPartyInviteResponse { Response: 0, Inviter: 0x11, Responder: 0x13, PartyId: 5 },
            "Invite response not read in client field order");
        Check(MagMessageCodec.SerializeForApplication(new MagRetailPartyInviteResponse
            { Response = 1, Inviter = 0x11, Responder = 0x13, PartyId = 5 }, 20231)
            .SequenceEqual(Convert.FromHexString("001B01000000110000001300000005")), "Invite response written out of client order");
    }

    static void AddedMessages()
    {
        // 0x22: declined account, party id, reason byte (server to client only).
        var decline = Convert.FromHexString("0022000000130000000504");
        Check(MagMessageCodec.TryParseForApplication(decline, 20231, out var parsed) &&
            parsed is MagRetailPartyDecline { Declined: 0x13, PartyId: 5, Reason: 4 } &&
            MagMessageCodec.SerializeForApplication(parsed, 20231).SequenceEqual(decline), "Decline notice layout changed");
        Check(!MagMessageCodec.TryParseClientForApplication(decline, 20231, out _), "Client ingress accepted a decline notice");

        // 0x25: one flag bit, then the party id straddling bytes, zero padded to a byte.
        foreach (var (hex, open, party) in new[] { ("00258000000280", true, 5u), ("00254081018200", false, 0x81020304u) })
        {
            var bytes = Convert.FromHexString(hex);
            Check(MagMessageCodec.TryParseClientForApplication(bytes, 20231, out parsed) &&
                parsed is MagRetailPartyOpenInvite request && request.OpenInvite == open && request.PartyId == party &&
                MagMessageCodec.SerializeForApplication(parsed, 20231).SequenceEqual(bytes), "Open-invite bit layout changed");
        }
        Check(!MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("00258000000281"), 20231, out _) &&
            !MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("0025800000028000"), 20231, out _) &&
            !MagMessageCodec.TryParseClientForApplication(Convert.FromHexString("002580000002"), 20231, out _),
            "Malformed open-invite request accepted");

        // 0x26: party id only (client to server).
        var rejoin = Convert.FromHexString("002600000005");
        Check(MagMessageCodec.TryParseClientForApplication(rejoin, 20231, out parsed) &&
            parsed is MagRetailPartyRejoinRequest { PartyId: 5 } &&
            MagMessageCodec.SerializeForApplication(parsed, 20231).SequenceEqual(rejoin), "Voice rejoin request layout changed");

        // 0x27: party id then a world descriptor (57 bytes in all; server to client only).
        var voiceHex = "0027" + "00000005" + "00000102" + "0001" + "0002" + "0A000001" + new string('0', 24) + "2775" +
            "00000009" + "4B4B4B4B4B4B4B4B4B4B4B4B4B4B4B4B00" + "0000ABCD";
        var voice = Convert.FromHexString(voiceHex);
        Check(voice.Length == 57 && MagMessageCodec.TryParseForApplication(voice, 20231, out parsed) &&
            parsed is MagRetailPartyRejoinResponse { PartyId: 5, MediusWorldUid: 0x102, ConnectionType: 1, AddressType: 2,
                ServerPort: 0x2775, WorldId: 9, Piuid: 0xABCD } descriptor && descriptor.ServerAddress[0] == 10 &&
            descriptor.AccessKey[15] == 0x4B && descriptor.AccessKey[16] == 0 &&
            MagMessageCodec.SerializeForApplication(parsed, 20231).SequenceEqual(voice), "Voice descriptor layout changed");
        Check(!MagMessageCodec.TryParseClientForApplication(voice, 20231, out _), "Client ingress accepted a voice descriptor");

        // Party-range types the client never acts on stay unparsed and draw no reply.
        foreach (ushort type in new ushort[] { 0x0016, 0x0018, 0x0019, 0x001d, 0x001e, 0x0023, 0x0024 })
            Check(!MagMessageCodec.TryParseForApplication(Convert.FromHexString($"{type:X4}00000005"), 20231, out _),
                $"Unused party type {type:X4} parsed");
    }
}
