using System.Net;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using RT.Models;
using RT.Models.MAG;
using Server.Medius;

// Operator kill (docs/admin-cli.md): Medius finds the account's placed player, asks only the worker
// that owns it and only when that worker advertises admin_kill_v1, and reports each refusal with
// its own code. All accounts, worlds and replies are synthetic.
internal static class AdminKillChecks
{
    static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    public static void Codec()
    {
        static bool Parse(JsonObject reply, bool eventReply, out GameplayBridgeResponse parsed) =>
            GameplayBridgeResponseCodec.TryParse(Encoding.UTF8.GetBytes(reply.ToJsonString()), "synthetic", eventReply, out parsed);
        JsonObject Reply(JsonNode outcome)
        {
            var reply = new JsonObject { ["v"] = 2, ["request_id"] = "synthetic", ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray() };
            if (outcome != null) reply["admin_kill"] = outcome;
            return reply;
        }
        Check(Parse(Reply(null), true, out var plain) && plain.AdminKill == null, "A reply without admin_kill was read as one");
        Check(Parse(Reply(new JsonObject { ["result"] = "killed" }), true, out var killed) &&
            killed.AdminKill == new GameplayAdminKillOutcome("killed", null), "A killed outcome was not read");
        Check(Parse(Reply(new JsonObject { ["result"] = "gave_up" }), true, out var gaveUp) && gaveUp.AdminKill.Result == "gave_up",
            "A gave_up outcome was not read");
        Check(Parse(Reply(new JsonObject { ["result"] = "refused", ["reason"] = "mounted_death_not_supported" }), true, out var refused) &&
            refused.AdminKill == new GameplayAdminKillOutcome("refused", "mounted_death_not_supported"), "A refused outcome lost its reason");
        foreach (var bad in new JsonNode[]
        {
            new JsonObject { ["result"] = "refused" },
            new JsonObject { ["result"] = "killed", ["reason"] = "extra" },
            new JsonObject { ["result"] = "exploded" },
            new JsonObject { ["result"] = "refused", ["reason"] = "not a code" },
            new JsonObject { ["result"] = "killed", ["unexpected"] = 1 },
            JsonValue.Create("killed")
        })
            Check(!Parse(Reply(bad), true, out _), $"A malformed admin_kill outcome was accepted: {bad.ToJsonString()}");
        Check(!Parse(Reply(new JsonObject { ["result"] = "killed" }), false, out _), "admin_kill was accepted on a control reply");
        var failed = Reply(new JsonObject { ["result"] = "killed" }); failed["ok"] = false; failed["error"] = "not_in_match";
        Check(!Parse(failed, true, out _), "admin_kill was accepted on a refusal");
        Check(Parse(new JsonObject { ["v"] = 2, ["request_id"] = "synthetic", ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray(),
            ["capabilities"] = new JsonArray("admin_kill_v1") }, false, out var hello) && hello.SupportsAdminKill,
            "admin_kill_v1 was not read from a hello");
    }

