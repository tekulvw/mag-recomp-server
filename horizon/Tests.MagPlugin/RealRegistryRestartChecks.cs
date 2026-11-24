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

                var victimPair = await Admit("alpha", "bravo");
                var survivorPair = await Admit("charlie", "delta");
                Check(victimPair.Concat(survivorPair).All(reply => reply.Result == 0), "Real worker admission failed before the restart");
                string victimId = registry.Worlds.Single(w => w.WorldId == victimPair[0].WorldId).WorkerId;
                var victim = victimId == first.Id ? first : second;
                var survivor = victimId == first.Id ? second : first;
                var before = World(registry, victimId);
                var survivorBefore = World(registry, survivor.Id);
                Check(survivorPair.All(reply => reply.WorldId == survivorBefore.WorldId) && survivorBefore.ReservedPlayers == 2 &&
                    Reservations(victim).Count == 2 && Reservations(survivor).Count == 2, "Each real worker did not hold its own match");

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
                Check(World(registry, survivor.Id) == survivorBefore && Releases(survivor).Count == 0 &&
                    Counters(registry, survivor.Id).WriteOffs == 0, "Adopting one real worker disturbed the other worker's match");

                // The released players form a new match on the restarted process.
                var again = await Admit("alpha", "bravo");
                Check(again.All(reply => reply.Result == 0 && reply.WorldId == adopted.WorldId) && Reservations(replacement).Count == 2 &&
                    Reservations(replacement).All(row => row["world_id"]!.GetValue<int>() == adopted.WorldId),
                    "A new match did not form on the restarted real worker");
                Check(World(registry, survivor.Id) == survivorBefore, "The other real worker's match changed during the new formation");
            }
            finally { if (replacement != null) await replacement.DisposeAsync(); }
        }
    });
}
