using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using RT.Models.MAG;
using Server.Medius;

// The internal admin API over a synthetic store and progression catalog (AdminCliChecks.Catalog):
// configuration, request signing, each endpoint, dry run versus apply with backup, and refusals.
// Secrets, accounts and actors are invented; nothing here starts Medius or reads real data.
internal static class AdminApiChecks
{
    static void Check(bool value, string message) { if (!value) throw new Exception(message); }

    static readonly byte[] Secret = Encoding.UTF8.GetBytes("synthetic-admin-api-secret-0123456789abcdef");
    const string Actor = "staff@example.invalid";
    static int _nonce;

    static MagAdminApiRequest Signed(string method, string path, DateTime now, string body = null, byte[] secret = null,
        string nonce = null, long? timestamp = null, string actor = Actor, string omit = null, Func<string, string> tamper = null)
    {
        byte[] bytes = body == null ? Array.Empty<byte>() : Encoding.UTF8.GetBytes(body);
        string ts = (timestamp ?? new DateTimeOffset(now).ToUnixTimeSeconds()).ToString();
        nonce ??= "synthetic-nonce-" + (++_nonce).ToString("D6");
        string signature = MagAdminApiAuthenticator.Sign(secret ?? Secret, method, path, ts, nonce, bytes);
        var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
        {
            [MagAdminApiAuthenticator.TimestampHeader] = ts,
            [MagAdminApiAuthenticator.NonceHeader] = nonce,
            [MagAdminApiAuthenticator.ActorHeader] = actor,
            [MagAdminApiAuthenticator.SignatureHeader] = tamper == null ? signature : tamper(signature),
        };
        if (omit != null) headers.Remove(omit);
        return new MagAdminApiRequest { Method = method, PathAndQuery = path, Headers = headers, Body = bytes };
    }

    static JsonElement Json(MagAdminApiResponse response) => JsonDocument.Parse(response.Body).RootElement;
    static string Error(MagAdminApiResponse response) => Json(response).GetProperty("error").GetString();