    public static void Run()
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        Server.Medius.Program.Settings.MagApplicationId = 20231;
        try { LeadershipProgressionChecks.RegistryAdminKill(); }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    }

    /// <summary>The CLI acts through the admin API and needs it configured.</summary>
    public static void Cli()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-admin-kill-cli-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "alpha", Faction = 0, UniverseId = 26, NpId = new byte[36] }, "alpha").Result
                == MagNetResult.Success, "Synthetic character creation failed");
            string markers = Path.Combine(directory, "markers");
            Directory.CreateDirectory(markers);
            var now = DateTime.UtcNow;
            MagActiveAccounts.Write(Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json"), new Dictionary<string, DateTime>(), now);
            int Cli(MagAdminApiClient api, out string stdout, out string stderr, params string[] args)
            {
                var output = new StringWriter(); var error = new StringWriter();
                int code = MagAdminCli.Run(args, store, Path.Combine(directory, "backups"), markers, now, output, error, api);
                stdout = output.ToString(); stderr = error.ToString();
                return code;
            }
            Check(Cli(null, out _, out var unconfigured, "character", "kill", "alpha", "--apply") == 2 &&
                unconfigured.Contains(MagAdminApiSettings.ListenVariable), "kill without the admin API did not explain what it needs");
            Check(Cli(null, out _, out _, "character", "show", "alpha", "--reason", "x") == 2, "--reason was accepted outside kill");
            Check(Cli(null, out _, out _, "character", "kill", "alpha", "--reason") == 2, "A --reason without a value was accepted");

            var secret = Encoding.UTF8.GetBytes(new string('k', 40));
            var api = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), secret, store, Path.Combine(directory, "backups"), markers,
                () => DateTime.UtcNow, null);
            api.Start();
            try
            {
                var client = new MagAdminApiClient(api.LocalEndPoint, secret);
                Check(Cli(client, out _, out var offline, "character", "kill", "alpha", "--reason", "stuck in a seat") == 1 &&
                    offline.Contains("not_online"), $"A logged-out account was not refused as not_online: {offline}");
                MagActiveAccounts.Write(Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json"),
                    new Dictionary<string, DateTime> { ["alpha"] = now.AddMinutes(2) }, now);
                Check(Cli(client, out _, out var lobby, "character", "kill", "alpha") == 1 && lobby.Contains("not_in_match"),
                    $"An online account outside any match was not refused as not_in_match: {lobby}");
                Check(Cli(client, out _, out var unknown, "character", "kill", "zulu", "--apply") == 1 && unknown.Contains("not_found"),
                    $"An unknown account was not refused as not_found: {unknown}");
                var wrongSecret = new MagAdminApiClient(api.LocalEndPoint, Encoding.UTF8.GetBytes(new string('x', 40)));
                Check(Cli(wrongSecret, out _, out var unauthorized, "character", "kill", "alpha") == 1 && unauthorized.Contains("unauthorized"),
                    $"A wrong secret was not reported: {unauthorized}");
            }
            finally { api.DisposeAsync().AsTask().GetAwaiter().GetResult(); }
        }
        finally { Directory.Delete(directory, true); }
    }
}

