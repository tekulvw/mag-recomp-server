using System;
using System.Linq;
using System.Threading.Tasks;

namespace Server.Medius
{
    public sealed partial class LocalDmeHost
    {
        const int ReleaseRetryBatchSize = 8;
        Task _receiptWorker = Task.CompletedTask;
        Task _releaseWorker = Task.CompletedTask;

        async Task ObserveLoadoutSelectionsAsync(GameplayBridgeResponse reply)
        {
            if (!_independentMatchCohorts) await SettleLoadoutSelectionsAsync(reply);
            else if (reply.LoadoutSelections.Count > 0) lock (_gate) _selectionRetry = true;
        }
        async Task ObserveReceiptsAsync(GameplayBridgeResponse reply, bool routed)
        {
            if (routed) await ObserveLoadoutSelectionsAsync(reply);
            if (!_independentMatchCohorts) await SettleRewardsAsync(reply);
            else if (reply?.Ok == true && reply.Rewards.Count > 0) lock (_gate) _rewardRetry = true;
        }
        async Task PollReceiptsAsync(bool reward, bool selection)
        {
            var reply = await _bridge.RequestProgressionAsync(new { v = 2, request_id = Id(), op = "hello" });
            bool routed;
            lock (_gate)
            {
                if (_disposed) return;
                routed = Route(reply);
                if (_independentMatchCohorts && !routed) { _rewardRetry |= reward; _selectionRetry |= selection; }
            }
            if (routed && _independentMatchCohorts)
            {
                await Task.WhenAll(SettleLoadoutSelectionsAsync(reply), SettleRewardsAsync(reply));
                return;
            }
            if (routed) await SettleLoadoutSelectionsAsync(reply);
            await SettleRewardsAsync(reply);
        }
        async Task RetryDueReleasesAsync()
        {
            PendingRelease[] pending;
            lock (_gate)
            {
                var due = _pendingReleases.Values.Where(r => !r.InFlight && r.NextAttempt <= DateTime.UtcNow);
                pending = (_independentMatchCohorts ? due.OrderBy(r => IsResultsCohort(r.Reservation))
                    .ThenBy(r => r.NextAttempt).Take(ReleaseRetryBatchSize) : due).ToArray();
            }
            if (_independentMatchCohorts)
            {
                await Task.WhenAll(pending.Select(RetryReleaseAsync));
                return;
            }
            foreach (var item in pending)
            {
                lock (_gate) if (_disposed) return;
                await RetryReleaseAsync(item);
            }
        }
        async Task ScheduleBridgeMaintenanceAsync()
        {
            Task poll = Task.CompletedTask;
            lock (_gate)
            {
                var now = DateTime.UtcNow;
                bool reward = _rewardRetry && now >= _nextRewardPoll;
                bool selection = _selectionRetry && now >= _nextSelectionPoll;
                if ((reward || selection) && _receiptWorker.IsCompleted)
                {
                    _nextRewardPoll = _nextSelectionPoll = now.AddSeconds(1);
                    if (_independentMatchCohorts)
                    {
                        if (reward) _rewardRetry = false;
                        if (selection) _selectionRetry = false;
                        _receiptWorker = Task.Run(() => PollReceiptsAsync(reward, selection));
                    }
                    else poll = PollReceiptsAsync(reward, selection);
                }
                if (_independentMatchCohorts && _releaseWorker.IsCompleted &&
                    _pendingReleases.Values.Any(p => !p.InFlight && p.NextAttempt <= now))
                    _releaseWorker = Task.Run(RetryDueReleasesAsync);
            }
            await poll;
            if (!_independentMatchCohorts) await RetryDueReleasesAsync();
        }

        async Task FlushRetiredReleasesAsync()
        {
            PendingRelease[] pending;
            var deadline = DateTime.UtcNow.AddSeconds(4);
            lock (_gate) pending = _pendingReleases.Values.Where(p => IsResultsCohort(p.Reservation)).ToArray();
            foreach (var batch in pending.Chunk(ReleaseRetryBatchSize))
            {
                if (DateTime.UtcNow >= deadline) return;
                lock (_gate)
                    foreach (var release in batch)
                        release.NextAttempt = DateTime.MinValue;
                await Task.WhenAll(batch.Select(RetryReleaseAsync));
            }
        }
    }
}
