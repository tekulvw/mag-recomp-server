using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json.Serialization;

namespace Server.Medius
{
    public sealed class GameplayLoadoutRefreshTarget
    {
        public string ReservationId { get; }
        public uint WorldId { get; }
        public ushort PlayerId { get; }
        public string AccountId { get; }
        public uint CharacterId { get; }
        public GameplayLoadoutRefreshTarget(string reservation, uint world, ushort player, string account, uint character)
        {
            ReservationId = reservation; WorldId = world; PlayerId = player; AccountId = account; CharacterId = character;
        }
        public bool MatchesOwner(string reservation, uint world, ushort player, string account, uint character) =>
            string.Equals(ReservationId, reservation, StringComparison.Ordinal) && WorldId == world && PlayerId == player &&
            string.Equals(AccountId, account, StringComparison.Ordinal) && CharacterId == character;
    }
    public sealed class GameplayLoadoutRefreshRequest
    {
        [JsonPropertyName("v")] public int Version => 2;
        [JsonPropertyName("op")] public string Operation => "refresh_loadouts";
        [JsonPropertyName("request_id")] public string RequestId { get; }
        [JsonIgnore] public GameplayLoadoutRefreshTarget Target { get; }
        [JsonPropertyName("reservation_id")] public string ReservationId => Target.ReservationId;
        [JsonPropertyName("world_id")] public uint WorldId => Target.WorldId;
        [JsonPropertyName("player_id")] public ushort PlayerId => Target.PlayerId;
        [JsonPropertyName("account_id")] public string AccountId => Target.AccountId;
        [JsonPropertyName("character_id")] public uint CharacterId => Target.CharacterId;
        [JsonPropertyName("revision")] public ulong Revision { get; }
        [JsonPropertyName("loadouts")] public GameplayAdmissionLoadouts Loadouts { get; }
        internal GameplayLoadoutRefreshRequest(GameplayLoadoutRefreshTarget target, ulong revision, GameplayAdmissionLoadouts loadouts)
        {
            RequestId = Guid.NewGuid().ToString("N"); Target = target; Revision = revision; Loadouts = loadouts.Snapshot();
        }
    }
    /// <summary>Latest-only refreshes of canonical stored inventories. The supplied
    /// owner gate serializes scheduling/projection, dispatch checks and release.
    /// A request owns an immutable snapshot for same-revision network retries.</summary>
    public sealed class GameplayLoadoutRefreshQueue
    {
        sealed class Entry
        {
            public GameplayLoadoutRefreshTarget Target;
            public ulong Revision;
            public GameplayLoadoutRefreshRequest Request;
            public int Reads, Sends;
            public bool Pending = true, InFlight;
            public DateTime NextAttempt;
        }
        readonly object _ownerGate;
        readonly Action<GameplayLoadoutRefreshTarget, ulong, string> _dropped;
        readonly Dictionary<string, Entry> _entries = new Dictionary<string, Entry>(StringComparer.Ordinal);
        public const int MaximumAttempts = 3;
        public const int MaximumOwners = 64;
        public GameplayLoadoutRefreshQueue(object ownerGate, Action<GameplayLoadoutRefreshTarget, ulong, string> dropped = null)
        { _ownerGate = ownerGate ?? throw new ArgumentNullException(nameof(ownerGate)); _dropped = dropped; }
        void Drop(Entry entry, string reason)
        {
            entry.Pending = false; entry.InFlight = false;
            _dropped?.Invoke(entry.Target, entry.Revision, reason);
        }
        bool Project(Entry entry, DateTime now, Func<GameplayLoadoutRefreshTarget, GameplayAdmissionLoadouts> project)
        {
            entry.Reads++;
            try
            {
                var loadouts = project(entry.Target) ?? throw new InvalidDataException("Missing canonical loadouts.");
                entry.Request = new GameplayLoadoutRefreshRequest(entry.Target, entry.Revision, loadouts);
                return true;
            }
            catch (InvalidDataException) { Drop(entry, "invalid_store_projection"); }
            catch (UnauthorizedAccessException) { Drop(entry, "store_access_rejected"); }
            catch (IOException)
            {
                if (entry.Reads >= MaximumAttempts) Drop(entry, "store_read_exhausted");
                else entry.NextAttempt = now.AddSeconds(1);
            }
            catch (Exception error) { Drop(entry, "unexpected_projection_" + error.GetType().Name); }
            return false;
        }
        public bool Schedule(GameplayLoadoutRefreshTarget target, DateTime now, Func<GameplayLoadoutRefreshTarget, bool> current,
            Func<GameplayLoadoutRefreshTarget, GameplayAdmissionLoadouts> project)
        {
            lock (_ownerGate)
            {
                if (!current(target)) return false;
                _entries.TryGetValue(target.ReservationId, out var previous);
                if (previous == null && _entries.Count >= MaximumOwners)
                { _dropped?.Invoke(target, 0, "refresh_capacity"); return false; }
                if (previous?.Revision == ulong.MaxValue) { Drop(previous, "refresh_revision_exhausted"); return false; }
                var entry = new Entry { Target = target, Revision = (previous?.Revision ?? 0) + 1, NextAttempt = now };
                _entries[target.ReservationId] = entry;
                Project(entry, now, project);
                return entry.Pending;
            }
        }
        public bool HasDue(DateTime now)
        { lock (_ownerGate) return _entries.Values.Any(entry => entry.Pending && !entry.InFlight && entry.NextAttempt <= now); }
        public GameplayLoadoutRefreshRequest TakeDue(DateTime now, Func<GameplayLoadoutRefreshTarget, bool> current,
            Func<GameplayLoadoutRefreshTarget, GameplayAdmissionLoadouts> project)
        {
            lock (_ownerGate)
            {
                foreach (var entry in _entries.Values)
                {
                    if (!entry.Pending || entry.InFlight || entry.NextAttempt > now) continue;
                    if (!current(entry.Target)) { Drop(entry, "owner_unavailable"); continue; }
                    if (entry.Request == null && !Project(entry, now, project)) continue;
                    entry.InFlight = true; entry.Sends++;
                    return entry.Request;
                }
                return null;
            }
        }
        public bool StillCurrent(GameplayLoadoutRefreshRequest request, Func<GameplayLoadoutRefreshTarget, bool> current)
        {
            lock (_ownerGate) return _entries.TryGetValue(request.ReservationId, out var entry) && entry.Pending && entry.InFlight &&
                ReferenceEquals(entry.Request, request) && current(request.Target);
        }
        public void Complete(GameplayLoadoutRefreshRequest request, DateTime now, bool accepted, bool transient,
            Func<GameplayLoadoutRefreshTarget, bool> current)
        {
            lock (_ownerGate)
            {
                if (!_entries.TryGetValue(request.ReservationId, out var entry) || !ReferenceEquals(entry.Request, request)) return;
                entry.InFlight = false;
                if (!current(entry.Target)) { Drop(entry, "owner_unavailable"); return; }
                if (accepted) { entry.Pending = false; return; }
                if (!transient) { Drop(entry, "refresh_rejected"); return; }
                if (entry.Sends >= MaximumAttempts) Drop(entry, "refresh_send_exhausted");
                else entry.NextAttempt = now.AddSeconds(1);
            }
        }
        public void Forget(string reservation)
        { lock (_ownerGate) _entries.Remove(reservation); }
    }
}