internal static partial class LeadershipProgressionChecks
{
    public static void RegistryAdminKill() => WithDirectory(directory =>
    {
        var store = new LocalCharacterStore(Path.Combine(directory, "characters.json")) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        foreach (var (account, faction) in new[] { ("alpha", (byte)0), ("bravo", (byte)1), ("charlie", (byte)2), ("delta", (byte)0), ("echo", (byte)1) })
        {
            var create = new MagCreateCharacterRequest { AccountName = account, Faction = faction, UniverseId = 26, NpId = new byte[36] };
            Check(store.CreateForAccount(create, account).Result == MagNetResult.Success, "Synthetic faction creation failed");
        }
        string markers = Path.Combine(directory, "markers");
        Directory.CreateDirectory(markers);
        void Online(params string[] accounts) => MagActiveAccounts.Write(Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json"),
            accounts.ToDictionary(a => a, _ => DateTime.UtcNow.AddMinutes(2)), DateTime.UtcNow);
        Run().GetAwaiter().GetResult();

        async Task Run()
        {
            var manager = new Server.Dme.MediusManager(20231) { LocalOnly = true };
            using var progressionGate = new SemaphoreSlim(1, 1);
            await using var firstWorker = new RegistryWorker("first") { SupportsAdminKill = true };
            await using var secondWorker = new RegistryWorker("second");
            await using var first = new LocalDmeHost(firstWorker.Client, store, Options("first", 1), progressionGate: progressionGate);
            await using var second = new LocalDmeHost(secondWorker.Client, store, Options("second", 2), progressionGate: progressionGate);
            await Task.WhenAll(first.StartInRegistryAsync(manager), second.StartInRegistryAsync(manager));
            var hosts = new[] { new MagAdminGameplayHost("first", first), new MagAdminGameplayHost("second", second) };

            MagMlsSession Session(string account)
            {
                var settings = Server.Medius.Program.Settings;
                int previous = settings.MagApplicationId;
                settings.MagApplicationId = 20231;
                try { return MakeSession(store, 20231, account, store.ReadForAccount(account).Single().Id); }
                finally { settings.MagApplicationId = previous; }
            }
            int transaction = 0;
            Task<MagJoinGameResponse> Queue(LocalDmeHost host, string account)
            {
                var reply = new TaskCompletionSource<MagJoinGameResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
                host.QueueJoin(Session(account), new MagRetailJoinGameRequest { TransactionId = ++transaction, Preference = 1, Value = 2 },
                    m => { if (m is MagJoinGameResponse response) reply.TrySetResult(response); }, () => true);
                return reply.Task;
            }
            Check(first.TryStageRegistryFormation("profile-0-1"), "The first worker could not stage a match");
            var firstJoins = new[] { Queue(first, "alpha"), Queue(first, "bravo") };
            await first.TickWorldAsync();
            Check((await Task.WhenAll(firstJoins).WaitAsync(AdmissionTimeout)).All(j => j.Result == 0), "The first match did not admit");
            Check(second.TryStageRegistryFormation("profile-0-2"), "The second worker could not stage a match");
            var secondJoins = new[] { Queue(second, "charlie"), Queue(second, "delta") };
            await second.TickWorldAsync();
            Check((await Task.WhenAll(secondJoins).WaitAsync(AdmissionTimeout)).All(j => j.Result == 0), "The second match did not admit");

            JsonObject Reservation(RegistryWorker worker, string account) =>
                worker.Reservations.Single(r => r["account_id"]!.GetValue<uint>() == store.ReadForAccount(account).Single().Id);
            var alpha = Reservation(firstWorker, "alpha");
            ConnectFixture(first, alpha);
            ConnectFixture(second, Reservation(secondWorker, "charlie"));
            Online("alpha", "bravo", "charlie", "delta", "echo");
            int Kills(RegistryWorker worker) => worker.Requests.Count(r => r["op"]!.GetValue<string>() == "admin_kill");
            Task<MagAdminKillResult> Kill(string account, bool apply = true, string reason = "stuck in a parachute") =>
                MagAdminKill.KillAsync(store, hosts, account, apply, reason, "synthetic staff", markers, DateTime.UtcNow);

            // A dry run names the player and its world without asking the worker.
            var plan = await Kill("alpha", apply: false);
            Check(plan.Refusal == MagAdminKillRefusal.None && !plan.Applied && plan.WorkerId == "first" &&
                plan.WorldId == (uint)first.World.WorldId && plan.PlayerId == alpha["player_id"]!.GetValue<ushort>() && Kills(firstWorker) == 0,
                "A dry run did not locate the player or still asked the worker");

            // The request reaches only the owning worker and names exactly that player.
            var killed = await Kill("alpha");
            Check(killed.Refusal == MagAdminKillRefusal.None && killed.Applied && killed.Result == "killed", $"The kill was not applied: {killed.Message}");
            var sent = firstWorker.Requests.Single(r => r["op"]!.GetValue<string>() == "admin_kill");
            Check(sent["reservation_id"]!.GetValue<string>() == alpha["reservation_id"]!.GetValue<string>() &&
                sent["world_id"]!.GetValue<uint>() == (uint)first.World.WorldId &&
                sent["player_id"]!.GetValue<ushort>() == alpha["player_id"]!.GetValue<ushort>() &&
                sent["operator"]!.GetValue<string>() == "synthetic staff" && sent["reason"]!.GetValue<string>() == "stuck in a parachute" &&
                sent["v"]!.GetValue<int>() == 2 && Kills(secondWorker) == 0,
                "The admin_kill did not name the exact player and world or reached another worker");
            Check(first.CanUse(first.World.Manager.GetClientByAccessToken(alpha["access_key"]!.GetValue<string>())),
                "An operator kill released the player instead of leaving it to respawn");

            // A downed player's give-up and the worker's own refusals.
            firstWorker.AdminKillReply = _ => new JsonObject { ["v"] = 2, ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray(),
                ["admin_kill"] = new JsonObject { ["result"] = "gave_up" } };
            Check((await Kill("alpha")).Result == "gave_up", "A give-up was not reported");
            JsonObject Refusal(string code) => new() { ["v"] = 2, ["ok"] = false, ["error"] = code, ["messages"] = new JsonArray() };
            foreach (var (code, refusal) in new[] { ("not_alive", MagAdminKillRefusal.NotAlive), ("not_in_match", MagAdminKillRefusal.NotInMatch),
                ("match_finished", MagAdminKillRefusal.MatchFinished), ("invalid_json_or_schema", MagAdminKillRefusal.WorkerRefused) })
            {
                firstWorker.AdminKillReply = _ => Refusal(code);
                var refused = await Kill("alpha");
                Check(refused.Refusal == refusal && !refused.Applied && refused.Code != null, $"Worker refusal {code} was reported as {refused.Refusal}");
                Check(first.CanUse(first.World.Manager.GetClientByAccessToken(alpha["access_key"]!.GetValue<string>())),
                    $"Worker refusal {code} invalidated the player");
            }
            firstWorker.AdminKillReply = _ => new JsonObject { ["v"] = 2, ["ok"] = true, ["error"] = null, ["messages"] = new JsonArray(),
                ["admin_kill"] = new JsonObject { ["result"] = "refused", ["reason"] = "mounted_death_not_supported" } };
            var denied = await Kill("alpha");
            Check(denied.Refusal == MagAdminKillRefusal.KillRefused && denied.Code == "kill_refused" && denied.WorkerReason == "mounted_death_not_supported",
                "A denied KillSelf was not reported with its reason");

            // A worker without the capability is refused at Medius and never sees the op.
            var unsupported = await Kill("charlie");
            Check(unsupported.Refusal == MagAdminKillRefusal.WorkerUnsupported && unsupported.Code == "worker_unsupported" &&
                unsupported.WorkerId == "second" && Kills(secondWorker) == 0, "A worker without admin_kill_v1 was asked or not refused");
            Check((await Kill("charlie", apply: false)).Refusal == MagAdminKillRefusal.WorkerUnsupported,
                "A dry run did not report the missing capability");

            // Reserved but never connected, online in the lobby, logged out, unknown and a bad reason.
            Check((await Kill("bravo")).Refusal == MagAdminKillRefusal.NotInMatch, "A player still joining was not refused as not_in_match");
            Check((await Kill("echo")).Refusal == MagAdminKillRefusal.NotInMatch, "An account in the lobby was not refused as not_in_match");
            Online("alpha", "bravo", "charlie", "delta");
            Check((await Kill("echo")).Code == "not_online", "A logged-out account was not refused as not_online");
            Check((await Kill("zulu")).Code == "not_found", "An unknown account was not refused as not_found");
            Check((await Kill("alpha", reason: "line\nbreak")).Refusal == MagAdminKillRefusal.InvalidReason, "A reason with a line break was accepted");
            Check(Kills(secondWorker) == 0 && Kills(firstWorker) == 7, $"Refusals at Medius reached a worker ({Kills(firstWorker)} kills sent)");

            // The admin API serves the same operation.
            var secret = Encoding.UTF8.GetBytes(new string('s', 40));
            var api = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), secret, store, Path.Combine(directory, "backups"), markers,
                () => DateTime.UtcNow, null, gameplayHosts: () => hosts);
            api.Start();
            try
            {
                firstWorker.AdminKillReply = null;
                var client = new MagAdminApiClient(api.LocalEndPoint, secret);
                (int Status, JsonElement? Body) Post(string account, string body) =>
                    client.Send("POST", $"/admin/v1/characters/{account}/kill", Encoding.UTF8.GetBytes(body), "synthetic staff", DateTime.UtcNow);
                var applied = Post("alpha", "{\"apply\":true,\"reason\":\"stuck in a seat\"}");
                Check(applied.Status == 200 && applied.Body.Value.GetProperty("applied").GetBoolean() &&
                    applied.Body.Value.GetProperty("result").GetString() == "killed" &&
                    applied.Body.Value.GetProperty("worker_id").GetString() == "first", "The admin API did not apply the kill");
                var dry = Post("alpha", "{\"apply\":false}");
                Check(dry.Status == 200 && !dry.Body.Value.GetProperty("applied").GetBoolean(), "The admin API dry run applied or failed");
                var unsupportedApi = Post("charlie", "{\"apply\":true}");
                Check(unsupportedApi.Status == 409 && unsupportedApi.Body.Value.GetProperty("error").GetString() == "worker_unsupported",
                    "The admin API did not refuse a worker without the capability");
                Check(Post("zulu", "{\"apply\":true}").Status == 404, "The admin API did not 404 an unknown account");
                foreach (var bad in new[] { "{}", "{\"apply\":1}", "{\"apply\":true,\"reason\":5}", "{\"apply\":true,\"extra\":1}", "[]" })
                    Check(Post("alpha", bad).Status == 400, $"The admin API accepted a bad kill body: {bad}");
                Check(client.Send("GET", "/admin/v1/characters/alpha/kill", null, "synthetic staff", DateTime.UtcNow).Status == 405,
                    "The kill endpoint accepted GET");
            }
            finally { await api.DisposeAsync(); }
        }

        static LocalDmeHostOptions Options(string name, int opposingFaction) =>
            new(20231, 2, 2, 8, new[] { 1, 1 }, new[] { 0, opposingFaction }, IPAddress.Parse("192.0.2.9"), 12073, name);
    });
}
