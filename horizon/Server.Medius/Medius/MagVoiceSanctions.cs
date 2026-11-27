using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading;

namespace Server.Medius
{
    /// <summary>
    /// The active voice sanctions main Medius holds in memory for the DME relay
    /// (docs/superpowers/specs/2026-11-25-voice-relay-design.md, "Moderation"). The relay asks it once per
    /// voice packet, which costs one dictionary lookup on an immutable snapshot and never touches the store.
    /// The snapshot is reloaded every <see cref="RefreshInterval"/> (an unchanged JSON file is not reread)
    /// and at once after the admin API applies a change; expiry is checked at each lookup, so an ended
    /// sanction stops dropping voice on time without a reload. A failed reload keeps the last snapshot.
    /// </summary>
    public sealed class VoiceSanctionCache : IDisposable
    {
        public static readonly TimeSpan RefreshInterval = TimeSpan.FromSeconds(5);
        static readonly IReadOnlyDictionary<string, VoiceSanction[]> Empty = new Dictionary<string, VoiceSanction[]>(StringComparer.Ordinal);

        /// <summary>The cache the relay consults; null (no sanctions) unless main Medius runs the voice relay.</summary>
        public static VoiceSanctionCache Current { get; set; }

        readonly LocalCharacterStore _store;
        readonly Func<DateTimeOffset> _clock;
        readonly Action<string> _log;
        readonly object _refresh = new object();
        volatile IReadOnlyDictionary<string, VoiceSanction[]> _active = Empty;
        string _token;
        bool _loaded, _failing;
        string _lastSummary;
        Timer _timer;

        public VoiceSanctionCache(LocalCharacterStore store, Func<DateTimeOffset> clock = null, Action<string> log = null)
        {
            _store = store ?? throw new ArgumentNullException(nameof(store));
            _clock = clock ?? (() => DateTimeOffset.UtcNow);
            _log = log ?? (_ => { });
        }

        /// <summary>Accounts with a sanction active now in the last snapshot.</summary>
        public int Count { get { var now = _clock(); return _active.Count(e => e.Value.Any(s => s.ActiveAt(now))); } }

        /// <summary>The sanction silencing <paramref name="account"/> now (a ban before a mute), or null.</summary>
        public VoiceSanction Silencing(string account)
        {
            if (account == null || !_active.TryGetValue(account, out var sanctions)) return null;
            var now = _clock();
            foreach (var sanction in sanctions) if (sanction.ActiveAt(now)) return sanction;
            return null;
        }

        /// <summary>Reload the snapshot. <paramref name="force"/> rereads even an unchanged JSON file.
        /// Returns false when the store could not be read; the previous snapshot then stays.</summary>
        public bool Refresh(bool force = false)
        {
            lock (_refresh)
            {
                try
                {
                    string token = _store.VoiceSanctionsChangeToken();
                    var now = _clock();
                    // An unchanged JSON file is not reread; its ended sanctions just leave the snapshot.
                    var source = !force && _loaded && token != null && token == _token
                        ? _active.Values.SelectMany(s => s) : _store.ReadVoiceSanctions();
                    _active = source.Where(s => s.ActiveAt(now))
                        .GroupBy(s => s.Account, StringComparer.Ordinal)
                        .ToDictionary(g => g.Key, g => g.OrderByDescending(s => s.Kind).ToArray(), StringComparer.Ordinal);
                    _token = token;
                    _loaded = true;
                    if (_failing) _log("Voice sanctions: store readable again.");
                    _failing = false;
                    string summary = Describe();
                    if (summary != _lastSummary) { _log("Voice sanctions: " + summary); _lastSummary = summary; }
                    return true;
                }
                catch (Exception e) when (e is IOException || e is InvalidDataException || e is UnauthorizedAccessException)
                {
                    if (!_failing) _log($"Voice sanctions: reload failed, keeping the last {_active.Count} account(s): {e.Message}");
                    _failing = true;
                    return false;
                }
            }
        }

        /// <summary>Counts only, never accounts or reasons.</summary>
        public string Describe()
        {
            var now = _clock();
            var all = _active.Values.SelectMany(s => s).Where(s => s.ActiveAt(now)).ToArray();
            return $"{all.Select(s => s.Account).Distinct(StringComparer.Ordinal).Count()} account(s) silenced ({all.Count(s => s.Kind == VoiceSanctionKind.Mute)} mute(s), " +
                $"{all.Count(s => s.Kind == VoiceSanctionKind.Ban)} ban(s))";
        }

        /// <summary>Load now, then every <see cref="RefreshInterval"/>.</summary>
        public void Start()
        {
            Refresh(force: true);
            _timer ??= new Timer(_ => Refresh(), null, RefreshInterval, RefreshInterval);
        }

        public void Dispose() { _timer?.Dispose(); _timer = null; }
    }

