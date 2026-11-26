using DotNetty.Common.Internal.Logging;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using RT.Models;
using Server.Common.Stream;

namespace Server.Dme
{
    /// <summary>
    /// Lab receipts for the voice capture (docs/superpowers/specs/2026-11-25-voice-relay-design.md).
    /// Off unless MAG_VOICE_DIAG=1; never set on live. For each client message it keeps a count per
    /// signature (client, transport message, plugin, first two body bytes), logs the first samples of
    /// each signature with their leading bytes and where a lab voice block starts, and logs the counts
    /// every ten seconds. It only observes: traffic is never changed, delayed or dropped.
    /// </summary>
    public sealed class VoiceDiagnostics
    {
        public const string Variable = "MAG_VOICE_DIAG";
        public const int SamplesPerSignature = 20;
        public const int MaxSignatures = 512;
        public const int SampleBytes = 64;
        public const long CountIntervalMs = 10_000;
        // The lab client's voice block (mag-recomp src/voice): marker, sequence, length, CRC-8, payload,
        // zero padding, 37 bytes at the game's 7300 bps port rate.
        public const byte BlockMarker = 0xA1;
        public const int BlockHeader = 4;
        public const int LabBlockSize = 37;

        static readonly IInternalLogger Logger = InternalLoggerFactory.GetInstance<VoiceDiagnostics>();
        public static VoiceDiagnostics Instance { get; set; } = FromEnvironment(Environment.GetEnvironmentVariable, line => Logger.Info(line));

        readonly Action<string> _sink;
        readonly object _gate = new object();
        readonly Dictionary<(int client, string transport, byte rt, int plugin, int lead), Signature> _signatures = new();
        long _nextCounts = -1;
        long _overflow;

        sealed class Signature { public long Total; public long SinceLast; public int Samples; }

        public bool Enabled { get; }

        public VoiceDiagnostics(bool enabled, Action<string> sink)
        {
            Enabled = enabled;
            _sink = sink ?? (_ => { });
        }

        public static VoiceDiagnostics FromEnvironment(Func<string, string> environment, Action<string> sink) =>
            new VoiceDiagnostics(environment?.Invoke(Variable) == "1", sink);

        /// <summary>Observes one decoded client message; serializes it only when enabled.</summary>
        public void Observe(int client, string transport, BaseScertMessage message, int mediusVersion, long nowMs)
        {
            if (!Enabled || message == null) return;
            try
            {
                byte? plugin = null;
                byte[] body;
                if (message is RT_MSG_CLIENT_APP_TO_PLUGIN toPlugin)
                {
                    plugin = toPlugin.PluginId;
                    body = toPlugin.Payload ?? Array.Empty<byte>();
                }
                else
                {
                    using var stream = new MemoryStream();
                    using (var writer = new MessageWriter(stream, Encoding.UTF8, true) { MediusVersion = mediusVersion })
                        message.Serialize(writer);
                    body = stream.ToArray();
                }
                Observe(client, transport, (byte)message.Id, plugin, body, nowMs);
            }
            catch (Exception error)
            {
                _sink($"voice_diag error={error.GetType().Name}");
            }
        }

        public void Observe(int client, string transport, byte rtType, byte? plugin, byte[] body, long nowMs)
        {
            if (!Enabled) return;
            body ??= Array.Empty<byte>();
            var lead = body.Length >= 2 ? (body[0] << 8) | body[1] : body.Length == 1 ? body[0] : -1;
            var key = (client, transport, rtType, plugin.HasValue ? plugin.Value : -1, lead);
            string sample = null;
            List<string> counts = null;
            lock (_gate)
            {
                if (!_signatures.TryGetValue(key, out var signature))
                {
                    if (_signatures.Count >= MaxSignatures) { _overflow++; signature = null; }
                    else _signatures[key] = signature = new Signature();
                }
                if (signature != null)
                {
                    signature.Total++;
                    signature.SinceLast++;
                    if (signature.Samples < SamplesPerSignature)
                    {
                        signature.Samples++;
                        var shown = Math.Min(body.Length, SampleBytes);
                        sample = $"voice_diag sample ms={nowMs} client={client} transport={transport} rt={rtType} " +
                                 $"plugin={(plugin.HasValue ? plugin.Value.ToString() : "-")} length={body.Length} " +
                                 $"lead=0x{(lead < 0 ? 0 : lead):X4} block_at={FindVoiceBlock(body, LabBlockSize)} " +
                                 $"bytes={Convert.ToHexString(body, 0, shown)}";
                    }
                }
                if (_nextCounts < 0) _nextCounts = nowMs + CountIntervalMs;
                else if (nowMs >= _nextCounts)
                {
                    _nextCounts = nowMs + CountIntervalMs;
                    counts = _signatures.Where(s => s.Value.SinceLast > 0)
                        .OrderByDescending(s => s.Value.SinceLast)
                        .Select(s => $"voice_diag counts ms={nowMs} client={s.Key.client} transport={s.Key.transport} rt={s.Key.rt} " +
                                     $"plugin={(s.Key.plugin < 0 ? "-" : s.Key.plugin.ToString())} lead=0x{(s.Key.lead < 0 ? 0 : s.Key.lead):X4} " +
                                     $"recent={s.Value.SinceLast} total={s.Value.Total}")
                        .ToList();
                    if (_overflow > 0) counts.Add($"voice_diag counts ms={nowMs} unrecorded_signatures={_overflow}");
                    foreach (var s in _signatures.Values) s.SinceLast = 0;
                }
            }
            if (sample != null) _sink(sample);
            if (counts != null) foreach (var line in counts) _sink(line);
        }

        /// <summary>CRC-8 (x^8 + x^2 + x + 1) over the sequence, the length and the payload.</summary>
        public static byte BlockCheck(byte sequence, ReadOnlySpan<byte> payload)
        {
            byte crc = 0;
            void Feed(byte value)
            {
                crc ^= value;
                for (var bit = 0; bit < 8; bit++) crc = (byte)((crc & 0x80) != 0 ? (crc << 1) ^ 0x07 : crc << 1);
            }
            Feed(sequence);
            Feed((byte)payload.Length);
            foreach (var b in payload) Feed(b);
            return crc;
        }

        /// <summary>Offset of the first complete lab voice block in <paramref name="body"/>, or -1.</summary>
        public static int FindVoiceBlock(byte[] body, int blockSize)
        {
            if (body == null || blockSize <= BlockHeader) return -1;
            for (var at = 0; at + blockSize <= body.Length; at++)
            {
                if (body[at] != BlockMarker) continue;
                var length = body[at + 2];
                if (length > blockSize - BlockHeader) continue;
                var payload = new ReadOnlySpan<byte>(body, at + BlockHeader, length);
                if (body[at + 3] != BlockCheck(body[at + 1], payload)) continue;
                var padded = true;
                for (var i = at + BlockHeader + length; i < at + blockSize; i++) if (body[i] != 0) { padded = false; break; }
                if (padded) return at;
            }
            return -1;
        }
    }
}