    // Shared signing vectors with the portal client (computed independently with Python's hmac).
    public static void Vectors()
    {
        byte[] secret = Encoding.ASCII.GetBytes("0123456789abcdef0123456789abcdef");
        Check(MagAdminApiAuthenticator.BodyHash(ReadOnlySpan<byte>.Empty) == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "Empty body hash");
        byte[] body = Encoding.ASCII.GetBytes("{\"level\":20,\"apply\":false}");
        Check(MagAdminApiAuthenticator.BodyHash(body) == "004aee1f47f0f579d4d17b066506e4a169ceae956f126cd39547165a49ceb77f", "Vector 1 body hash");
        Check(MagAdminApiAuthenticator.Sign(secret, "POST", "/admin/v1/characters/alpha/level", "1800000000", "test-nonce-0123456789", body) ==
            "67830476b109612f9f91f3c0b40f996288f6324dafe3e5d173d7cf1152727e9c", "Vector 1 signature");
        Check(MagAdminApiAuthenticator.Sign(secret, "GET", "/admin/v1/accounts", "1800000000", "test-nonce-0123456789", ReadOnlySpan<byte>.Empty) ==
            "75f73e3794341809c53bc58ac4a3a854e390514e6473d24aaf845a81f445e453", "Vector 2 signature");

        // The verifier accepts the vector request at its timestamp, with the secret read from a
        // file exactly as configured (surrounding whitespace trimmed, no decoding).
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-admin-api-vectors-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string file = Path.Combine(directory, "secret"); File.WriteAllText(file, "  0123456789abcdef0123456789abcdef\n");
            var settings = MagAdminApiSettings.Load(name => name == MagAdminApiSettings.ListenVariable ? "127.0.0.1:10490" :
                name == MagAdminApiSettings.SecretFileVariable ? file : null);
            Check(settings.Secret.SequenceEqual(secret), "Vector secret read from file");
            var verifier = new MagAdminApiAuthenticator(settings.Secret);
            var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
            {
                [MagAdminApiAuthenticator.TimestampHeader] = "1800000000",
                [MagAdminApiAuthenticator.NonceHeader] = "test-nonce-0123456789",
                [MagAdminApiAuthenticator.ActorHeader] = Actor,
                [MagAdminApiAuthenticator.SignatureHeader] = "67830476b109612f9f91f3c0b40f996288f6324dafe3e5d173d7cf1152727e9c",
            };
            Check(verifier.Verify("POST", "/admin/v1/characters/alpha/level", headers, body, DateTime.UnixEpoch.AddSeconds(1800000000), out var actor) == null &&
                actor == Actor, "Vector 1 verifies");
        }
        finally { Directory.Delete(directory, true); }
    }

    public static void Settings()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-admin-api-settings-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            string secretText = "synthetic-secret-value-that-is-long-enough-0001";
            string secretFile = Path.Combine(directory, "secret"); File.WriteAllText(secretFile, secretText + "\n");
            string shortFile = Path.Combine(directory, "short"); File.WriteAllText(shortFile, "synthetic-short-secret-value-31");
            MagAdminApiSettings Load(string listen, string file) => MagAdminApiSettings.Load(name =>
                name == MagAdminApiSettings.ListenVariable ? listen : name == MagAdminApiSettings.SecretFileVariable ? file : null);
            string Fails(string listen, string file)
            {
                try { Load(listen, file); }
                catch (ArgumentException e) { Check(!e.Message.Contains(secretText) && !e.Message.Contains("synthetic-short"), "Error echoes the secret"); return e.Message; }
                throw new Exception($"Admin API settings accepted listen={listen} file={file}");
            }

            // Off by default: with neither variable (or only empty values) nothing is configured.
            Check(Load(null, null) == null && Load("", " ") == null, "Admin API must be off without configuration");
            Check(Fails("0.0.0.0:10490", null).Contains(MagAdminApiSettings.SecretFileVariable), "Listen alone must name the missing secret file");
            Check(Fails(null, secretFile).Contains(MagAdminApiSettings.ListenVariable), "Secret alone must name the missing listen address");
            Check(Fails("0.0.0.0:10490", shortFile).Contains("at least 32 bytes"), "Short secret refused");
            Check(Fails("0.0.0.0:10490", Path.Combine(directory, "missing")).Contains("cannot be read"), "Missing secret file refused");
            foreach (var bad in new[] { "nope", "0.0.0.0", "0.0.0.0:0", "0.0.0.0:70000", "admin.example.invalid:10490", "0.0.0.0:+10490" })
                Fails(bad, secretFile);
            var settings = Load("0.0.0.0:10490", secretFile);
            Check(settings.Listen.Equals(new IPEndPoint(IPAddress.Any, 10490)), "Listen address parsed");
            Check(settings.Secret.SequenceEqual(Encoding.UTF8.GetBytes(secretText)), "Secret file is read with surrounding whitespace trimmed");
            Check(Load("[::]:10490", secretFile).Listen.Equals(new IPEndPoint(IPAddress.IPv6Any, 10490)), "IPv6 listen address parsed");

            // Main role only: never in the MAPS process, and only where the store is configured.
            var main = new Server.Medius.Config.ServerSettings { MagCharacterStorePath = "/accounts/characters.json", EnableMagMasAdapter = true };
            MagAdminApiSettings.RequireMainRole(main);
            var maps = new Server.Medius.Config.ServerSettings { MagCharacterStorePath = "/accounts/characters.json", EnableMagPluginAdapter = true };
            bool refused = false;
            try { MagAdminApiSettings.RequireMainRole(maps); } catch (ArgumentException e) { refused = e.Message.Contains("MAPS"); }
            Check(refused, "Admin API must refuse the MAPS role");
            refused = false;
            try { MagAdminApiSettings.RequireMainRole(new Server.Medius.Config.ServerSettings()); } catch (ArgumentException) { refused = true; }
            Check(refused, "Admin API needs the character store");
        }
        finally { Directory.Delete(directory, true); }
    }

    public static void Run()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-admin-api-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var now = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
            string catalogPath = Path.Combine(directory, "progression.json"); AdminCliChecks.Catalog(catalogPath);
            var progression = MagProgressionCatalog.Load(catalogPath, 20231);
            string storePath = Path.Combine(directory, "characters.json");
            var store = new LocalCharacterStore(storePath) { ProgressionCatalog = progression };
            Check(store.CreateForAccount(new MagCreateCharacterRequest { AccountName = "Example", Faction = 0, UniverseId = 26, NpId = new byte[36] },
                "synthetic").Result == MagNetResult.Success, "Synthetic character creation");
            string backups = MagAdminOperations.BackupDirectory(storePath);
            string markers = Path.Combine(directory, "markers"); Directory.CreateDirectory(markers);
            string marker = Path.Combine(markers, MagActiveAccounts.FilePrefix + "main.json");
            var log = new List<string>();
            var api = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), Secret, store, backups, markers, () => now, log.Add);

            // Authentication: every refusal is the same 401 body and logs only the reason class.
            Check(api.Handle(Signed("GET", "/admin/v1/accounts", now)).Status == 200, "Signed request accepted");
            foreach (var header in new[] { MagAdminApiAuthenticator.TimestampHeader, MagAdminApiAuthenticator.NonceHeader,
                MagAdminApiAuthenticator.ActorHeader, MagAdminApiAuthenticator.SignatureHeader })
            {
                var missing = api.Handle(Signed("GET", "/admin/v1/accounts", now, omit: header));
                Check(missing.Status == 401 && Error(missing) == "unauthorized", "Missing " + header + " accepted");
            }
            var unauthorized = new[]
            {
                ("short nonce", Signed("GET", "/admin/v1/accounts", now, nonce: "short")),
                ("bad nonce", Signed("GET", "/admin/v1/accounts", now, nonce: "synthetic.nonce.0001")),
                ("long nonce", Signed("GET", "/admin/v1/accounts", now, nonce: new string('n', 65))),
                ("empty actor", Signed("GET", "/admin/v1/accounts", now, actor: "")),
                ("control actor", Signed("GET", "/admin/v1/accounts", now, actor: "staff\u0001")),
                ("long actor", Signed("GET", "/admin/v1/accounts", now, actor: new string('a', 255))),
                ("uppercase signature", Signed("GET", "/admin/v1/accounts", now, tamper: s => s.ToUpperInvariant())),
                ("short signature", Signed("GET", "/admin/v1/accounts", now, tamper: s => s[..63])),
                ("old timestamp", Signed("GET", "/admin/v1/accounts", now, timestamp: new DateTimeOffset(now).ToUnixTimeSeconds() - 61)),
                ("future timestamp", Signed("GET", "/admin/v1/accounts", now, timestamp: new DateTimeOffset(now).ToUnixTimeSeconds() + 61)),
                ("huge timestamp", Signed("GET", "/admin/v1/accounts", now, timestamp: 999999999999)),
                ("other secret", Signed("GET", "/admin/v1/accounts", now, secret: Encoding.UTF8.GetBytes("another-synthetic-secret-0123456789abcdef"))),
                ("flipped signature", Signed("GET", "/admin/v1/accounts", now, tamper: s => (s[0] == '0' ? "1" : "0") + s[1..])),
            };
            foreach (var (name, request) in unauthorized)
            {
                var response = api.Handle(request);
                Check(response.Status == 401 && Error(response) == "unauthorized", "Unauthorized request accepted: " + name);
            }
            Check(api.Handle(Signed("GET", "/admin/v1/accounts", now, timestamp: new DateTimeOffset(now).ToUnixTimeSeconds() - 60)).Status == 200, "60 s skew is allowed");
            var tamperedPath = Signed("GET", "/admin/v1/accounts", now);
            Check(api.Handle(new MagAdminApiRequest { Method = "GET", PathAndQuery = "/admin/v1/accounts?x=1", Headers = tamperedPath.Headers }).Status == 401, "Signature binds the path and query");
            var tamperedBody = Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":2,\"apply\":false}");
            Check(api.Handle(new MagAdminApiRequest { Method = "POST", PathAndQuery = tamperedBody.PathAndQuery, Headers = tamperedBody.Headers,
                Body = Encoding.UTF8.GetBytes("{\"level\":4,\"apply\":false}") }).Status == 401, "Signature binds the body");
            Check(api.Handle(new MagAdminApiRequest { Method = "PUT", PathAndQuery = tamperedBody.PathAndQuery, Headers = tamperedBody.Headers,
                Body = tamperedBody.Body }).Status == 401, "Signature binds the method");
            var once = Signed("GET", "/admin/v1/accounts", now, nonce: "synthetic-replayed-nonce");
            Check(api.Handle(once).Status == 200 && api.Handle(once).Status == 401, "Replayed nonce must be refused");
            now = now.AddMinutes(4);
            Check(api.Handle(Signed("GET", "/admin/v1/accounts", now, nonce: "synthetic-replayed-nonce")).Status == 401, "Nonce remembered for five minutes");
            now = now.AddMinutes(2);
            Check(api.Handle(Signed("GET", "/admin/v1/accounts", now, nonce: "synthetic-replayed-nonce")).Status == 200, "Expired nonce entry is forgotten");
            var reasons = log.Where(l => l.Contains("unauthorized")).ToArray();
            Check(reasons.Length > 0 && reasons.All(l => !l.Contains(Encoding.UTF8.GetString(Secret)) && !l.Contains("synthetic-nonce") && !l.Contains(Actor)),
                "Unauthorized log lines must carry only the reason class");
            Check(reasons.Any(l => l.Contains("(clock-skew)")) && reasons.Any(l => l.Contains("(replay)")) && reasons.Any(l => l.Contains("(bad-signature)")) &&
                reasons.Any(l => l.Contains("(missing-header)")) && reasons.Any(l => l.Contains("(malformed-header)")), "Reason classes are logged");

            // Routing.
            Check(api.Handle(Signed("GET", "/admin/v1/unknown", now)).Status == 404 && Error(api.Handle(Signed("GET", "/", now))) == "not_found", "Unknown path");
            var wrongMethod = api.Handle(Signed("POST", "/admin/v1/accounts", now, "{}"));
            Check(wrongMethod.Status == 405 && wrongMethod.Allow == "GET", "Wrong method on accounts");
            Check(api.Handle(Signed("GET", "/admin/v1/characters/synthetic/level", now)).Status == 405, "Wrong method on set-level");
            Check(api.Handle(Signed("DELETE", "/admin/v1/characters/synthetic", now)).Status == 405, "Wrong method on character");

            // No session marker yet: session is unknown, dry runs work, writes are refused.
            var listed = Json(api.Handle(Signed("GET", "/admin/v1/accounts", now))).GetProperty("accounts");
            Check(listed.GetArrayLength() == 1 && listed[0].GetProperty("account").GetString() == "synthetic" &&
                listed[0].GetProperty("level").GetInt32() == 1 && listed[0].GetProperty("experience").GetInt32() == 0 &&
                listed[0].GetProperty("session").GetString() == "unknown", "Account list without markers");
            int startingPoints = listed[0].GetProperty("skill_points").GetInt32();
            var unknown = api.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":3,\"apply\":true}"));
            Check(unknown.Status == 409 && Error(unknown) == "session_unknown" && Json(unknown).GetProperty("message").GetString().Length > 0, "Write without a marker refused");
            MagActiveAccounts.Write(marker, new Dictionary<string, DateTime>(), now);

            var shown = api.Handle(Signed("GET", "/admin/v1/characters/synthetic", now));
            var character = Json(shown);
            Check(shown.Status == 200 && character.GetProperty("account").GetString() == "synthetic" && character.GetProperty("level").GetInt32() == 1 &&
                character.GetProperty("skill_points").GetInt32() == startingPoints && character.GetProperty("next_level_experience").GetInt32() == 100 &&
                character.GetProperty("session").GetString() == "offline", "Character detail");
            Check(api.Handle(Signed("GET", "/admin/v1/characters/nobody", now)).Status == 404, "Unknown character");
            Check(Error(api.Handle(Signed("GET", "/admin/v1/characters/Bad%20Name", now))) == "not_found", "Invalid account name");
            Check(api.Handle(Signed("POST", "/admin/v1/characters/nobody/level", now, "{\"level\":2,\"apply\":true}")).Status == 404, "Unknown account set-level");

            foreach (var body in new[] { "", "nope", "[]", "{\"level\":3}", "{\"apply\":true}", "{\"level\":\"3\",\"apply\":true}",
                "{\"level\":3.5,\"apply\":true}", "{\"level\":3,\"apply\":\"yes\"}", "{\"level\":1e40,\"apply\":true}" })
            {
                var bad = api.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, body));
                Check(bad.Status == 400 && Error(bad) == "invalid_request", "Invalid body accepted: " + body);
            }

            // Dry run: the planned change, no write and no backup.
            byte[] original = File.ReadAllBytes(storePath);
            var dry = api.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":3,\"apply\":false}"));
            var plan = Json(dry);
            Check(dry.Status == 200 && !plan.GetProperty("applied").GetBoolean() && plan.GetProperty("account").GetString() == "synthetic" &&
                plan.GetProperty("from_level").GetInt32() == 1 && plan.GetProperty("to_level").GetInt32() == 3 &&
                plan.GetProperty("experience_before").GetInt32() == 0 && plan.GetProperty("experience_after").GetInt32() == 250 &&
                plan.GetProperty("skill_points_before").GetInt32() == startingPoints && plan.GetProperty("skill_points_after").GetInt32() == startingPoints + 1 + 2 &&
                plan.GetProperty("backup").ValueKind == JsonValueKind.Null, "Dry run plan");
            Check(File.ReadAllBytes(storePath).SequenceEqual(original) && !Directory.Exists(backups), "Dry run must not write or back up");

            // A live lease refuses the write but still allows the dry run.
            MagActiveAccounts.Write(marker, new Dictionary<string, DateTime> { ["synthetic"] = now.AddSeconds(60) }, now);
            Check(Json(api.Handle(Signed("GET", "/admin/v1/accounts", now))).GetProperty("accounts")[0].GetProperty("session").GetString() == "online", "Online session listed");
            var online = api.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":3,\"apply\":true}"));
            Check(online.Status == 409 && Error(online) == "online" && File.ReadAllBytes(storePath).SequenceEqual(original), "Online write refused");
            Check(api.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":3,\"apply\":false}")).Status == 200, "Online dry run allowed");
            MagActiveAccounts.Write(marker, new Dictionary<string, DateTime> { ["synthetic"] = now.AddSeconds(-1) }, now);

            // Apply: the same store rules as mag-admin, with the exact previous store backed up.
            log.Clear();
            var applied = api.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":3,\"apply\":true}"));
            var result = Json(applied);
            string backupName = result.GetProperty("backup").GetString();
            Check(applied.Status == 200 && result.GetProperty("applied").GetBoolean() && result.GetProperty("to_level").GetInt32() == 3 &&
                backupName.StartsWith("characters.before-admin-set-level-synthetic-3-") && !backupName.Contains('/'), "Applied raise");
            Check(File.ReadAllBytes(Path.Combine(backups, backupName)).SequenceEqual(original), "Backup must hold the exact previous store");
            var after = new LocalCharacterStore(storePath) { ProgressionCatalog = progression }.ReadAll().Single().ToRecord();
            Check(after.Level == 3 && after.Experience == 250 && after.SkillPoints == startingPoints + 3, "Raise must match earned progression");
            Check(log.Any(l => l.Contains("applied set-level") && l.Contains("actor=\"" + Actor + "\"") && l.Contains("account=synthetic") &&
                l.Contains("from=1") && l.Contains("to=3") && l.Contains("backup=" + backupName)), "Applied change must be logged with actor and backup");

            MagAdminApiResponse Level(int level, bool apply = true) =>
                api.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, $"{{\"level\":{level},\"apply\":{(apply ? "true" : "false")}}}"));
            Check(Level(3).Status == 409 && Error(Level(3)) == "same_level" && Error(Level(3, false)) == "same_level", "Same level refused");
            Check(Error(Level(2)) == "lower_level" && Error(Level(2, false)) == "lower_level", "Lowering refused");
            foreach (int level in new[] { 0, -1, 5, 256, int.MaxValue })
                Check(Level(level).Status == 409 && Error(Level(level)) == "level_out_of_range", "Out-of-range level accepted: " + level);
            Check(Json(api.Handle(Signed("GET", "/admin/v1/characters/synthetic", now))).GetProperty("level").GetInt32() == 3, "Detail shows the new level");
            Check(Level(4).Status == 200 &&
                Json(api.Handle(Signed("GET", "/admin/v1/characters/synthetic", now))).GetProperty("next_level_experience").ValueKind == JsonValueKind.Null,
                "Maximum level has no next threshold");

            // Without a progression catalog nothing can be raised.
            var bare = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), Secret, new LocalCharacterStore(storePath), backups, markers, () => now, null);
            var noCatalog = bare.Handle(Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":4,\"apply\":true}"));
            Check(noCatalog.Status == 409 && Error(noCatalog) == "no_progression_catalog", "No catalog refused");

            Wire(api, now);
        }
        finally { Directory.Delete(directory, true); }
    }

    // A real loopback round trip: HTTP framing, headers, limits and Connection: close.
    static void Wire(MagAdminApi api, DateTime now)
    {
        api.Start();
        try
        {
            var endpoint = api.LocalEndPoint;
            using var http = new HttpClient { BaseAddress = new Uri($"http://127.0.0.1:{endpoint.Port}") };
            HttpResponseMessage Send(MagAdminApiRequest signed)
            {
                var message = new HttpRequestMessage(new HttpMethod(signed.Method), signed.PathAndQuery);
                foreach (var (name, value) in signed.Headers) message.Headers.TryAddWithoutValidation(name, value);
                if (signed.Body.Length > 0) message.Content = new ByteArrayContent(signed.Body);
                return http.Send(message);
            }
            var listed = Send(Signed("GET", "/admin/v1/accounts", now));
            Check(listed.StatusCode == HttpStatusCode.OK && listed.Content.Headers.ContentType?.MediaType == "application/json",
                "Wire GET " + listed.StatusCode);
            Check(JsonDocument.Parse(listed.Content.ReadAsStream()).RootElement.GetProperty("accounts")[0].GetProperty("level").GetInt32() == 4, "Wire body");
            var dry = Send(Signed("POST", "/admin/v1/characters/synthetic/level", now, "{\"level\":4,\"apply\":false}"));
            Check(dry.StatusCode == HttpStatusCode.Conflict, "Wire POST " + dry.StatusCode);
            Check(Send(Signed("GET", "/admin/v1/accounts", now, omit: MagAdminApiAuthenticator.SignatureHeader)).StatusCode == HttpStatusCode.Unauthorized, "Wire 401");

            string Raw(string head)
            {
                using var client = new TcpClient();
                client.Connect(IPAddress.Loopback, endpoint.Port);
                var stream = client.GetStream();
                stream.Write(Encoding.ASCII.GetBytes(head));
                using var reader = new StreamReader(stream, Encoding.ASCII);
                return reader.ReadToEnd();
            }
            string large = Raw($"POST /admin/v1/characters/synthetic/level HTTP/1.1\r\nHost: admin\r\nContent-Length: {MagAdminApi.MaximumBodyBytes + 1}\r\n\r\n");
            Check(large.StartsWith("HTTP/1.1 413 ") && large.Contains("Connection: close"), "Oversized body must be refused");
            Check(Raw("GET /admin/v1/accounts HTTP/1.1\r\nHost: admin\r\nTransfer-Encoding: chunked\r\n\r\n").StartsWith("HTTP/1.1 400 "), "Chunked bodies are refused");
            Check(Raw("GET admin HTTP/1.1\r\n\r\n").StartsWith("HTTP/1.1 400 "), "Malformed request line refused");
        }
        finally { api.DisposeAsync().AsTask().GetAwaiter().GetResult(); }
    }

    /// <summary>The signed status endpoint shows DME ports in use against the cap.</summary>
    public static void DmePorts()
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-admin-ports-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var now = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"));
            var budget = new MagDmePortBudget(10505, null);
            var held = budget.TryAcquire(5);
            var api = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), Secret, store, directory, directory, () => now, _ => { }, budget);
            var response = api.Handle(Signed("GET", "/admin/v1/dme-ports", now));
            Check(response.Status == 200, "DME port status refused");
            var body = Json(response);
            Check(body.GetProperty("capacity").GetInt32() == 32 && body.GetProperty("in_use").GetInt32() == 5 &&
                body.GetProperty("available").GetInt32() == 27 && body.GetProperty("first_port").GetInt32() == 10474 &&
                body.GetProperty("last_port").GetInt32() == 10505 && body.GetProperty("source").GetString() == "port_range" &&
                body.TryGetProperty("udp_ports_bound", out _), "DME port status lost ports in use against the cap");
            Check(api.Handle(Signed("POST", "/admin/v1/dme-ports", now, "{}")).Status == 405, "DME port status accepted a write");
            Check(api.Handle(Signed("GET", "/admin/v1/dme-ports", now, tamper: s => s[..63])).Status == 401, "DME port status skipped authentication");
            var open = new MagAdminApi(new IPEndPoint(IPAddress.Loopback, 0), Secret, store, directory, directory, () => now, _ => { });
            var unbounded = Json(open.Handle(Signed("GET", "/admin/v1/dme-ports", now)));
            Check(unbounded.GetProperty("capacity").ValueKind == JsonValueKind.Null && unbounded.GetProperty("source").GetString() == "unbounded",
                "Unbounded DME ports reported a cap");
            foreach (var lease in held) lease.Dispose();
        }
        finally { Directory.Delete(directory, true); }
    }
}
