using System;
using System.Collections.Generic;
using System.Linq;

namespace Server.Medius
{
    /// <summary>Bounded pending human ownership. This class has no internal
    /// locking: the caller must hold one gate around all reads and mutations,
    /// including selection, callback ownership checks and queue commits.</summary>
    public sealed class MagPendingHumanQueue<T> where T : class
    {
        public sealed record Entry(string Owner, MagQueuedHuman Human, T Value);
        readonly Dictionary<string, Entry> _entries = new(StringComparer.Ordinal);
        readonly Func<string> _nextTicketId;
        long _nextOrder;
        public int Count => _entries.Count;

        public MagPendingHumanQueue(Func<string> nextTicketId = null)
        {
            _nextTicketId = nextTicketId ?? (() => Guid.NewGuid().ToString("N"));
        }

        public Entry Put(string owner, int gameType, byte faction, T value)
        {
            ValidateOwner(owner);
            if (gameType < 0 || gameType > 1 || faction > 2)
                throw new ArgumentException("Pending humans require mode 0/1 and saved faction 0..2.");
            if (value == null) throw new ArgumentNullException(nameof(value));
            _entries.TryGetValue(owner, out var previous);
            if (previous != null && previous.Human.GameType == gameType && previous.Human.Faction == faction)
            {
                var refreshed = new Entry(owner, previous.Human, value);
                _entries[owner] = refreshed;
                return refreshed;
            }
            if (previous == null && _entries.Count >= 4096)
                throw new InvalidOperationException("Pending human queue capacity reached.");
            long followingOrder = checked(_nextOrder + 1);
            string ticketId = _nextTicketId();
            if (string.IsNullOrEmpty(ticketId) || ticketId.Length > 64 ||
                !ticketId.All(c => (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
                    (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.' || c == ':') ||
                _entries.Values.Any(entry => entry.Human.TicketId == ticketId))
                throw new InvalidOperationException("Ticket generator must return a unique pending identifier of at most 64 characters.");
            var replacement = new Entry(owner, new MagQueuedHuman(ticketId, gameType, faction, _nextOrder), value);
            _entries[owner] = replacement;
            _nextOrder = followingOrder;
            return replacement;
        }

        public bool Remove(string owner)
        {
            ValidateOwner(owner);
            return _entries.Remove(owner);
        }

        public bool Remove(Entry expected)
        {
            if (!IsCurrent(expected)) return false;
            return _entries.Remove(expected.Owner);
        }

        public bool IsCurrent(Entry expected) => expected != null && expected.Owner != null &&
            _entries.TryGetValue(expected.Owner, out var current) && ReferenceEquals(current, expected);

        public IReadOnlyList<Entry> Snapshot() => Array.AsReadOnly(_entries.Values
            .OrderBy(entry => entry.Human.Order).ThenBy(entry => entry.Human.TicketId, StringComparer.Ordinal).ToArray());

        static void ValidateOwner(string owner)
        {
            if (string.IsNullOrWhiteSpace(owner) || owner.Length > 128)
                throw new ArgumentException("Pending owner must contain 1..128 nonblank characters.", nameof(owner));
        }
    }
}
