using System.Net;
using System.Text;
using System.Text.Json;
using Npgsql;
using Server.Medius;

// Synthetic voice moderation checks: invented accounts, operators, reasons and times; no game data.
internal static partial class LeadershipProgressionChecks
{
    private static readonly DateTimeOffset VoiceNow = new DateTimeOffset(2026, 11, 27, 12, 0, 0, TimeSpan.Zero);

    /// <summary>A JSON store with plain writes (durability is not under test) and two characters.</summary>
    private static LocalCharacterStore VoiceStore(string directory, Func<DateTimeOffset> clock, string file = "characters.json")
    {
        var store = new LocalCharacterStore(Path.Combine(directory, file), new PlainCommitter()) { ProgressionCatalog = Load(directory, LegacyCatalog()) };
        store.Clock = clock;
        Create(store, "alpha", "Alpha");
        Create(store, "bravo", "Bravo");
        return store;
    }

    /// <summary>The same moderation sequence for any backend; every outcome is recorded for parity.</summary>
    private static List<string> RunVoiceScript(LocalCharacterStore store, string backups, Action<TimeSpan> advance)
    {
        var log = new List<string>();
        string Show(VoiceSanction s) => s == null ? "none" :
            $"{VoiceSanction.Name(s.Kind)}/{s.Operator}/{s.Reason ?? "-"}/{s.CreatedAt.ToUnixTimeSeconds()}/{s.ExpiresAt?.ToUnixTimeSeconds().ToString() ?? "open"}";
        void Step(string what, string account, VoiceSanctionKind kind, bool lift, string reason, TimeSpan? duration, bool apply, string by = "staff")
        {
            var c = store.ChangeVoiceSanction(account, kind, lift, reason, by, duration, apply, backups);
            log.Add($"{what}: refusal={c.Refusal ?? "-"} applied={c.Applied} op={c.Operation} before={Show(c.Before)} after={Show(c.After)}");
        }
        void Active() => log.Add("active: " + string.Join(",", MagVoiceModeration.Active(store, store.Clock()).Select(s => s.Account + ":" + Show(s))));

        Step("dry mute", "alpha", VoiceSanctionKind.Mute, false, "synthetic spam", TimeSpan.FromMinutes(30), false);
        Active();
        Step("mute", "alpha", VoiceSanctionKind.Mute, false, "synthetic spam", TimeSpan.FromMinutes(30), true);
        Step("mute again", "alpha", VoiceSanctionKind.Mute, false, "synthetic repeat", TimeSpan.FromHours(2), true);
        Step("ban", "bravo", VoiceSanctionKind.Ban, false, null, null, true);
        Step("unknown account", "nobody", VoiceSanctionKind.Mute, false, null, null, true);
        Step("invalid account", "x", VoiceSanctionKind.Mute, false, null, null, true);
        Step("unban without ban", "alpha", VoiceSanctionKind.Ban, true, null, null, true);
        Step("lift with duration", "alpha", VoiceSanctionKind.Mute, true, null, TimeSpan.FromMinutes(5), true);
        Step("too short", "alpha", VoiceSanctionKind.Mute, false, null, TimeSpan.FromSeconds(30), true);
        Step("too long", "alpha", VoiceSanctionKind.Mute, false, null, TimeSpan.FromDays(3651), true);
        Step("bad reason", "alpha", VoiceSanctionKind.Mute, false, "syntheticé", null, true);
        Step("bad operator", "alpha", VoiceSanctionKind.Mute, false, null, null, true, by: "");
        Active();
        advance(TimeSpan.FromHours(2));
        Active();
        Step("unmute expired", "alpha", VoiceSanctionKind.Mute, true, null, null, true);
        Step("dry unban", "bravo", VoiceSanctionKind.Ban, true, "synthetic appeal", null, false);
        Step("unban", "bravo", VoiceSanctionKind.Ban, true, "synthetic appeal", null, true);
        Step("ban alpha", "alpha", VoiceSanctionKind.Ban, false, "synthetic", TimeSpan.FromDays(7), true);
        Active();
        log.Add("stored: " + string.Join(",", store.ReadVoiceSanctions().Select(s => s.Account + ":" + Show(s))));
        return log;
    }

