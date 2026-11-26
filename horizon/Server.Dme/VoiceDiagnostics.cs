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
    /// each signature with their leading bytes and where a lab voice block starts, then one sample in
    /// every <see cref="SampleEvery"/> for as long as the signature lasts, and logs the counts every
    /// ten seconds. Messages that parse as voice packets (<see cref="VoicePacketHeader"/>) also feed
    /// a stream per client and group, whose counts line carries the data bytes, the data length
    /// range, packets holding more than one lab block, and sequence gaps and steps back. It only
    /// observes: traffic is never changed, delayed or dropped.
    /// </summary>
    public sealed class VoiceDiagnostics
    {
        public const string Variable = "MAG_VOICE_DIAG";
        public const int SamplesPerSignature = 20;
        /// <summary>After the first samples, one sample per this many messages of a signature
        /// (about one every eight seconds for one voice group).</summary>
        public const int SampleEvery = 100;
        public const int MaxStreams = 512;
        public const int MaxSignatures = 512;
        /// <summary>Once <see cref="MaxSignatures"/> are in use, a new signature is counted and sampled
        /// under its client, transport, message type and plugin with "lead=other", up to this many such
        /// buckets, so traffic that starts late in a long-lived process stays visible.</summary>
        public const int MaxOtherSignatures = 256;
        const int OtherLead = -2;
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
        readonly Dictionary<(int client, int group), VoiceStream> _streams = new();
        long _overflow;
        int _otherSignatures;
        long _streamOverflow;

        sealed class Signature { public long Total; public long SinceLast; public int Samples; }

        sealed class VoiceStream
        {
            public long Total, Recent, RecentBytes, TotalBytes, RecentMultiBlock, RecentGaps, TotalGaps, RecentBack, TotalBack;
            public long RecentMismatch, TotalMismatch;
            public int RecentMismatchBodyMax = -1;
            public int RecentMin = int.MaxValue, RecentMax = -1;
            public int? LastSequence;

            public void Add(VoicePacketHeader header)
            {
                Total++; Recent++; RecentBytes += header.DataLength; TotalBytes += header.DataLength;
                RecentMin = Math.Min(RecentMin, header.DataLength); RecentMax = Math.Max(RecentMax, header.DataLength);
                if (header.DataLength > LabBlockSize) RecentMultiBlock++;
                if (LastSequence is int last)
                {
                    int step = (header.Sequence - last) & 0xFFFF;
                    if (step == 0 || step >= 0x8000) { RecentBack++; TotalBack++; }
                    else { RecentGaps += step - 1; TotalGaps += step - 1; }
                }
                LastSequence = header.Sequence;
            }

            /// <summary>A packet with the voice kind and matching group bytes whose data length field
            /// does not match its size (for example several packets in one message).</summary>
            public void AddMismatch(int bodyLength)
            {
                RecentMismatch++; TotalMismatch++;
                RecentMismatchBodyMax = Math.Max(RecentMismatchBodyMax, bodyLength);
            }

            public void Reset()
            {
                Recent = RecentBytes = RecentMultiBlock = RecentGaps = RecentBack = RecentMismatch = 0;
                RecentMismatchBodyMax = -1;
                RecentMin = int.MaxValue; RecentMax = -1;
            }
        }

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
                    if (_signatures.Count < MaxSignatures) _signatures[key] = signature = new Signature();
                    else
                    {
                        key = (client, transport, rtType, plugin.HasValue ? plugin.Value : -1, OtherLead);
                        if (_signatures.TryGetValue(key, out signature)) { }
                        else if (_otherSignatures < MaxOtherSignatures) { _signatures[key] = signature = new Signature(); _otherSignatures++; }
                        else _overflow++;
                    }
                }
                if (signature != null)
                {
                    signature.Total++;
                    signature.SinceLast++;
                    if (signature.Samples < SamplesPerSignature || signature.Total % SampleEvery == 0)
                    {
                        signature.Samples++;
                        var shown = Math.Min(body.Length, SampleBytes);
                        sample = $"voice_diag sample ms={nowMs} client={client} transport={transport} rt={rtType} " +
                                 $"plugin={(plugin.HasValue ? plugin.Value.ToString() : "-")} length={body.Length} " +
                                 $"lead=0x{(lead < 0 ? 0 : lead):X4} block_at={FindVoiceBlock(body, LabBlockSize)} " +
                                 $"bytes={Convert.ToHexString(body, 0, shown)}";
                    }
                }
                bool parsed = VoicePacketHeader.TryParse(body, out var header);
                if (parsed || VoicePacketHeader.HasVoiceKind(body))
                {
                    var streamKey = (client, parsed ? header.Group : body[6]);
                    if (!_streams.TryGetValue(streamKey, out var stream))
                    {
                        if (_streams.Count >= MaxStreams) { _streamOverflow++; stream = null; }
                        else _streams[streamKey] = stream = new VoiceStream();
                    }
                    if (parsed) stream?.Add(header);
                    else stream?.AddMismatch(body.Length);
                }
                if (_nextCounts < 0) _nextCounts = nowMs + CountIntervalMs;
                else if (nowMs >= _nextCounts)
                {
                    _nextCounts = nowMs + CountIntervalMs;
                    counts = _signatures.Where(s => s.Value.SinceLast > 0)
                        .OrderByDescending(s => s.Value.SinceLast)
                        .Select(s => $"voice_diag counts ms={nowMs} client={s.Key.client} transport={s.Key.transport} rt={s.Key.rt} " +
                                     $"plugin={(s.Key.plugin < 0 ? "-" : s.Key.plugin.ToString())} lead={(s.Key.lead == OtherLead ? "other" : $"0x{(s.Key.lead < 0 ? 0 : s.Key.lead):X4}")} " +
                                     $"recent={s.Value.SinceLast} total={s.Value.Total}")
                        .ToList();
                    if (_overflow > 0) counts.Add($"voice_diag counts ms={nowMs} unrecorded_signatures={_overflow}");
                    counts.AddRange(_streams.Where(s => s.Value.Recent > 0).OrderBy(s => s.Key.client).ThenBy(s => s.Key.group)
                        .Select(s => $"voice_diag counts ms={nowMs} client={s.Key.client} voice_group={s.Key.group} " +
                                     $"recent={s.Value.Recent} total={s.Value.Total} data_bytes={s.Value.RecentBytes} " +
                                     $"data_bytes_total={s.Value.TotalBytes} data_min={s.Value.RecentMin} data_max={s.Value.RecentMax} " +
                                     $"multi_block={s.Value.RecentMultiBlock} seq_gaps={s.Value.RecentGaps} seq_gaps_total={s.Value.TotalGaps} " +
                                     $"seq_back={s.Value.RecentBack} seq_back_total={s.Value.TotalBack} " +
                                     $"length_mismatch={s.Value.RecentMismatch} length_mismatch_total={s.Value.TotalMismatch} " +
                                     $"mismatch_body_max={s.Value.RecentMismatchBodyMax}"));
                    if (_streamOverflow > 0) counts.Add($"voice_diag counts ms={nowMs} unrecorded_voice_streams={_streamOverflow}");
                    foreach (var s in _signatures.Values) s.SinceLast = 0;
                    foreach (var s in _streams.Values) s.Reset();
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
