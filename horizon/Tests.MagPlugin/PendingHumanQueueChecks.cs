using System;
using System.Linq;
using Server.Medius;

// Invented owners/tickets exercise pending queue lifetime without services.
internal static class PendingHumanQueueChecks
{
    static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }
    static void Throws<T>(Action action) where T : Exception
    {
        try { action(); }
        catch (T) { return; }
        throw new Exception("Expected " + typeof(T).Name);
    }
    public static void Run()
    {
        int generated = 0;
        var queue = new MagPendingHumanQueue<object>(() => "ticket-" + ++generated);
        var firstValue = new object();
        var old = queue.Put("one", 0, 1, firstValue);
        var second = queue.Put("two", 0, 1, new object());
        var replacementValue = new object();
        var current = queue.Put("one", 0, 1, replacementValue);
        Check(queue.Count == 2 && generated == 2 && old.Human == current.Human &&
            old.Human.Order < second.Human.Order && !ReferenceEquals(old, current), "Duplicate owner lost queue age or created a ticket");
        Check(ReferenceEquals(current.Value, replacementValue) && !queue.IsCurrent(old) && queue.IsCurrent(current), "Replacement generation ownership");
        Check(!queue.Remove(old), "Stale generation cancelled current owner");
        var forged = new MagPendingHumanQueue<object>.Entry(current.Owner, current.Human, current.Value);
        Check(!queue.IsCurrent(forged) && !queue.Remove(forged), "Structural record equality must not confer ownership");
        var snapshot = queue.Snapshot();
        Check(snapshot.Select(e => e.Owner).SequenceEqual(new[] { "one", "two" }), "Snapshot lost original age");
        var moved = queue.Put("one", 1, 1, new object());
        Check(moved.Human.TicketId != current.Human.TicketId && moved.Human.Order > second.Human.Order &&
            queue.Snapshot().Select(e => e.Owner).SequenceEqual(new[] { "two", "one" }), "Mode change must issue a new ticket at tail");
        Check(snapshot[0] == current && snapshot.Count == 2, "Caller snapshot mutated with queue");
        var factionChanged = queue.Put("one", 1, 2, new object());
        Check(factionChanged.Human.TicketId != moved.Human.TicketId && !queue.Remove(moved), "Faction change did not fence prior ticket");
        Check(queue.Remove("one") && !queue.Remove("one") && !queue.IsCurrent(factionChanged), "Owner cancellation");
        Check(queue.Remove(second) && queue.Count == 0 && !queue.Remove(second), "Expected-entry cancellation");
        Check(!queue.IsCurrent(null) && !queue.Remove((MagPendingHumanQueue<object>.Entry)null), "Null stale entries must be harmless");

        foreach (string bad in new[] { null, "", " ", new string('a', 129) })
            Throws<ArgumentException>(() => queue.Put(bad, 0, 0, new object()));
        Throws<ArgumentException>(() => queue.Put("bad", 2, 0, new object()));
        Throws<ArgumentException>(() => queue.Put("bad", 0, 3, new object()));
        Throws<ArgumentNullException>(() => queue.Put("bad", 0, 0, null));
        Check(queue.Count == 0 && generated == 4, "Invalid input mutated queue or called generator");

        string next = "stable";
        var badGenerator = new MagPendingHumanQueue<object>(() => next);
        var stable = badGenerator.Put("owner", 0, 0, new object());
        foreach (string bad in new[] { null, "", " ", new string('a', 65), "bad/ticket", "stable" })
        {
            next = bad;
            Throws<InvalidOperationException>(() => badGenerator.Put("owner", 1, 0, new object()));
            Check(badGenerator.Count == 1 && badGenerator.IsCurrent(stable), "Bad generator replaced current entry");
        }
        badGenerator.Remove(stable);
        next = "stable";
        var reused = badGenerator.Put("new-owner", 0, 0, new object());
        Check(reused.Human.Order == 1 && !badGenerator.IsCurrent(stable), "Removed ID reuse must retain generation fencing and fresh age");
        next = "fresh";
        Check(badGenerator.Put("fresh-owner", 0, 0, new object()).Human.Order == 2, "Rejected tickets consumed order");

        var full = new MagPendingHumanQueue<object>();
        for (int i = 0; i < 4096; i++) full.Put("owner-" + i, 0, 0, new object());
        Throws<InvalidOperationException>(() => full.Put("overflow", 0, 0, new object()));
        var refreshed = full.Put("owner-0", 0, 0, new object());
        Check(full.Count == 4096 && full.IsCurrent(refreshed), "Capacity refusal mutated queue or blocked replacement");
    }
}