    public static void VoiceSanctionStore() => WithDirectory(directory =>
    {
        var clock = VoiceNow;
        var store = VoiceStore(directory, () => clock);
        string path = Path.Combine(directory, "characters.json"), backups = Path.Combine(directory, "backups");
        Check(!File.ReadAllText(path).Contains("voiceSanctions"), "A store without sanctions writes no sanctions field");
        byte[] original = File.ReadAllBytes(path);
        var log = RunVoiceScript(store, backups, step => clock = clock.Add(step));
        string Line(string prefix) => log.Single(line => line.StartsWith(prefix + ":", StringComparison.Ordinal));
        Check(Line("dry mute").Contains("applied=False") && Line("dry mute").Contains("after=mute/staff/synthetic spam/"), "Dry run plans the mute: " + Line("dry mute"));
        Check(log[1] == "active: ", "A dry run wrote a sanction: " + log[1]);
        Check(Line("mute").Contains("refusal=- applied=True op=voice-mute before=none"), Line("mute"));
        Check(Line("mute again").Contains("before=mute/staff/synthetic spam") && Line("mute again").Contains("after=mute/staff/synthetic repeat"),
            "Imposing again replaces: " + Line("mute again"));
        Check(Line("ban").Contains("after=ban/staff/-/") && Line("ban").EndsWith("/open"), "A ban without a duration lasts until lifted: " + Line("ban"));
        foreach (var (step, refusal) in new[] { ("unknown account", "account-not-found"), ("invalid account", "account-not-found"),
            ("unban without ban", "not-sanctioned"), ("lift with duration", "invalid-duration"), ("too short", "invalid-duration"),
            ("too long", "invalid-duration"), ("bad reason", "invalid-reason"), ("bad operator", "invalid-operator"), ("unmute expired", "not-sanctioned") })
            Check(Line(step).Contains($"refusal={refusal} applied=False"), $"{step}: {Line(step)}");
        Check(log.Count(line => line.StartsWith("active:")) == 4, "Active snapshots");
        var actives = log.Where(line => line.StartsWith("active:")).ToArray();
        Check(actives[1].Contains("alpha:mute") && actives[1].Contains("bravo:ban"), "Both sanctions active: " + actives[1]);
        Check(!actives[2].Contains("alpha:") && actives[2].Contains("bravo:ban"), "An expired mute is not active: " + actives[2]);
        Check(Line("unban").Contains("applied=True op=voice-unban") && Line("unban").Contains("after=none"), Line("unban"));
        Check(actives[3].Contains("alpha:ban") && !actives[3].Contains("bravo"), "Final active: " + actives[3]);
        Check(Line("stored").Contains("alpha:ban") && Line("stored").Contains("alpha:mute") && !Line("stored").Contains("bravo"),
            "An expired sanction stays stored until replaced; a lifted one leaves: " + Line("stored"));

        // Every applied change keeps the file as it was before it; dry runs and refusals keep nothing.
        var kept = Directory.GetFiles(backups).Select(Path.GetFileName).OrderBy(n => n, StringComparer.Ordinal).ToArray();
        Check(kept.Length == 5 && kept.Count(n => n.StartsWith("characters.before-admin-voice-mute-alpha-")) == 2 &&
            kept.Count(n => n.StartsWith("characters.before-admin-voice-unban-bravo-")) == 1, "Backups: " + string.Join(", ", kept));
        Check(Directory.GetFiles(backups).Any(file => File.ReadAllBytes(file).SequenceEqual(original)), "The first backup holds the store before any sanction");
        var json = JsonDocument.Parse(File.ReadAllBytes(path)).RootElement.GetProperty("voiceSanctions");
        Check(json.GetArrayLength() == 2 && json[0].GetProperty("kind").GetString() == "ban" && json[0].GetProperty("account").GetString() == "alpha",
            "Stored sanctions are ordered by account and kind");

        // A store with an invalid sanction is refused, like any other invalid record.
        foreach (var (name, sanction) in new[] {
            // A sanction may outlive its account's characters (character slots), but never names an invalid account.
            ("invalid account", """{"account":"No Body","kind":"mute","operator":"staff","createdAt":1}"""),
            ("unknown kind", """{"account":"alpha","kind":"gag","operator":"staff","createdAt":1}"""),
            ("no operator", """{"account":"alpha","kind":"mute","operator":"","createdAt":1}"""),
            ("ends before it starts", """{"account":"alpha","kind":"mute","operator":"staff","createdAt":10,"expiresAt":10}"""),
            ("duplicate", """{"account":"bravo","kind":"mute","operator":"staff","createdAt":1},{"account":"bravo","kind":"mute","operator":"staff","createdAt":2}""") })
        {
            var root = System.Text.Json.Nodes.JsonNode.Parse(File.ReadAllText(path))!.AsObject();
            root["voiceSanctions"] = System.Text.Json.Nodes.JsonNode.Parse("[" + sanction + "]");
            string bad = Path.Combine(directory, "bad.json");
            File.WriteAllText(bad, root.ToJsonString());
            bool refused = false;
            try { new LocalCharacterStore(bad, new PlainCommitter()).ReadVoiceSanctions(); }
            catch (InvalidDataException) { refused = true; }
            Check(refused, "An invalid sanction was accepted: " + name);
        }
        // An older document (no field) and an unchanged character write keep the sanctions.
        Check(store.SaveNominationsForAccount("bravo", 2, 1).Result == RT.Models.MAG.MagNetResult.Success, "Synthetic character write");
        Check(store.ReadVoiceSanctions().Count == 2, "A character write dropped the sanctions");
    });

