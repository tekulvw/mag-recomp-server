using System;
using System.Collections.Generic;
using System.Linq;
using Server.Dme;

// Synthetic voice-diagnostic checks: invented clients and payloads, no capture or game data.
internal static class VoiceDiagnosticsChecks
{
    static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }

    // A lab block as the mag-recomp voice service writes it: marker, sequence, length, CRC-8, payload, padding.
    static byte[] Block(byte sequence, byte[] payload, int size)
    {
        var block = new byte[size];
        block[0] = VoiceDiagnostics.BlockMarker;
        block[1] = sequence;
        block[2] = (byte)payload.Length;
        block[3] = VoiceDiagnostics.BlockCheck(sequence, payload);
        Array.Copy(payload, 0, block, VoiceDiagnostics.BlockHeader, payload.Length);
        return block;
    }

    public static void Run()
    {
        // The check byte matches the client's CRC-8 for a known vector: sequence 0, empty payload.
        Check(VoiceDiagnostics.BlockCheck(0, ReadOnlySpan<byte>.Empty) == 0x00, "CRC-8 of zero bytes");
        Check(VoiceDiagnostics.BlockCheck(1, new byte[] { 0x02 }) == CrcReference(new byte[] { 1, 1, 2 }), "CRC-8 reference");

        var payload = Enumerable.Range(0, 33).Select(i => (byte)(7 + i)).ToArray();
        var block = Block(7, payload, VoiceDiagnostics.LabBlockSize);
        var framed = new byte[] { 0x01, 0x02, 0xA1, 0xFF, 0x00 }.Concat(block).Concat(new byte[] { 0x10 }).ToArray();
        Check(VoiceDiagnostics.FindVoiceBlock(framed, VoiceDiagnostics.LabBlockSize) == 5, "block found after a 5-byte header");
        Check(VoiceDiagnostics.FindVoiceBlock(block[..30], VoiceDiagnostics.LabBlockSize) == -1, "a truncated block is not reported");
        var corrupt = (byte[])block.Clone(); corrupt[10] ^= 0x40;
        Check(VoiceDiagnostics.FindVoiceBlock(corrupt, VoiceDiagnostics.LabBlockSize) == -1, "a corrupt block fails its check");
        var shortPayload = Block(3, new byte[] { 1, 2, 3 }, VoiceDiagnostics.LabBlockSize);
        Check(VoiceDiagnostics.FindVoiceBlock(shortPayload, VoiceDiagnostics.LabBlockSize) == 0, "padded block found");
        shortPayload[20] = 9;
        Check(VoiceDiagnostics.FindVoiceBlock(shortPayload, VoiceDiagnostics.LabBlockSize) == -1, "nonzero padding rejected");

        // Off unless MAG_VOICE_DIAG=1, and off means nothing is written.
        var lines = new List<string>();
        foreach (var value in new string[] { null, "", "0", "true", " 1" })
        {
            var off = VoiceDiagnostics.FromEnvironment(name => name == VoiceDiagnostics.Variable ? value : null, lines.Add);
            Check(!off.Enabled, $"enabled by {value ?? "null"}");
            off.Observe(1, "udp", 61, 3, framed, 0);
        }
        Check(lines.Count == 0, "disabled diagnostics wrote lines");

        var on = VoiceDiagnostics.FromEnvironment(name => name == VoiceDiagnostics.Variable ? "1" : null, lines.Add);
        Check(on.Enabled, "MAG_VOICE_DIAG=1 did not enable");
        for (var i = 0; i < 50; i++) on.Observe(2, "udp", 61, 3, framed, 100 + i);
        var samples = lines.Where(l => l.StartsWith("voice_diag sample")).ToList();
        Check(samples.Count == VoiceDiagnostics.SamplesPerSignature, $"samples per signature capped: {samples.Count}");
        Check(samples[0].Contains("client=2") && samples[0].Contains("rt=61") && samples[0].Contains("plugin=3") &&
              samples[0].Contains($"length={framed.Length}") && samples[0].Contains("lead=0x0102") && samples[0].Contains("block_at=5"),
              "sample fields: " + samples[0]);
        Check(samples[0].Contains("bytes=" + Convert.ToHexString(framed)), "sample bytes");
        // Another signature gets its own samples; counts follow after the interval.
        on.Observe(2, "tcp", 2, null, new byte[] { 0x55 }, 200);
        Check(lines.Count(l => l.StartsWith("voice_diag sample")) == VoiceDiagnostics.SamplesPerSignature + 1, "second signature sampled");
        Check(!lines.Any(l => l.StartsWith("voice_diag counts")), "counts before the interval");
        on.Observe(2, "udp", 61, 3, framed, 100 + VoiceDiagnostics.CountIntervalMs + 1);
        var counts = lines.Where(l => l.StartsWith("voice_diag counts")).ToList();
        Check(counts.Count == 2 && counts[0].Contains("recent=51") && counts[0].Contains("total=51") && counts[1].Contains("rt=2"),
              "counts: " + string.Join(" | ", counts));
        // Long samples show at most SampleBytes bytes.
        var big = new byte[300];
        on.Observe(9, "udp", 61, 3, big, 50_000);
        var bigSample = lines.Last(l => l.StartsWith("voice_diag sample"));
        Check(bigSample.Contains("length=300") && bigSample.Contains("bytes=" + new string('0', VoiceDiagnostics.SampleBytes * 2) + "") &&
              !bigSample.Contains(new string('0', VoiceDiagnostics.SampleBytes * 2 + 2)), "sample truncated to SampleBytes");
        // Signatures are bounded.
        var bounded = VoiceDiagnostics.FromEnvironment(name => name == VoiceDiagnostics.Variable ? "1" : null, _ => { });
        for (var client = 0; client < VoiceDiagnostics.MaxSignatures + 10; client++) bounded.Observe(client, "udp", 61, 3, framed, 0);

        Streams();
    }

    // A synthetic voice packet in the observed client layout: 14-byte header, then the data.
    internal static byte[] VoicePacket(int group, ushort sequence, int dataLength, byte fill = 0x5A)
    {
        var packet = new byte[VoicePacketHeader.Length + dataLength];
        packet[0] = 0x81; packet[1] = (byte)group; packet[3] = 1;
        packet[4] = 0; packet[5] = (byte)VoicePacketHeader.Kind;
        packet[6] = (byte)group; packet[7] = 1;
        packet[8] = (byte)(dataLength >> 8); packet[9] = (byte)dataLength;
        packet[10] = (byte)(sequence >> 8); packet[11] = (byte)sequence;
        for (var i = VoicePacketHeader.Length; i < packet.Length; i++) packet[i] = fill;
        return packet;
    }

    static void Streams()
    {
        // Sampling continues at a steady low rate after the first samples.
        var lines = new List<string>();
        var diag = new VoiceDiagnostics(true, lines.Add);
        for (var i = 1; i <= 3 * VoiceDiagnostics.SampleEvery; i++)
            diag.Observe(4, "udp", 53, null, VoicePacket(40, (ushort)i, VoiceDiagnostics.LabBlockSize), i);
        var samples = lines.Count(l => l.StartsWith("voice_diag sample"));
        Check(samples == VoiceDiagnostics.SamplesPerSignature + 3, $"steady sampling: {samples}");

        // Per client and group: data bytes and range, multi-block packets, sequence gaps and steps back.
        lines.Clear();
        diag = new VoiceDiagnostics(true, lines.Add);
        void Send(int client, int group, ushort sequence, int length, long ms) =>
            diag.Observe(client, "udp", 53, null, VoicePacket(group, sequence, length), ms);
        Send(4, 40, 1, 37, 0);           // the first message arms the counts interval
        Send(4, 40, 2, 37, 10);
        Send(4, 40, 5, 74, 20);          // two lost (3, 4) and a two-block packet
        Send(4, 40, 5, 37, 30);          // a repeat
        Send(4, 40, 0xFFFF, 37, 40);     // a step back
        Send(4, 40, 0x0001, 37, 50);     // a wrap: one lost (0)
        Send(4, 83, 9, 37, 60);          // another group of the same client is its own stream
        Send(5, 40, 7, 37, 70);          // and another client
        diag.Observe(4, "udp", 53, null, new byte[] { 0x81, 40, 0, 1, 0, 0x2E, 40, 1, 0, 1, 0, 1, 0, 0, 9 }, 80); // not voice
        diag.Observe(4, "udp", 53, null, VoicePacket(40, 2, 37)[..30], 90);                                     // length mismatch
        Send(4, 40, 2, 37, VoiceDiagnostics.CountIntervalMs + 1);
        var stream = lines.Where(l => l.Contains("voice_group=")).ToList();
        Check(stream.Count == 3, "stream lines: " + string.Join(" | ", stream));
        var first = stream.Single(l => l.Contains("client=4 voice_group=40 "));
        Check(first.Contains("recent=7 ") && first.Contains("total=7 ") && first.Contains($"data_bytes={37 * 6 + 74} ") &&
              first.Contains("data_min=37 ") && first.Contains("data_max=74 ") && first.Contains("multi_block=1 ") &&
              first.Contains("seq_gaps=3 ") && first.Contains("seq_back=2 "), "stream counters: " + first);
        Check(stream.Any(l => l.Contains("client=4 voice_group=83 recent=1 ")) && stream.Any(l => l.Contains("client=5 voice_group=40 recent=1 ")),
              "streams per client and group: " + string.Join(" | ", stream));
        // Recent counters restart each interval; totals carry on.
        lines.Clear();
        Send(4, 40, 3, 37, 2 * VoiceDiagnostics.CountIntervalMs + 2);
        var next = lines.Single(l => l.Contains("voice_group="));
        Check(next.Contains("client=4 voice_group=40 recent=1 total=8 ") && next.Contains("seq_gaps=0 seq_gaps_total=3 ") &&
              next.Contains("seq_back=0 seq_back_total=2"), "next interval: " + next);
        Check(VoicePacketHeader.TryParse(VoicePacket(83, 2, 37), out var header) && header.Group == 83 && header.MediaType == 1 &&
              header.DataLength == 37 && header.Sequence == 2, "voice header parse");
        var mismatch = VoicePacket(36, 1, 37); mismatch[6] = 37;
        Check(!VoicePacketHeader.TryParse(mismatch, out _), "the two group bytes must agree");
        Check(!VoicePacketHeader.TryParse(VoicePacket(36, 1, 37)[..13], out _), "a short header is not voice");
        Check(VoicePacketHeader.TryParse(VoicePacket(36, 1, 0), out header) && header.DataLength == 0, "an empty voice packet");
    }

    static byte CrcReference(byte[] bytes)
    {
        byte crc = 0;
        foreach (var b in bytes)
        {
            crc ^= b;
            for (var bit = 0; bit < 8; bit++) crc = (byte)((crc & 0x80) != 0 ? (crc << 1) ^ 0x07 : crc << 1);
        }
        return crc;
    }
}
