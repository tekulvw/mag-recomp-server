using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

/// <summary>Subscription options a configured world sends, and a worker's shutdown drain,
/// with invented loopback workers.</summary>
internal static class WorkerDrainChecks
{
    public static void Run()
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try
        {
            LeadershipProgressionChecks.RegistrySubscriptionOptions();
            LeadershipProgressionChecks.RegistryWorkerDrainNotice();
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    }

    public static void Codec()
    {
        static bool Parse(JsonObject frame, out GameplayBridgeResponse reply) =>
            GameplayBridgeResponseCodec.TryParse(System.Text.Encoding.UTF8.GetBytes(frame.ToJsonString()), "synthetic", true, out reply);
        JsonObject Frame() => new() { ["v"] = 2, ["request_id"] = "synthetic", ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray() };

        if (!Parse(Frame(), out var plain) || plain.WorkerDraining)
            throw new Exception("A frame without worker_draining was read as draining");
        var draining = Frame(); draining["worker_draining"] = true;
        if (!Parse(draining, out var reply) || !reply.WorkerDraining)
            throw new Exception("A worker_draining frame was not read");
        var falseFlag = Frame(); falseFlag["worker_draining"] = false;
        var refused = Frame(); refused["ok"] = false; refused["error"] = "synthetic_error"; refused["worker_draining"] = true;
        if (Parse(falseFlag, out _) || Parse(refused, out _))
            throw new Exception("worker_draining was accepted as false or on a refusal");

        var refusal = new GameplayWorldSubscriptionRefusedException("subscription_world_mismatch");
        if (refusal.Message.Length == 0 || refusal.Reason != "subscription_world_mismatch" ||
            new GameplayWorldSubscriptionRefusedException("not a code\nline").Reason != "unspecified")
            throw new Exception("A subscription refusal lost its reason or kept unvalidated worker text");
        if (!GameplayWorldSubscription.IsValidControlPlane(Guid.NewGuid().ToString("N")) ||
            GameplayWorldSubscription.IsValidControlPlane("short") || GameplayWorldSubscription.IsValidControlPlane(new string('a', 65)))
            throw new Exception("Control plane names were not bounded like the worker's");
    }
}

internal static partial class LeadershipProgressionChecks
{
    static IReadOnlyList<JsonObject> Subscriptions(RegistryWorker worker) =>
        worker.Requests.Where(r => r["op"]!.GetValue<string>() == "subscribe_world").ToArray();

    /// <summary>World IDs follow configuration order; a world names its Horizon process only to a
    /// worker that can rebind, keeps that name across its own reconnects, and asks for drain
    /// notices only from a worker that sends them.</summary>
    public static void RegistrySubscriptionOptions() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "options");
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            await using var named = new RegistryWorker("named") { Fence = true, SupportsControlPlane = true };
            await using var draining = new RegistryWorker("draining") { Fence = true, SupportsDrain = true };
            string first;
            await using (var registry = AdoptionRegistry(store, null, AdoptionWorkerRow("named", named.Port), AdoptionWorkerRow("draining", draining.Port)))
            {
                await registry.InitializeAsync();
                Check(World(registry, "draining").WorldId == World(registry, "named").WorldId + 1,
                    "Configured worlds did not take their IDs in configuration order");
                var subscription = Subscriptions(named).Single();
                first = subscription["control_plane"]?.GetValue<string>();
                Check(GameplayWorldSubscription.IsValidControlPlane(first) && subscription["drain_notices"] == null,
                    "A worker with control_plane_rebind_v1 was not told which Horizon process subscribed");
                var plain = Subscriptions(draining).Single();
                Check(plain["control_plane"] == null && plain["drain_notices"]?.GetValue<bool>() == true,
                    "Subscription options were sent to a worker that did not advertise them");

                // The same process reconnecting keeps its name, so the worker keeps its players.
                named.BreakStreams();
                await TickUntil(registry, () => Subscriptions(named).Count >= 2 && World(registry, "named").Ready,
                    "A broken world stream did not resubscribe");
                Check(Subscriptions(named).All(r => r["control_plane"]?.GetValue<string>() == first),
                    "A reconnect from the same Horizon process changed its control plane name");
            }
            await using (var restarted = AdoptionRegistry(store, null, AdoptionWorkerRow("named", named.Port), AdoptionWorkerRow("draining", draining.Port)))
            {
                await restarted.InitializeAsync();
                string second = Subscriptions(named).Last()["control_plane"]?.GetValue<string>();
                Check(GameplayWorldSubscription.IsValidControlPlane(second) && second != first,
                    "A new Horizon process reused the previous process's control plane name");
            }
        }
    });

    /// <summary>A worker that announces its shutdown drain is closed to placement at once,
    /// while another worker keeps admitting.</summary>
    public static void RegistryWorkerDrainNotice() => WithDirectory(directory =>
    {
        var store = AdoptionStore(directory, "drain", ("alpha", 0), ("bravo", 1));
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            await using var leaving = new RegistryWorker("leaving") { Fence = true, SupportsDrain = true };
            await using var staying = new RegistryWorker("staying") { Fence = true, SupportsDrain = true };
            await using var registry = AdoptionRegistry(store, null, AdoptionWorkerRow("leaving", leaving.Port), AdoptionWorkerRow("staying", staying.Port));
            await registry.InitializeAsync();
            Check(registry.Worlds.All(w => w.Ready && !w.Draining), "Synthetic workers did not open");

            var notice = CohortReply();
            notice["worker_draining"] = true;
            await leaving.PublishAsync(notice);
            await TickUntil(registry, () => World(registry, "leaving").Draining, "A draining worker's notice did not close its world");
            Check(!World(registry, "staying").Draining, "One worker's drain closed another world");

            var queued = new[] { QueueRegistryJoin(registry, store, "alpha", 1), QueueRegistryJoin(registry, store, "bravo", 2) };
            await TickUntil(registry, () => queued.All(t => t.IsCompleted), "Players did not reach the remaining world");
            var replies = await Task.WhenAll(queued);
            Check(replies.All(r => r.Result == 0 && r.WorldId == World(registry, "staying").WorldId) && leaving.Reservations.IsEmpty,
                "A draining worker was offered new players");
        }
    });
}
