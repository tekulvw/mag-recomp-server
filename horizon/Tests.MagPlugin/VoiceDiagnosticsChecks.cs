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
