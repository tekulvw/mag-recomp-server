using System;
using System.Collections.Concurrent;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Server.Medius;

internal static class AuthenticationQueueChecks
{
    static readonly TimeSpan Deadline = TimeSpan.FromSeconds(5);

    public static void Run()
    {
        SlowWorkDoesNotBlockTickOrAnotherChannel();
        SameChannelIsOrderedAndSerialized();
        CapacityIncludesRunningWorkAndRecovers();
        ClosedOwnersAreCheckedAtDispatch();
        ErrorsCannotStrandTheChannel();
        ConcurrentProducersDrainWithoutOverlap();
    }

    static void Check(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException("Authentication queue: " + message);
    }

    static void Wait(ManualResetEventSlim signal, string message) => Check(signal.Wait(Deadline), message);

    // Inline dispatch would hold the enqueue caller and the shared Medius tick here.
    static void SlowWorkDoesNotBlockTickOrAnotherChannel()
    {
        var queue = new MagAuthenticationWorkQueue(concurrency: 2);
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var completed = new ManualResetEventSlim();
        using var unrelated = new ManualResetEventSlim();
        var caller = Task.Run(() => queue.TryEnqueue("slow-store", () => true, () =>
        {
            entered.Set();
            try { Check(release.Wait(Deadline), "slow store release timed out"); }
            finally { completed.Set(); }
        }));
        try
        {
            Check(caller.Wait(Deadline) && caller.Result, "enqueue must return while slow store work is held");
            Wait(entered, "slow store work never entered");
            int ticks = 0;
            ticks++; // The shared tick can continue while the store callback remains held.
            Check(ticks == 1 && !completed.IsSet, "slow store work blocked the caller tick");
            Check(queue.TryEnqueue("other-owner", () => true, unrelated.Set), "unrelated channel was rejected");
            Wait(unrelated, "an unrelated channel could not run beside the held store callback");
            Check(!completed.IsSet, "held store callback was released prematurely");
        }
        finally
        {
            release.Set();
            caller.Wait(Deadline);
            if (entered.IsSet) Wait(completed, "slow callback did not finish");
        }
    }

    // Starting a second callback on this owner before the first returns breaks session state.
    static void SameChannelIsOrderedAndSerialized()
    {
        var queue = new MagAuthenticationWorkQueue(concurrency: 2);
        var order = new ConcurrentQueue<int>();
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var done = new ManualResetEventSlim();
        int overlap = 0, active = 0;
        Check(queue.TryEnqueue("owner", () => true, () =>
        {
            if (Interlocked.Increment(ref active) != 1) Interlocked.Increment(ref overlap);
            order.Enqueue(1);
            entered.Set();
            release.Wait(Deadline);
            Interlocked.Decrement(ref active);
        }), "first owner work rejected");
        try
        {
            Wait(entered, "first owner callback never started");
            Check(queue.TryEnqueue("owner", () => true, () =>
            {
                if (Interlocked.Increment(ref active) != 1) Interlocked.Increment(ref overlap);
                order.Enqueue(2);
                Interlocked.Decrement(ref active);
            }), "second owner work rejected");
            Check(queue.TryEnqueue("owner", () => true, () =>
            {
                if (Interlocked.Increment(ref active) != 1) Interlocked.Increment(ref overlap);
                order.Enqueue(3);
                Interlocked.Decrement(ref active);
                done.Set();
            }), "third owner work rejected");
            Check(order.SequenceEqual(new[] { 1 }), "same-owner callbacks ran while its first callback was held");
        }
        finally { release.Set(); }
        Wait(done, "ordered owner callbacks did not drain");
        Check(overlap == 0 && order.SequenceEqual(new[] { 1, 2, 3 }), "same-owner callbacks overlapped or reordered");
    }

    // Counting only waiting work would admit these over-capacity requests.
    static void CapacityIncludesRunningWorkAndRecovers()
    {
        var queue = new MagAuthenticationWorkQueue(concurrency: 1, capacity: 3, perChannel: 2);
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var drained = new CountdownEvent(3);
        Check(queue.TryEnqueue("a", () => true, () => { entered.Set(); release.Wait(Deadline); drained.Signal(); }), "capacity first enqueue");
        try
        {
            Wait(entered, "capacity callback never started");
            Check(queue.TryEnqueue("a", () => true, () => drained.Signal()), "second same-owner request should fit");
            Check(!queue.TryEnqueue("a", () => true, () => { }), "running work was excluded from the per-owner bound");
            Check(queue.TryEnqueue("b", () => true, () => drained.Signal()), "third global request should fit");
            Check(!queue.TryEnqueue("c", () => true, () => { }), "running work was excluded from the global bound");
        }
        finally { release.Set(); }
        Check(drained.Wait(Deadline), "bounded work did not drain");
        using var recovered = new ManualResetEventSlim();
        // The last callback's signal precedes its queue cleanup; retry admission only until that cleanup completes.
        Check(SpinWait.SpinUntil(() => queue.TryEnqueue("c", () => true, recovered.Set), Deadline), "capacity did not recover");
        Wait(recovered, "recovered request did not execute");
    }