    public static void VoiceSanctionCacheRefresh() => WithDirectory(directory =>
    {
        var clock = VoiceNow;
        var store = VoiceStore(directory, () => clock);
        var log = new List<string>();
        var cache = new VoiceSanctionCache(store, () => clock, log.Add);
        string backups = Path.Combine(directory, "backups");
        Check(cache.Refresh() && cache.Count == 0 && cache.Silencing("alpha") == null, "Nothing silenced at first");
        Check(store.ChangeVoiceSanction("alpha", VoiceSanctionKind.Mute, false, null, "staff", TimeSpan.FromMinutes(5), true, backups).Applied, "mute");
        Check(cache.Silencing("alpha") == null, "The snapshot changed without a refresh");
        Check(cache.Refresh() && cache.Silencing("alpha")?.Kind == VoiceSanctionKind.Mute && cache.Silencing("bravo") == null, "The refresh loads the mute");
        Check(log.Last() == "Voice sanctions: 1 account(s) silenced (1 mute(s), 0 ban(s))", "Change logged with counts only: " + log.Last());
        int lines = log.Count;
        Check(cache.Refresh() && log.Count == lines, "An unchanged refresh logs nothing");
        // A ban outranks a mute; expiry is checked at each lookup.
        Check(store.ChangeVoiceSanction("alpha", VoiceSanctionKind.Ban, false, null, "staff", TimeSpan.FromMinutes(10), true, backups).Applied, "ban");
        cache.Refresh();
        Check(cache.Silencing("alpha")?.Kind == VoiceSanctionKind.Ban, "A ban is reported before a mute");
        clock = clock.AddMinutes(10);
        Check(cache.Silencing("alpha") == null, "Expired sanctions still silence");
        cache.Refresh();
        Check(cache.Count == 0, "Expired sanctions are not loaded");

        // A store that cannot be read keeps the last snapshot and logs once.
        Check(store.ChangeVoiceSanction("bravo", VoiceSanctionKind.Ban, false, null, "staff", null, true, backups).Applied, "standing ban");
        cache.Refresh();
        string path = Path.Combine(directory, "characters.json");
        string good = File.ReadAllText(path);
        File.WriteAllText(path, "{ not json");
        lines = log.Count;
        Check(!cache.Refresh() && !cache.Refresh() && cache.Silencing("bravo") != null, "A failed reload dropped the snapshot");
        Check(log.Count == lines + 1 && log.Last().Contains("reload failed"), "A failing store is logged once");
        File.WriteAllText(path, good);
        Check(cache.Refresh() && log.Any(line => line.Contains("readable again")), "Recovery is logged");
        Check(store.ChangeVoiceSanction("bravo", VoiceSanctionKind.Ban, true, null, "staff", null, true, backups).Applied, "unban");
        Check(cache.Refresh() && cache.Silencing("bravo") == null, "A lift is picked up");

        // A requested reload runs in the background and returns at once, even while a reload holds the lock;
        // a burst of requests is folded into the reloads already queued.
        Check(store.ChangeVoiceSanction("bravo", VoiceSanctionKind.Mute, false, null, "staff", TimeSpan.FromMinutes(5), true, backups).Applied, "mute");
        var timer = System.Diagnostics.Stopwatch.StartNew();
        for (int i = 0; i < 50; i++) cache.RequestRefresh();
        Check(timer.Elapsed < TimeSpan.FromMilliseconds(500), $"Requested reloads blocked the caller: {timer.Elapsed.TotalMilliseconds:0} ms");
        Check(Eventually(() => cache.Silencing("bravo") != null), "A requested reload did not load the mute");
        Check(store.ChangeVoiceSanction("bravo", VoiceSanctionKind.Mute, true, null, "staff", null, true, backups).Applied, "unmute");
        cache.RequestRefresh();
        Check(Eventually(() => cache.Silencing("bravo") == null), "A request after a burst was lost");
    });