    /// <summary>Why a voice moderation command did not change anything (<see cref="None"/> for planned or applied).</summary>
    public enum MagVoiceRefusal { None, NotFound, NotSanctioned, InvalidRequest, Failed }

    public sealed class MagVoiceResult
    {
        public MagVoiceRefusal Refusal { get; init; }
        public string Message { get; init; }
        public VoiceSanctionChange Change { get; init; }
        /// <summary>Stable API code for a refusal; null for none.</summary>
        public string Code => Refusal switch
        {
            MagVoiceRefusal.NotFound => "not_found",
            MagVoiceRefusal.NotSanctioned => "not_sanctioned",
            MagVoiceRefusal.InvalidRequest => "invalid_request",
            MagVoiceRefusal.Failed => "failed",
            _ => null
        };
    }

    /// <summary>
    /// Voice moderation shared by <c>mag-admin voice …</c> and the admin API. The rules live in
    /// <see cref="LocalCharacterStore.ChangeVoiceSanction"/>; this maps its refusals and parses durations.
    /// </summary>
    public static class MagVoiceModeration
    {
        /// <summary>
        /// Parses a duration such as <c>30m</c>, <c>12h</c>, <c>7d</c> or <c>2w</c> (a whole number and one unit),
        /// from one minute to <see cref="LocalCharacterStore.MaximumVoiceSanctionDuration"/>.
        /// </summary>
        public static bool TryParseDuration(string text, out TimeSpan duration)
        {
            duration = default;
            if (string.IsNullOrEmpty(text) || text.Length < 2) return false;
            if (!long.TryParse(text.AsSpan(0, text.Length - 1), NumberStyles.None, CultureInfo.InvariantCulture, out long count) || count < 1)
                return false;
            double minutes = text[^1] switch { 'm' => 1, 'h' => 60, 'd' => 1440, 'w' => 10080, _ => 0 };
            if (minutes == 0 || count * minutes > LocalCharacterStore.MaximumVoiceSanctionDuration.TotalMinutes) return false;
            duration = TimeSpan.FromMinutes(count * minutes);
            return true;
        }

        /// <summary>The active sanctions at <paramref name="now"/>, ordered by account, a ban before a mute.</summary>
        public static IReadOnlyList<VoiceSanction> Active(LocalCharacterStore store, DateTimeOffset now) =>
            store.ReadVoiceSanctions().Where(s => s.ActiveAt(now))
                .OrderBy(s => s.Account, StringComparer.Ordinal).ThenByDescending(s => s.Kind).ToArray();

        public static MagVoiceResult Change(LocalCharacterStore store, string account, VoiceSanctionKind kind, bool lift, string reason,
            string operatorName, TimeSpan? duration, bool apply, string backupDirectory)
        {
            var change = store.ChangeVoiceSanction(account, kind, lift, reason, operatorName, duration, apply, backupDirectory);
            string kindName = VoiceSanction.Name(kind);
            return change.Refusal switch
            {
                null => new MagVoiceResult { Refusal = MagVoiceRefusal.None, Change = change },
                "account-not-found" => Refuse(MagVoiceRefusal.NotFound, $"account-not-found: account {account} has no character", change),
                "not-sanctioned" => Refuse(MagVoiceRefusal.NotSanctioned, $"not-sanctioned: {account} has no active voice {kindName}", change),
                "invalid-reason" => Refuse(MagVoiceRefusal.InvalidRequest,
                    $"invalid-reason: the reason must be printable ASCII of at most {MagAdminKill.MaximumReasonLength} characters", change),
                "invalid-operator" => Refuse(MagVoiceRefusal.InvalidRequest,
                    $"invalid-operator: the operator must be 1 to {MagAdminKill.MaximumOperatorLength} printable ASCII characters", change),
                "invalid-duration" => Refuse(MagVoiceRefusal.InvalidRequest, lift
                    ? "invalid-duration: a lift takes no duration"
                    : $"invalid-duration: the duration must be from 1 minute to {LocalCharacterStore.MaximumVoiceSanctionDuration.TotalDays:0} days", change),
                _ => Refuse(MagVoiceRefusal.Failed, change.Refusal, change)
            };
        }

        static MagVoiceResult Refuse(MagVoiceRefusal refusal, string message, VoiceSanctionChange change) =>
            new MagVoiceResult { Refusal = refusal, Message = message, Change = change };

        /// <summary>The API and log form of one sanction.</summary>
        public static object Json(VoiceSanction s) => s == null ? null : new
        {
            account = s.Account, kind = VoiceSanction.Name(s.Kind), reason = s.Reason, @operator = s.Operator,
            created_at = s.CreatedAt.UtcDateTime.ToString("yyyy-MM-ddTHH:mm:ssZ", CultureInfo.InvariantCulture),
            expires_at = s.ExpiresAt?.UtcDateTime.ToString("yyyy-MM-ddTHH:mm:ssZ", CultureInfo.InvariantCulture)
        };
    }
}