    // Capturing liveness at enqueue time would run this stale owner's callback.
    static void ClosedOwnersAreCheckedAtDispatch()
    {
        var queue = new MagAuthenticationWorkQueue(concurrency: 1, capacity: 3, perChannel: 2);
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var done = new ManualResetEventSlim();
        int live = 1, staleCalls = 0, predicateCalls = 0;
        Check(queue.TryEnqueue("held", () => true, () => { entered.Set(); release.Wait(Deadline); }), "held owner enqueue");
        try
        {
            Wait(entered, "held owner never started");
            Check(queue.TryEnqueue("closing", () => { Interlocked.Increment(ref predicateCalls); return Volatile.Read(ref live) == 1; },
                () => Interlocked.Increment(ref staleCalls)), "closing owner enqueue");
            Check(queue.TryEnqueue("closing", () => true, done.Set), "closing owner successor enqueue");
            Check(predicateCalls == 0, "liveness was checked before acquiring a worker slot");
            Volatile.Write(ref live, 0);
        }
        finally { release.Set(); }
        Wait(done, "skipping a closed owner stranded its successor");
        Check(staleCalls == 0 && predicateCalls == 1, "closed-owner work executed or dispatch liveness was not checked once");
    }

    // Exceptions from either callback, including the error reporter, must release ownership and capacity.
    static void ErrorsCannotStrandTheChannel()
    {
        var queue = new MagAuthenticationWorkQueue(concurrency: 1, capacity: 4, perChannel: 4);
        var reported = new ConcurrentQueue<Exception>();
        using var done = new ManualResetEventSlim();
        var actionError = new InvalidOperationException("synthetic store failure");
        var predicateError = new InvalidOperationException("synthetic liveness failure");
        int skipped = 0;
        Check(queue.TryEnqueue("errors", () => true, () => throw actionError, error =>
        { reported.Enqueue(error); throw new InvalidOperationException("synthetic reporter failure"); }), "throwing action enqueue");
        Check(queue.TryEnqueue("errors", () => throw predicateError, () => Interlocked.Increment(ref skipped), reported.Enqueue), "throwing predicate enqueue");
        Check(queue.TryEnqueue("errors", () => true, () => throw new InvalidOperationException("no reporter")), "unreported error enqueue");
        Check(queue.TryEnqueue("errors", () => true, done.Set), "post-error successor enqueue");
        Wait(done, "an error stranded the worker or same-owner successor");
        Check(skipped == 0 && reported.SequenceEqual(new Exception[] { actionError, predicateError }), "errors were swallowed, duplicated or misreported");
        using var recovered = new ManualResetEventSlim();
        Check(SpinWait.SpinUntil(() => queue.TryEnqueue("errors", () => true, recovered.Set), Deadline), "error path leaked capacity");
        Wait(recovered, "error path stranded future work");
    }

    // Concurrent admission and idle transitions must not lose slots or strand work.
    static void ConcurrentProducersDrainWithoutOverlap()
    {
        const int channels = 6, each = 40;
        var queue = new MagAuthenticationWorkQueue(concurrency: 2, capacity: 16, perChannel: 4);
        using var done = new CountdownEvent(channels * each);
        var ownerActive = new int[channels];
        var next = new int[channels];
        int active = 0, violations = 0;
        var producers = Enumerable.Range(0, channels).Select(owner => Task.Run(() =>
        {
            for (int index = 0; index < each; index++)
            {
                int expected = index;
                Action work = () =>
                {
                    if (Interlocked.Increment(ref active) > 2) Interlocked.Increment(ref violations);
                    if (Interlocked.Increment(ref ownerActive[owner]) != 1) Interlocked.Increment(ref violations);
                    if (next[owner]++ != expected) Interlocked.Increment(ref violations);
                    Thread.Yield();
                    Interlocked.Decrement(ref ownerActive[owner]);
                    Interlocked.Decrement(ref active);
                    done.Signal();
                };
                Check(SpinWait.SpinUntil(() => queue.TryEnqueue("owner-" + owner, () => true, work), Deadline), "concurrent admission never recovered");
            }
        })).ToArray();
        Check(Task.WaitAll(producers, Deadline), "concurrent producers did not finish");
        Check(done.Wait(Deadline), "concurrent accepted requests were stranded");
        Check(violations == 0 && next.All(count => count == each), "worker limit, owner exclusion or FIFO failed under concurrent admission");
    }
}