    /// <summary>Polls <paramref name="condition"/> for up to ten seconds.</summary>
    private static bool Eventually(Func<bool> condition)
    {
        var wait = System.Diagnostics.Stopwatch.StartNew();
        while (!condition())
        {
            if (wait.Elapsed > TimeSpan.FromSeconds(10)) return false;
            Thread.Sleep(5);
        }
        return true;
    }

    private static readonly byte[] VoiceApiSecret = Encoding.UTF8.GetBytes("synthetic-voice-api-secret-0123456789abcdef");
    private static int _voiceNonce;

    private static MagAdminApiRequest VoiceRequest(string method, string path, DateTime now, string body = null, string actor = "staff@example.invalid")
    {
        byte[] bytes = body == null ? Array.Empty<byte>() : Encoding.UTF8.GetBytes(body);
        string ts = new DateTimeOffset(now).ToUnixTimeSeconds().ToString(), nonce = "synthetic-voice-" + (++_voiceNonce).ToString("D6");
        return new MagAdminApiRequest
        {
            Method = method, PathAndQuery = path, Body = bytes,
            Headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
            {
                [MagAdminApiAuthenticator.TimestampHeader] = ts, [MagAdminApiAuthenticator.NonceHeader] = nonce,
                [MagAdminApiAuthenticator.ActorHeader] = actor,
                [MagAdminApiAuthenticator.SignatureHeader] = MagAdminApiAuthenticator.Sign(VoiceApiSecret, method, path, ts, nonce, bytes),
            }
        };
    }

