using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    /// <summary>Kill one of two owned synthetic workers mid-match and start it again on the same
    /// endpoint: Horizon writes off the old incarnation, adopts the new one for that entry only
    /// and forms a new match there, while the other worker's match continues.</summary>
    public static void RealRegistryRestart(string binary, string catalogs) => WithDirectory(directory =>
    {
        var settings = Server.Medius.Program.Settings;
        int previous = settings.MagApplicationId;
        settings.MagApplicationId = 20231;
        try { VerifyAsync().GetAwaiter().GetResult(); }
        finally { settings.MagApplicationId = previous; }

        async Task VerifyAsync()
        {
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1), ("charlie", (byte)0), ("delta", (byte)1) })
                Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] },
                    account).Result == MagNetResult.Success, "Synthetic character creation failed");

            await using var first = await RealRegistryWorkerChecks.Worker.StartAsync("first", directory, binary, catalogs);
            await using var second = await RealRegistryWorkerChecks.Worker.StartAsync("second", directory, binary, catalogs);
            RealRegistryWorkerChecks.Worker replacement = null;
            try
            {
                await using var registry = AdoptionRegistry(store, null, AdoptionWorkerRow(first.Id, first.Port), AdoptionWorkerRow(second.Id, second.Port));
                await registry.InitializeAsync();
                int transaction = 0;
                async Task<MagJoinGameResponse[]> Admit(params string[] accounts)
                {
                    var queued = accounts.Select(account => QueueRegistryJoin(registry, store, account, ++transaction, mode: 0)).ToArray();
                    await TickUntil(registry, () => queued.All(t => t.IsCompleted), "Real worker admission did not finish");
                    return await Task.WhenAll(queued);
                }

                // Wait for a settled state; on timeout, name what each worker held.
                async Task Settle(Func<bool> condition, string failure, params RealRegistryWorkerChecks.Worker[] workers)
                {
                    try { await TickUntil(registry, condition, failure); }
                    catch (Exception error) when (error.Message == failure)
                    {
                        throw new Exception($"{failure} ({string.Join("; ", workers.Select(Holdings))})");
                    }
                }

                string Holdings(RealRegistryWorkerChecks.Worker worker)
                {
                    var world = World(registry, worker.Id);
                    return $"{worker.Id}: world={world.WorldId} incarnation={world.IncarnationId} ready={world.Ready} draining={world.Draining} " +
                        $"reserved_players={world.ReservedPlayers} journal_reserved={Reservations(worker).Count} journal_released={Releases(worker).Count}";
                }

                var victimPair = await Admit("alpha", "bravo");
                var survivorPair = await Admit("charlie", "delta");
                Check(victimPair.Concat(survivorPair).All(reply => reply.Result == 0), "Real worker admission failed before the restart");
                Check(victimPair.Select(reply => reply.WorldId).Distinct().Count() == 1 && survivorPair.Select(reply => reply.WorldId).Distinct().Count() == 1 &&
                    victimPair[0].WorldId != survivorPair[0].WorldId,
                    $"Each admitted pair did not share a world of its own (worlds {string.Join(",", victimPair.Concat(survivorPair).Select(reply => reply.WorldId))})");
                string victimId = registry.Worlds.Single(w => w.WorldId == victimPair[0].WorldId).WorkerId;
                var victim = victimId == first.Id ? first : second;
                var survivor = victimId == first.Id ? second : first;
                // Poll instead of reading once: on a loaded host the registry's placement snapshot
                // and the worker journals can trail the admission replies, and an attempt the queue
                // refused and retried leaves a reserved row together with its release.
                await Settle(() => World(registry, victimId).ReservedPlayers == 2 &&
                    World(registry, survivor.Id) is { ReservedPlayers: 2 } held && held.WorldId == survivorPair[0].WorldId &&
                    LiveReservations(victim).Count == 2 && LiveReservations(survivor).Count == 2,
                    "Each real worker did not hold its own match", victim, survivor);
                var before = World(registry, victimId);
                var survivorBefore = World(registry, survivor.Id);
                int survivorReleases = Releases(survivor).Count;

                // Crash the victim mid-match and start a new process on the same endpoint.
                await victim.KillAsync();
                await TickUntil(registry, () => !World(registry, victimId).Ready, "A killed worker still looked ready");
                replacement = await RealRegistryWorkerChecks.Worker.StartAsync(victimId, directory, binary, catalogs,
                    port: victim.Port, instance: victimId + "-restarted");
                await TickUntil(registry, () => World(registry, victimId) is { Ready: true } world && world.IncarnationId != before.IncarnationId,
                    "The restarted real worker was not adopted");
                var adopted = World(registry, victimId);
                var counters = Counters(registry, victimId);
                Check(adopted.WorldId != before.WorldId && adopted.ReservedPlayers == 0 && counters.Adoptions == 1 && counters.WriteOffs == 1 &&
                    counters.ReleasedPlayers + counters.DroppedReleases >= 2 && counters.LostRewards == 0,
                    "The real worker's old incarnation was not written off before its new world opened");
                await Settle(() => World(registry, survivor.Id) == survivorBefore, "Adopting one real worker disturbed the other worker's match",
                    survivor);
                Check(Releases(survivor).Count == survivorReleases && Counters(registry, survivor.Id).WriteOffs == 0,
                    "Adopting one real worker released or wrote off the other worker's match");

                // The released players form a new match on the restarted process.
                var again = await Admit("alpha", "bravo");
                Check(again.All(reply => reply.Result == 0 && reply.WorldId == adopted.WorldId), "A new match did not form on the restarted real worker");
                await Settle(() => LiveReservations(replacement) is { Count: 2 } rows && rows.All(row => row["world_id"]!.GetValue<int>() == adopted.WorldId),
                    "The restarted real worker did not record the new match", replacement);
                await Settle(() => World(registry, survivor.Id) == survivorBefore, "The other real worker's match changed during the new formation",
                    survivor);
            }
            finally { if (replacement != null) await replacement.DisposeAsync(); }
        }
    });

    /// <summary>Journal reservations without a matching release row.</summary>
    private static IReadOnlyList<JsonObject> LiveReservations(RealRegistryWorkerChecks.Worker worker)
    {
        var released = Ids(Releases(worker));
        return Reservations(worker).Where(row => !released.Contains(row["reservation_id"]!.GetValue<string>())).ToArray();
    }
}
