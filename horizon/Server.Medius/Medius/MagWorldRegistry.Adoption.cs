using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace Server.Medius
{
    /// <summary>Restart adoption for configured workers. On by default; at most
    /// <see cref="MaximumAdoptions"/> adoptions per entry within <see cref="Window"/>.</summary>
    public sealed record MagWorkerAdoptionPolicy(bool Enabled, int MaximumAdoptions, TimeSpan Window)
    {
        public static readonly MagWorkerAdoptionPolicy Default = new(true, 3, TimeSpan.FromMinutes(10));
    }

    /// <summary>Restart-adoption counters for one configured entry since Horizon started.</summary>
    public sealed record MagWorkerAdoptionCounters(string WorkerId, int Adoptions, int WriteOffs, int Refusals, int CapHits,
        int ReleasedPlayers, int DroppedReleases, int LostRewards, int LostLoadoutSelections, int UntrackedReceipts);

    public sealed partial class MagWorldRegistry
    {
        /// <summary>How often a faulted entry's endpoint is asked for its current incarnation.</summary>
        public static readonly TimeSpan AdoptionProbeInterval = TimeSpan.FromSeconds(1);

        /// <summary>Per configured entry; survives the replacement of its slot.</summary>
        sealed class AdoptionEntry
        {
            public readonly string WorkerId;
            public AdoptionEntry(string workerId) => WorkerId = workerId;
            public Task Supervision = Task.CompletedTask;
            public DateTime NextProbe;
            // Written only by this entry's single supervision task, or before supervision starts.
            public MagWorkerContract Contract;
            public string RefusedIncarnation;
            public string LastNotice;
            public readonly Queue<DateTime> Adoptions = new();
            public int AdoptionCount, WriteOffs, Refusals, CapHits, ReleasedPlayers, DroppedReleases, LostRewards, LostSelections, Untracked;
        }
        readonly AdoptionEntry[] _entries;

        public MagWorkerAdoptionPolicy AdoptionPolicy => _config.Adoption;

        public IReadOnlyList<MagWorkerAdoptionCounters> AdoptionCounters
        {
            get
            {
                lock (_gate)
                    return Array.AsReadOnly(_entries.Select(e => new MagWorkerAdoptionCounters(e.WorkerId, e.AdoptionCount, e.WriteOffs,
                        e.Refusals, e.CapHits, e.ReleasedPlayers, e.DroppedReleases, e.LostRewards, e.LostSelections, e.Untracked)).ToArray());
            }
        }

        /// <summary>Start one probe for each entry whose bridge identity faulted or whose host
        /// failed. A healthy entry, or one whose stream is merely reconnecting to the same
        /// incarnation, is never probed: that path keeps its existing behaviour.</summary>
        void SuperviseWorkers()
        {
            if (!_config.Adoption.Enabled) return;
            var now = DateTime.UtcNow;
            lock (_gate)
            {
                if (_disposed || !_initialized) return;
                for (int i = 0; i < _slots.Length; i++)
                {
                    var slot = _slots[i]; var entry = _entries[i];
                    if (!entry.Supervision.IsCompleted || now < entry.NextProbe) continue;
                    if (!slot.Faulted && !slot.Bridge.HasWorkerIdentityFault) continue;
                    entry.NextProbe = now + AdoptionProbeInterval;
                    int index = i;
                    entry.Supervision = Task.Run(async () =>
                    {
                        try { await SuperviseAsync(index, slot, entry); }
                        catch (Exception error) { Logger.Error($"Configured worker supervision deferred: worker={entry.WorkerId} error={error.GetType().Name}"); }
                    });
                }
            }
        }

        void Notice(AdoptionEntry entry, string key, Action log)
        {
            lock (_gate) { if (entry.LastNotice == key) return; entry.LastNotice = key; }
            log();
        }

        async Task SuperviseAsync(int index, Slot old, AdoptionEntry entry)
        {
            var row = old.Config;
            // 1. Detect: ask the endpoint, without a fence, which incarnation now answers there.
            // Discovery has no side effects on the worker. The probe bridge binds whatever
            // incarnation answers with the configured worker ID and worker_registry_v1.
            var probe = new GameplayBridge(row.WorkerHost, row.WorkerPort, row.WorkerId);
            var hello = await probe.RequestHelloAsync(new { v = 2, request_id = Guid.NewGuid().ToString("N"), op = "hello" });
            if (hello?.Ok != true || probe.WorkerIdentity == null)
            {
                if (hello?.Failure == GameplayBridgeFailure.WorkerIdentityMismatch)
                    Notice(entry, "foreign", () => Logger.Error($"Configured worker adoption refused: worker={row.WorkerId} reason=identity; the endpoint answers without this worker ID or worker_registry_v1"));
                else if (hello != null && !hello.IsTransientFailure)
                    Notice(entry, "discovery:" + hello.Failure, () => Logger.Error($"Configured worker adoption refused: worker={row.WorkerId} reason=discovery failure={hello.Failure} error={hello.Error ?? "none"}"));
                return; // Unreachable: the worker is still down or starting. Probe again later.
            }
            string next = probe.WorkerIdentity.IncarnationId;
            string previous = old.Bridge.WorkerIdentity?.IncarnationId;
            if (next == previous)
            {
                // The same process still answers: not a restart. Its fault keeps the entry closed as before.
                Notice(entry, "same:" + next, () => Logger.Warn($"Configured worker fault without a restart: worker={row.WorkerId} incarnation={next}; not adopting, entry stays closed"));
                return;
            }

            // 2. Write off the old incarnation: another incarnation answers, so the old process is gone.
            await WriteOffAsync(old, entry, next);

            // 3. Adopt the new incarnation, unless refused or capped.
            lock (_gate) if (_disposed || entry.RefusedIncarnation == next) return;
            var contract = MagWorkerContract.From(hello);
            if (entry.Contract?.Difference(contract) is string difference)
            {
                Refuse(entry, next, difference);
                return;
            }
            var now = DateTime.UtcNow;
            int recent;
            lock (_gate)
            {
                while (entry.Adoptions.Count > 0 && now - entry.Adoptions.Peek() >= _config.Adoption.Window) entry.Adoptions.Dequeue();
                recent = entry.Adoptions.Count;
                if (recent < _config.Adoption.MaximumAdoptions) entry.Adoptions.Enqueue(now);
            }
            if (recent >= _config.Adoption.MaximumAdoptions)
            {
                DateTime reopens;
                lock (_gate) reopens = entry.Adoptions.Peek() + _config.Adoption.Window;
                Notice(entry, "cap:" + next, () =>
                {
                    lock (_gate) entry.CapHits++;
                    Logger.Error($"Configured worker adoption cap reached: worker={row.WorkerId} incarnation={next} adoptions={recent} limit={_config.Adoption.MaximumAdoptions} window_s={(int)_config.Adoption.Window.TotalSeconds}; entry stays closed until {reopens:O} (crash loop?)");
                });
                return;
            }

            var replacement = CreateSlot(row, probe);
            try { await replacement.Host.StartInRegistryAsync(_manager); }
            catch (Exception error) when (error is IOException || error is ArgumentException || error is InvalidOperationException || error is TimeoutException)
            {
                // StartInRegistryAsync disposed the replacement. The next probe tries again within the cap.
                Logger.Warn($"Configured worker adoption failed: worker={row.WorkerId} incarnation={next} error={error.GetType().Name}");
                return;
            }
            // The startup discovery may come from yet another restart; it must still match.
            string changed = (entry.Contract ?? contract).Difference(replacement.Host.WorkerContract) ??
                (replacement.Bridge.WorkerIdentity?.IncarnationId != next ? "incarnation_changed_during_start" : null);
            if (changed != null)
            {
                await replacement.Host.DisposeAsync();
                Refuse(entry, next, changed);
                return;
            }
            bool installed;
            lock (_gate)
            {
                installed = !_disposed;
                if (installed)
                {
                    _slots[index] = replacement;
                    entry.Contract ??= replacement.Host.WorkerContract;
                    entry.AdoptionCount++;
                    entry.LastNotice = null;
                    entry.RefusedIncarnation = null;
                }
            }
            if (!installed) { await replacement.Host.DisposeAsync(); return; }
            Logger.Warn($"Configured worker adopted: worker={row.WorkerId} incarnation={next} previous={previous ?? "none"} world={replacement.Host.World?.WorldId} adoptions_in_window={recent + 1}/{_config.Adoption.MaximumAdoptions}; admission reopened for this entry only");
            StartPlacement();
        }

        void Refuse(AdoptionEntry entry, string incarnation, string difference)
        {
            lock (_gate) { entry.RefusedIncarnation = incarnation; entry.Refusals++; entry.LastNotice = "refused:" + incarnation; }
            Logger.Error($"Configured worker adoption refused: worker={entry.WorkerId} incarnation={incarnation} reason=contract_mismatch {difference}; entry stays closed until a matching incarnation answers or Medius restarts");
        }

        async Task WriteOffAsync(Slot old, AdoptionEntry entry, string replacement)
        {
            lock (_gate)
            {
                if (old.WrittenOff) return;
                old.WrittenOff = true;
                old.Faulted = true;
            }
            // An entry that never bound an incarnation (its worker was unreachable at startup)
            // has nothing to write off; its host was already disposed.
            var report = old.Bridge.WorkerIdentity == null ? null : await old.Host.WriteOffIncarnationAsync(replacement);
            await old.Host.DisposeAsync();
            if (report == null) return;
            int writeOffs, lostRewards, lostSelections;
            lock (_gate)
            {
                writeOffs = ++entry.WriteOffs;
                entry.ReleasedPlayers += report.ReleasedPlayers;
                entry.DroppedReleases += report.DroppedReleases;
                lostRewards = entry.LostRewards += report.LostRewards;
                lostSelections = entry.LostSelections += report.LostLoadoutSelections;
                entry.Untracked += report.UntrackedReceipts;
            }
            Logger.Warn($"Configured worker write-off totals: worker={entry.WorkerId} write_offs={writeOffs} lost_rewards_total={lostRewards} lost_loadout_selections_total={lostSelections}");
            // Tickets forwarded to the old host return to the global queue on the next pass.
            StartPlacement();
        }
    }
}