    public static void VoiceSanctionAdmin() => WithDirectory(directory =>
    {
        var clock = VoiceNow;
        var store = VoiceStore(directory, () => clock);
        DateTime now = clock.UtcDateTime;
        string path = Path.Combine(directory, "characters.json"), backups = Path.Combine(directory, "backups");
        (int code, string output, string error) Admin(params string[] args)
        {
            var output = new StringWriter(); var error = new StringWriter();
            int code = MagAdminCli.Run(args, store, backups, directory, now, output, error);
            return (code, output.ToString(), error.ToString());
        }

        // The CLI: dry run, apply, list, refusals and usage errors.
        byte[] original = File.ReadAllBytes(path);
        var dry = Admin("voice", "mute", "alpha", "--for", "30m", "--reason", "synthetic spam", "--operator", "staff");
        Check(dry.code == 0 && dry.output.Contains("dry run") && dry.output.Contains("mute until 2026-11-27T12:30:00Z, by staff: synthetic spam"),
            "CLI dry run: " + dry.output + dry.error);
        Check(File.ReadAllBytes(path).SequenceEqual(original) && !Directory.Exists(backups), "A CLI dry run wrote");
        var applied = Admin("voice", "mute", "alpha", "--for", "30m", "--reason", "synthetic spam", "--operator", "staff", "--apply");
        Check(applied.code == 0 && applied.output.Contains("applied: voice mute alpha") && applied.output.Contains("journal "), "CLI apply: " + applied.output + applied.error);
        Check(Admin("voice", "ban", "bravo", "--apply").code == 0, "CLI ban until lifted");
        var listed = Admin("voice", "list");
        Check(listed.code == 0 && listed.output.Contains("alpha") && listed.output.Contains("2026-11-27T12:30:00Z") &&
            listed.output.Contains("bravo") && listed.output.Contains("until lifted") && listed.output.Contains("mag-admin"), "CLI list: " + listed.output);
        Check(Admin("voice", "unban", "alpha", "--apply") is var none && none.code == 1 && none.error.Contains("refused (not_sanctioned)"), "CLI lift without sanction");
        Check(Admin("voice", "mute", "nobody", "--apply").error.Contains("refused (not_found)"), "CLI unknown account");
        foreach (var args in new[] { new[] { "voice", "mute", "alpha", "--for", "30" }, new[] { "voice", "mute", "alpha", "--for", "0m" },
            new[] { "voice", "mute", "alpha", "--for", "3651d" }, new[] { "voice", "unmute", "alpha", "--for", "5m" },
            new[] { "voice", "silence", "alpha" }, new[] { "character", "show", "alpha", "--for", "5m" },
            new[] { "voice", "mute", "alpha", "--reason", "syntheticé" } })
            Check(Admin(args).code == 2, "CLI usage error accepted: " + string.Join(" ", args));
        Check(MagVoiceModeration.TryParseDuration("2w", out var weeks) && weeks == TimeSpan.FromDays(14) &&
            MagVoiceModeration.TryParseDuration("12h", out var hours) && hours == TimeSpan.FromHours(12) &&
            !MagVoiceModeration.TryParseDuration("-5m", out _) && !MagVoiceModeration.TryParseDuration("5 m", out _), "Durations");
        Check(Admin("voice", "unmute", "alpha", "--apply").code == 0 && Admin("voice", "unban", "bravo", "--apply").code == 0, "CLI lifts");

        // The admin API: the same rules, the actor as operator, and the refresh hook only after an applied change.
        // The hook runs off the request: here it blocks until released, and every answer still comes at once.
        var log = new List<string>();
        int hooked = 0;
        using var release = new ManualResetEventSlim(false);
        var api = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), VoiceApiSecret, store, backups, directory, () => now,
            line => { lock (log) log.Add(line); }, voiceSanctionsChanged: () => { Interlocked.Increment(ref hooked); release.Wait(); });
        int refreshed = 0;
        void Hooked(int count)
        {
            Eventually(() => Volatile.Read(ref hooked) >= count);
            Thread.Sleep(20); // and no more than asked
            refreshed = Volatile.Read(ref hooked);
        }
        JsonElement Body(MagAdminApiResponse response) => JsonDocument.Parse(response.Body).RootElement;
        var dryApi = api.Handle(VoiceRequest("POST", "/admin/v1/voice/alpha/mute", now, """{"apply":false,"reason":"synthetic","duration_minutes":60}"""));
        Hooked(0);
        Check(dryApi.Status == 200 && !Body(dryApi).GetProperty("applied").GetBoolean() && refreshed == 0 &&
            Body(dryApi).GetProperty("after").GetProperty("expires_at").GetString() == "2026-11-27T13:00:00Z", "API dry run: " + Encoding.UTF8.GetString(dryApi.Body));
        var timer = System.Diagnostics.Stopwatch.StartNew();
        var mute = api.Handle(VoiceRequest("POST", "/admin/v1/voice/alpha/mute", now, """{"apply":true,"reason":"synthetic","duration_minutes":60}"""));
        Check(timer.Elapsed < TimeSpan.FromSeconds(1), $"The mute waited for its blocked reload: {timer.Elapsed.TotalMilliseconds:0} ms");
        Hooked(1);
        Check(mute.Status == 200 && Body(mute).GetProperty("applied").GetBoolean() && refreshed == 1 &&
            Body(mute).GetProperty("after").GetProperty("operator").GetString() == "staff@example.invalid" &&
            Body(mute).GetProperty("before").ValueKind == JsonValueKind.Null, "API mute: " + Encoding.UTF8.GetString(mute.Body));
        Check(log.Any(line => line.StartsWith("admin api: applied voice mute actor=\"staff@example.invalid\" account=alpha until=2026-11-27T13:00:00Z")),
            "API mute logged: " + string.Join(" | ", log.ToArray()));
        var ban = api.Handle(VoiceRequest("POST", "/admin/v1/voice/bravo/ban", now, """{"apply":true}"""));
        Hooked(2);
        Check(ban.Status == 200 && Body(ban).GetProperty("after").GetProperty("expires_at").ValueKind == JsonValueKind.Null && refreshed == 2, "API ban");
        var list = api.Handle(VoiceRequest("GET", "/admin/v1/voice", now));
        var sanctions = Body(list).GetProperty("sanctions");
        Check(list.Status == 200 && sanctions.GetArrayLength() == 2 && sanctions[0].GetProperty("account").GetString() == "alpha" &&
            sanctions[1].GetProperty("kind").GetString() == "ban", "API list: " + Encoding.UTF8.GetString(list.Body));
        var notSanctioned = api.Handle(VoiceRequest("POST", "/admin/v1/voice/alpha/unban", now, """{"apply":true}"""));
        Hooked(2);
        Check(notSanctioned.Status == 409 && Body(notSanctioned).GetProperty("error").GetString() == "not_sanctioned" && refreshed == 2, "API 409");
        Check(api.Handle(VoiceRequest("POST", "/admin/v1/voice/nobody/mute", now, """{"apply":true}""")).Status == 404, "API unknown account");
        Check(api.Handle(VoiceRequest("GET", "/admin/v1/voice/alpha/mute", now)).Status == 405, "API wrong method");
        Check(api.Handle(VoiceRequest("POST", "/admin/v1/voice", now, "{}")).Status == 405, "API list method");
        Check(api.Handle(VoiceRequest("POST", "/admin/v1/voice/alpha/silence", now, """{"apply":true}""")).Status == 404, "API unknown action");
        foreach (var body in new[] { "{}", """{"apply":"yes"}""", """{"apply":true,"minutes":5}""", """{"apply":true,"duration_minutes":0}""",
            """{"apply":true,"duration_minutes":5256001}""", """{"apply":true,"duration_minutes":1.5}""", """{"apply":true,"reason":5}""",
            """{"apply":true,"reason":"synthetic\u0001"}""", "[]", "not json" })
            Check(api.Handle(VoiceRequest("POST", "/admin/v1/voice/alpha/mute", now, body)).Status == 400, "API accepted: " + body);
        Check(api.Handle(VoiceRequest("POST", "/admin/v1/voice/alpha/unmute", now, """{"apply":true,"duration_minutes":5}""")).Status == 400,
            "API lift with a duration");
        var unmute = api.Handle(VoiceRequest("POST", "/admin/v1/voice/alpha/unmute", now, """{"apply":true,"reason":"synthetic appeal"}"""));
        Hooked(3);
        Check(unmute.Status == 200 && Body(unmute).GetProperty("after").ValueKind == JsonValueKind.Null && refreshed == 3, "API unmute");
        Check(store.ReadVoiceSanctions().Single().Account == "bravo", "API changes reach the store");
        release.Set();
    });

    public static void PostgresVoiceSanctions() => WithDirectory(directory => WithPostgres("postgres voice sanctions", (database, connection) =>
    {
        var clock = VoiceNow;
        var catalog = Load(directory, LegacyCatalog());
        var json = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new PlainCommitter()) { ProgressionCatalog = catalog, Clock = () => clock };
        var pg = LocalCharacterStore.ForPostgres(database);
        pg.ProgressionCatalog = catalog; pg.Clock = () => clock;
        foreach (var store in new[] { json, pg }) { Create(store, "alpha", "Alpha"); Create(store, "bravo", "Bravo"); }
        var start = clock;
        var expected = RunVoiceScript(json, Path.Combine(directory, "json-backups"), step => clock = clock.Add(step));
        clock = start;
        var actual = RunVoiceScript(pg, Path.Combine(directory, "pg-backups"), step => clock = clock.Add(step));
        for (int i = 0; i < Math.Max(expected.Count, actual.Count); i++)
            Check(i < expected.Count && i < actual.Count && expected[i] == actual[i],
                $"Backends disagree at step {i}: json '{(i < expected.Count ? expected[i] : "")}' postgres '{(i < actual.Count ? actual[i] : "")}'");
        var differences = pg.CompareWith(json);
        Check(differences.Count == 0, "Stores differ after the same moderation: " + string.Join("; ", differences));
        Check(pg.Summarize() == json.Summarize() && pg.Summarize().VoiceSanctions == 2 && pg.Summarize().Describe().Contains("2 voice sanctions"),
            "Digests and counts: " + pg.Summarize().Describe());
        Check(Scalar(connection, "SELECT count(*) FROM mag.admin_changes WHERE operation LIKE 'voice-%'") == 5, "Each applied change is journalled once");
        Check(Scalar(connection, "SELECT count(*) FROM mag.admin_changes WHERE operation = 'voice-unban' AND account = 'bravo' AND " +
            "before->'sanction'->>'kind' = 'ban' AND after->'sanction' = 'null'::jsonb AND after->>'reason' = 'synthetic appeal' AND " +
            "after->>'operator' = 'staff'") == 1, "The unban journal holds the sanction before, the operator and the reason");
        Check(Scalar(connection, "SELECT count(*) FROM mag.voice_sanctions WHERE account = 'alpha' AND expires_at IS NOT NULL") == 2, "Rows");
        // The schema refuses what the store would refuse.
        foreach (var sql in new[] {
            "INSERT INTO mag.voice_sanctions VALUES ('No Body', 'mute', NULL, 'staff', now(), NULL)",
            "INSERT INTO mag.voice_sanctions VALUES ('bravo', 'gag', NULL, 'staff', now(), NULL)",
            "INSERT INTO mag.voice_sanctions VALUES ('bravo', 'mute', NULL, '', now(), NULL)",
            "INSERT INTO mag.voice_sanctions VALUES ('bravo', 'mute', NULL, 'staff', now(), now() - interval '1 minute')",
            "INSERT INTO mag.voice_sanctions VALUES ('alpha', 'ban', NULL, 'staff', now(), NULL)" })
        {
            bool refused = false;
            try { Sql(connection, sql); } catch (PostgresException) { refused = true; }
            Check(refused, "The schema accepted: " + sql);
        }

        // Import and export carry the sanctions; a replace keeps them in step with the JSON store.
        string exported = Path.Combine(directory, "exported.json");
        pg.ExportTo(new LocalCharacterStore(exported, new PlainCommitter()) { ProgressionCatalog = catalog }, false);
        var roundTrip = new LocalCharacterStore(exported, new PlainCommitter()) { ProgressionCatalog = catalog };
        Check(roundTrip.ReadVoiceSanctions().Count == 2 && pg.CompareWith(roundTrip).Count == 0, "Export carries the sanctions");
        // Sanctions are per account since character slots (0003): an empty database has neither.
        Sql(connection, "DELETE FROM mag.characters; DELETE FROM mag.voice_sanctions");
        var imported = pg.ImportFrom(json);
        Check(imported.After.VoiceSanctions == 2 && pg.CompareWith(json).Count == 0, "Import carries the sanctions");

        // Upgrading a schema-1 database keeps its characters and adds the table.
        Sql(connection, "DROP SCHEMA mag CASCADE");
        Check(database.Migrate(1) == 1 && database.SchemaVersion() == 1, "Synthetic schema-1 database");
        Sql(connection, SchemaOneCharacters);
        Check(database.Migrate() == database.LatestSchemaVersion - 1 && database.SchemaVersion() == database.LatestSchemaVersion,
            "Migration 0002 and later apply to a schema-1 database");
        Check(pg.ReadAll().Count == 2 && pg.ReadVoiceSanctions().Count == 0, "The upgrade kept the characters");
        Check(pg.ChangeVoiceSanction("alpha", VoiceSanctionKind.Mute, false, null, "staff", null, true, null).Applied, "A sanction after the upgrade");
    }));
    /// <summary>
    /// The live failure of 2026-11-27: CNPG's synchronous standby was slow to acknowledge, so a voice mute's
    /// COMMIT sat in "waiting for synchronous replication" past the command timeout. The mute was saved on
    /// the primary yet the admin API answered 500 after five seconds and the portal timed out. Here a
    /// standby that never answers stands in: moderation must not wait for it, the relay's reload must not
    /// hold up the answer, and a commit cut off while waiting is reported as an unknown outcome, not a failure.
    /// </summary>
    public static void PostgresVoiceWritesIgnoreStalledStandby() => WithDirectory(directory => WithPostgres(
        "postgres voice writes answer at once while the synchronous standby stalls", (database, connection) =>
    {
        var clock = VoiceNow;
        var pg = LocalCharacterStore.ForPostgres(database);
        pg.ProgressionCatalog = Load(directory, LegacyCatalog()); pg.Clock = () => clock;
        Create(pg, "alpha", "Alpha"); Create(pg, "bravo", "Bravo");
        string name = new NpgsqlConnectionStringBuilder(connection).Database;
        string Text(string sql)
        {
            using var c = new NpgsqlConnection(connection);
            c.Open();
            using var command = new NpgsqlCommand(sql, c);
            return command.ExecuteScalar()?.ToString();
        }
        long SyncRepWaiters() => Scalar(connection,
            $"SELECT count(*) FROM pg_stat_activity WHERE datname = '{name}' AND wait_event = 'SyncRep'");
        // The check server runs with synchronous_commit=off; only this throwaway database turns it back on,
        // so only its commits wait for the standby that is never there.
        Sql(connection, "ALTER SYSTEM SET synchronous_standby_names = 'mag_check_absent_standby'");
        try
        {
            Sql(connection, "SELECT pg_reload_conf()");
            Sql(connection, $"ALTER DATABASE {name} SET synchronous_commit = on");
            NpgsqlConnection.ClearAllPools();
            Check(Eventually(() => Text("SHOW synchronous_standby_names") == "mag_check_absent_standby" && Text("SHOW synchronous_commit") == "on"),
                "The stalled standby was not configured");

            // Fresh connections, so the database's setting applies (pooled ones keep the session they began with).
            using var waiting = new PostgresCharacterDatabase(connection) { Clock = () => clock };
            pg = LocalCharacterStore.ForPostgres(waiting);
            pg.ProgressionCatalog = Load(directory, LegacyCatalog()); pg.Clock = () => clock;
            DateTime now = clock.UtcDateTime;
            var log = new List<string>();
            void Log(string line) { lock (log) log.Add(line); }
            var cache = new VoiceSanctionCache(pg, () => clock);
            Check(cache.Refresh() && cache.Silencing("alpha") == null, "Nothing silenced at first");
            string markers = Path.Combine(directory, "markers"); Directory.CreateDirectory(markers);
            MagActiveAccounts.Write(Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json"), new Dictionary<string, DateTime>(), now);
            var api = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), VoiceApiSecret, pg, Path.Combine(directory, "backups"), markers, () => now, Log,
                voiceSanctionsChanged: cache.RequestRefresh);

            // A character write waits for the standby, as every commit did on live.
            var level = Task.Run(() => api.Handle(VoiceRequest("POST", "/admin/v1/characters/bravo/level", now, """{"level":3,"apply":true}""")));
            Check(Eventually(() => SyncRepWaiters() == 1 || level.IsCompleted) && !level.IsCompleted, "The level change did not wait for the absent standby: " +
                (level.IsCompleted ? $"{level.Result.Status} {Encoding.UTF8.GetString(level.Result.Body)} {string.Join(" | ", log.ToArray())}" : "no SyncRep wait"));

            // A mute, a ban and their lifts do not, and the relay's cache picks each up in the background.
            foreach (var (action, silenced) in new[] { ("mute", true), ("ban", true), ("unmute", true), ("unban", false) })
            {
                var timer = System.Diagnostics.Stopwatch.StartNew();
                var answer = api.Handle(VoiceRequest("POST", $"/admin/v1/voice/alpha/{action}", now,
                    action.StartsWith("un") ? """{"apply":true}""" : """{"apply":true,"reason":"synthetic","duration_minutes":60}"""));
                Check(answer.Status == 200 && JsonDocument.Parse(answer.Body).RootElement.GetProperty("applied").GetBoolean(),
                    $"API {action} under a stalled standby: {answer.Status} {Encoding.UTF8.GetString(answer.Body)}");
                Check(timer.Elapsed < TimeSpan.FromSeconds(1), $"The voice {action} took {timer.Elapsed.TotalMilliseconds:0} ms");
                Check(Eventually(() => (cache.Silencing("alpha") != null) == silenced), $"The cache did not follow the {action}");
            }
            Check(Scalar(connection, "SELECT count(*) FROM mag.admin_changes WHERE operation LIKE 'voice-%' AND account = 'alpha'") == 4,
                "Each voice change is journalled");
            Check(SyncRepWaiters() == 1 && !level.IsCompleted, "The level change stopped waiting by itself");

            // Cut off while waiting (what Npgsql's timeout and cancel did on live): the primary keeps the change,
            // so the API says the outcome is unknown instead of failed.
            Sql(connection, $"SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{name}' AND wait_event = 'SyncRep'");
            Check(level.Wait(TimeSpan.FromSeconds(10)), "The level change did not return after its connection ended");
            var cut = level.Result;
            Check(cut.Status == 503 && JsonDocument.Parse(cut.Body).RootElement.GetProperty("error").GetString() == "outcome_unknown",
                $"A commit cut off while waiting: {cut.Status} {Encoding.UTF8.GetString(cut.Body)}");
            Check(log.ToArray().Any(line => line.Contains("outcome unknown")), "The unknown outcome is logged: " + string.Join(" | ", log.ToArray()));
            Thread.Sleep(PostgresCharacterDatabase.UnavailableBackoff + TimeSpan.FromMilliseconds(100));
            Check(pg.ReadForAccount("bravo").Single().ToRecord().Level == 3, "The cut-off level change was not kept by the primary");
        }
        finally
        {
            Sql(connection, "ALTER SYSTEM RESET synchronous_standby_names");
            Sql(connection, "SELECT pg_reload_conf()");
        }
    }));
}
