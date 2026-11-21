using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;

namespace Server.Medius
{
    public sealed partial class LocalDmeHost
    {
        sealed class ProgressionRefresh
        {
            public Reservation Owner { get; }
            public int Attempts;
            public DateTime NextAttempt;
            public ProgressionRefresh(Reservation owner) => Owner = owner;
        }

        readonly Dictionary<string, ProgressionRefresh> _progressionRefreshes = new(StringComparer.Ordinal);
        Task _progressionRefreshWorker = Task.CompletedTask;

        public event Action<string, uint> DurableProgressionCommitted;

        /// <summary>Refresh the exact current actor from the shared durable record.</summary>
        public void QueueProgressionRefresh(string account, uint characterId)
        {
            if (!InRegistry) return;
            lock (_gate)
            {
                if (_disposed) return;
                var owner = _reservations.Values.FirstOrDefault(row =>
                    IsLiveCohort(row) && !row.Invalidated && row.Account == account && row.CharacterId == characterId);
                // Admission publishes a pending owner under the shared progression
                // gate. An event before that point is already in its reserve snapshot.
                if (owner != null) _progressionRefreshes[owner.Id] = new ProgressionRefresh(owner);
            }
        }

        bool CanRefreshProgression(ProgressionRefresh refresh) =>
            refresh.Owner.Accepted && !refresh.Owner.Invalidated && IsLiveCohort(refresh.Owner) &&
            refresh.Owner.Deadline > DateTime.UtcNow && !refresh.Owner.Client.IsDestroyed &&
            refresh.NextAttempt <= DateTime.UtcNow;

        void StartProgressionRefreshPump()
        {
            if (!InRegistry) return;
            lock (_gate)
            {
                if (ReadyForGameplay && _progressionRefreshWorker.IsCompleted &&
                    _progressionRefreshes.Values.Any(CanRefreshProgression))
                    _progressionRefreshWorker = Task.Run(PumpProgressionRefreshesAsync);
            }
        }

        async Task PumpProgressionRefreshesAsync()
        {
            for (int batch = 0; batch < 8; batch++)
            {
                ProgressionRefresh refresh;
                lock (_gate)
                {
                    if (!ReadyForGameplay) return;
                    refresh = _progressionRefreshes.Values.FirstOrDefault(CanRefreshProgression);
                    if (refresh == null) return;
                }
                var owner = refresh.Owner;
                bool retry = false;
                try
                {
                    var row = _store.ReadForAccount(owner.Account).SingleOrDefault(character => character.Id == owner.CharacterId);
                    if (row == null) throw new InvalidDataException("Durable progression owner is missing.");
                    var record = row.ToRecord();
                    var request = new
                    {
                        v = 2, request_id = Id(), op = "refresh_progression", reservation_id = owner.Id,
                        world_id = (uint)World.WorldId, player_id = (ushort)owner.Client.DmeId,
                        account_id = owner.Account, character_id = owner.CharacterId,
                        progression = new { experience = record.Experience, level = record.Level, skill_points = record.SkillPoints },
                        leadership = row.Leadership == null ? null : new GameplayLeadershipRecord(row.Leadership)
                    };
                    lock (_gate)
                    {
                        if (!ReadyForGameplay) return;
                        if (_progressionRefreshes.GetValueOrDefault(owner.Id) != refresh || !CanRefreshProgression(refresh)) continue;
                    }
                    var reply = await _bridge.RequestProgressionRefreshAsync(request);
                    bool routed;
                    lock (_gate) routed = ReadyForGameplay && Route(reply);
                    if (routed) await ObserveReceiptsAsync(reply, true);
                    else retry = reply?.IsTransientFailure == true || reply?.Error == "refresh_progression_publications_full";
                }
                catch (IOException) { retry = true; }
                catch (Exception error) when (error is InvalidDataException || error is UnauthorizedAccessException)
                { Logger.Warn($"Durable progression refresh could not read its owner: player={owner.Client.DmeId} error={error.GetType().Name}"); }
                lock (_gate)
                {
                    if (_progressionRefreshes.GetValueOrDefault(owner.Id) != refresh) continue;
                    if (retry && ++refresh.Attempts < 3) refresh.NextAttempt = DateTime.UtcNow.AddSeconds(1);
                    else
                    {
                        _progressionRefreshes.Remove(owner.Id);
                        if (retry) Logger.Warn($"Durable progression refresh exhausted its retries: player={owner.Client.DmeId}");
                    }
                }
            }
        }
    }
}
