using Newtonsoft.Json;
using Org.BouncyCastle.Math;
using RT.Cryptography;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Common;
using Server.Medius;

// All values below are synthetic examples, constructed in memory. No game,
// account, packet capture, catalog export or live service is used by these tests.
int passed = 0;
void Check(bool condition, string message)
{
    if (!condition) throw new Exception(message);
}
void Throws<T>(Action action) where T : Exception
{
    try { action(); }
    catch (T) { return; }
    throw new Exception("Expected " + typeof(T).Name);
}
void Test(string name, Action action)
{
    action();
    Console.WriteLine("PASS " + name);
    passed++;
}

Test("startup team minima share bridge policy and advertised total", () =>
{
    var policy = new MagMatchStartupPolicy(1, 8, new[] { 2, 1 });
    Check(policy.MinimumPlayers == 3, "Status must not advertise fewer players than team minima");
    using var request = System.Text.Json.JsonDocument.Parse(System.Text.Json.JsonSerializer.Serialize(policy.ConfigureRequest("synthetic", 20231)));
    var root = request.RootElement;
    Check(root.GetProperty("minimum_players").GetInt32() == 3 && root.GetProperty("maximum_players").GetInt32() == 8, "Bridge aggregate differs from status");
    Check(root.GetProperty("minimum_players_per_team").EnumerateArray().Select(x => x.GetInt32()).SequenceEqual(new[] {2,1}), "Bridge team minima");
    Check(new MagMatchStartupPolicy(5,8,null).MinimumPlayers == 5, "Legacy total minimum");
    Check(new MagMatchStartupPolicy(1,8,null).MinimumPlayers == 1, "Solo default");
    var supplied = new[] {1,1};
    var copied = new MagMatchStartupPolicy(1,8,supplied); supplied[0]=8;
    Check(copied.MinimumPlayers == 2, "Policy changed with caller array");
    foreach (var teams in new[] {new[] {1}, new[] {1,1,1}, new[] {-1,1}, new[] {5,4}, new[] {int.MaxValue,1}})
        Throws<ArgumentException>(() => new MagMatchStartupPolicy(1,8,teams));
    Throws<ArgumentException>(() => new MagMatchStartupPolicy(0,8,null));
    Throws<ArgumentException>(() => new MagMatchStartupPolicy(9,8,null));
});

Test("signed skill fields and strict request length", () =>
{
    foreach (sbyte id in new sbyte[] { -1, 0, 1, 126, 127 })
    {
        var bytes = MagMessageCodec.Serialize(new MagPurchaseSkillRequest { SkillId = id });
        // Independent field check: two big-endian type bytes, then signed ID.
        Check(bytes.Length == 3 && bytes[0] == 2 && bytes[1] == 80 && bytes[2] == unchecked((byte)id), "Skill wire layout");
        Check(MagMessageCodec.TryParseClient(bytes, out var parsed) && parsed is MagPurchaseSkillRequest request && request.SkillId == id, "Signed ID");
        Check(!MagMessageCodec.TryParseClient(bytes[..^1], out _), "Truncation accepted");
        Check(!MagMessageCodec.TryParseClient(bytes.Concat(new byte[1]).ToArray(), out _), "Trailing byte accepted");
    }
});

Test("application-specific item widths and malformed loadouts", () =>
{
    foreach (int appId in new[] { 20230, 20231 })
    {
        var kit = new MagLoadout { UpdateId = 2 };
        short maximum = (short)(appId == 20230 ? 1023 : 4095);
        kit.Items[0] = 1;
        kit.Items[21] = maximum;
        var bytes = MagMessageCodec.SerializeForApplication(new MagLoadoutUpdateRequest { LoadoutIndex = 4, Loadout = kit }, appId);
        Check(MagMessageCodec.TryParseClientForApplication(bytes, appId, out var parsed) && parsed is MagLoadoutUpdateRequest request &&
            request.LoadoutIndex == 4 && request.Loadout.UpdateId == 2 && request.Loadout.Items.SequenceEqual(kit.Items), "Loadout fields");
        Check(!MagMessageCodec.TryParseClientForApplication(bytes[..^1], appId, out _), "Truncated loadout");
        var badIndex = (byte[])bytes.Clone(); badIndex[2] = 5;
        Check(!MagMessageCodec.TryParseClientForApplication(badIndex, appId, out _), "Invalid loadout index");
        kit.Items[0] = (short)(maximum + 1);
        Throws<ArgumentException>(() => MagMessageCodec.SerializeForApplication(new MagLoadoutUpdateRequest { Loadout = kit }, appId));
    }
    Throws<ArgumentException>(() => MagMessageCodec.Serialize(new MagLoadoutUpdateRequest { Loadout = new MagLoadout { Items = new short[21] } }));
});

Test("response direction, unaligned boolean and zero padding", () =>
{
    foreach (bool success in new[] { false, true })
    {
        var bytes = MagMessageCodec.Serialize(new MagLoadoutUpdateResponse { Success = success });
        Check(bytes.Length == 3 && bytes[0] == 2 && bytes[1] == 32 && bytes[2] == (success ? 128 : 0), "Response bit layout");
        Check(MagMessageCodec.TryParse(bytes, out var parsed) && parsed is MagLoadoutUpdateResponse response && response.Success == success, "Response bit");
        Check(!MagMessageCodec.TryParseClient(bytes, out _), "Server response admitted as request");
        bytes[^1] |= 1;
        Check(!MagMessageCodec.TryParse(bytes, out _), "Nonzero padding accepted");
    }
});

Test("full synthetic record preserves fields in each profile", () =>
{
    foreach (int appId in new[] { 20230, 20231 })
    {
        var record = new MagCharacterRecord { Name = "Example", AccountId = 7, Faction = 1, SkillPoints = 9, VeteranLevel = 2, Flags = 1 };
        record.AvailableItems[23] = 3;
        record.Skills[4] = 5;
        record.NpId[35] = 1;
        var bytes = MagMessageCodec.SerializeForApplication(new MagPurchaseSkillResponse { Success = true, Record = record }, appId);
        Check(MagMessageCodec.TryParseForApplication(bytes, appId, out var parsed) && parsed is MagPurchaseSkillResponse, "Record parse");
        var actual = ((MagPurchaseSkillResponse)parsed).Record;
        Check(actual.Name == "Example" && actual.AccountId == 7 && actual.SkillPoints == 9 && actual.AvailableItems[23] == 3 && actual.Skills[4] == 5 && actual.NpId[35] == 1, "Record fields");
        Check(appId != 20231 || (actual.VeteranLevel == 2 && actual.Flags == 1), "Retail tail");
        Check(!MagMessageCodec.TryParseForApplication(bytes[..^1], appId, out _), "Truncated record");
    }
});

Test("external synthetic catalog validates assignments and clones defaults", () =>
{
    var directory = Path.Combine(Path.GetTempPath(), "mag-synthetic-" + Guid.NewGuid());
    Directory.CreateDirectory(directory);
    try
    {
        string itemsPath = Path.Combine(directory, "items.json"), defaultsPath = Path.Combine(directory, "defaults.json");
        File.WriteAllText(itemsPath, """
            {"source":"synthetic example; not game data","schema_version":1,"application_id":20230,
             "definitions":{"weapons":{"Example":{"parameters":{"magazine_capacity":1}}}},
             "items":{"1":{"item_id":1,"name":"Example","item_type":"kWeapon","definition":{"group":"weapons","name":"Example"}}}}
            """);
        var slots = Enumerable.Repeat((short)-1, 22).ToArray(); slots[0] = 1;
        File.WriteAllText(defaultsPath, System.Text.Json.JsonSerializer.Serialize(new {
            source = "synthetic example; not game data", application_id = 20230,
            factions = new[] { new { faction = 0, loadouts = new[] { new { update_id = 2, items = slots } } } }
        }));
        var catalog = MagLoadoutCatalog.Load(itemsPath, defaultsPath);
        var defaults = catalog.DefaultsForFaction(0);
        Check(defaults.Length == 5 && catalog.HasValidPrimary(defaults[0]), "External catalog");
        defaults[0].Items[0] = 2;
        Check(!catalog.Valid(defaults[0]), "Unknown assignment admitted");
        Check(catalog.DefaultsForFaction(0)[0].Items[0] == 1, "Catalog state exposed");
        Throws<FileNotFoundException>(() => MagLoadoutCatalog.Load(Path.Combine(directory, "missing.json"), defaultsPath));
    }
    finally { Directory.Delete(directory, true); }
});

Test("development RSA key consistency and unchanged external JSON schema", () =>
{
    var key = DevelopmentRsaKeys.ForProcess;
    Check(ReferenceEquals(key, new Server.Medius.Config.ServerSettings().DefaultKey), "Per-process key changed");
    Check(key.N.BitLength == 512 && key.E.Equals(BigInteger.ValueOf(17)), "Legacy modulus/exponent width");
    var value = BigInteger.ValueOf(42);
    Check(value.ModPow(key.E, key.N).ModPow(key.D, key.N).Equals(value), "RSA pair inconsistent");
    var json = JsonConvert.SerializeObject(key);
    var copy = JsonConvert.DeserializeObject<RsaKeyPair>(json);
    Check(copy.N.Equals(key.N) && copy.E.Equals(key.E) && copy.D.Equals(key.D), "External n/e/d schema");
});

Test("ticket-login credential seeds are not shared", () =>
{
    var first = new MediusTicketLoginRequest();
    var second = new MediusTicketLoginRequest();
    Check(first.Password.Length == 32 && second.Password.Length == 32 && first.Password != second.Password, "Shared credential seed");
});

Test("PS3 hello requires external templates and never modifies caller bytes", () =>
{
    var names = new[] { "HORIZON_SCERT_MAS_CERTIFICATE_PATH", "HORIZON_SCERT_MLS_CERTIFICATE_PATH" };
    var previous = names.Select(Environment.GetEnvironmentVariable).ToArray();
    try
    {
        foreach (var name in names) Environment.SetEnvironmentVariable(name, null);
        using var output = new MemoryStream();
        using var writer = new MessageWriter(output) { MediusVersion = 112 };
        Throws<InvalidOperationException>(() => new RT_MSG_SERVER_HELLO().Serialize(writer));
        Throws<InvalidOperationException>(() => new RT_MSG_SERVER_HELLO { MLS = true }.Serialize(writer));
        Throws<InvalidDataException>(() => new RT_MSG_SERVER_HELLO { Certificate = new byte[1] }.Serialize(writer));
        var synthetic = new byte[0x1d4];
        var hello = new RT_MSG_SERVER_HELLO { Certificate = synthetic, RsaPublicKey = BigInteger.ValueOf(42) };
        output.SetLength(0); output.Position = 0;
        hello.Serialize(writer);
        Check(output.Length == synthetic.Length && output.ToArray()[0x194] == 42, "MAS modulus placement");
        Check(synthetic.All(value => value == 0), "Template was modified");
    }
    finally { for (int i = 0; i < names.Length; i++) Environment.SetEnvironmentVariable(names[i], previous[i]); }
});

Test("aggregate grouping retains strict boundary and singleton behavior", () =>
{
    var groups = new[] { 2, 3, 1, 9, 1 }.GroupWhileAggregating(0, (sum, item) => sum + item, (sum, item) => sum < 6)
        .Select(group => group.ToArray()).ToArray();
    Check(groups.Length == 4 && groups[0].SequenceEqual(new[] { 2, 3 }) && groups[1].SequenceEqual(new[] { 1 }) &&
        groups[2].SequenceEqual(new[] { 9 }) && groups[3].SequenceEqual(new[] { 1 }), "Grouping boundary");
    Check(!Array.Empty<int>().GroupWhileAggregating(0, (sum, item) => sum + item, (sum, item) => true).Any(), "Empty grouping");
    int predicates = 0;
    var singleton = new[] { 99 }.GroupWhileAggregating(5, (sum, item) => sum + item, (sum, item) => { predicates++; return false; }).Single();
    Check(singleton.Single() == 99 && predicates == 0, "First item must not be rejected");
});

Test("BCL minimum selection keeps comparer, first tie and empty rejection", () =>
{
    Check(Utils.MinBy(new[] { "aa", "b", "c" }, value => value.Length, Comparer<int>.Default) == "b", "First minimum tie");
    Check(Utils.MinBy(new[] { 1, 3, 2 }, value => value, Comparer<int>.Create((a, b) => b.CompareTo(a))) == 3, "Custom comparer");
    Throws<InvalidOperationException>(() => Utils.MinBy(Array.Empty<string>(), value => value, null));
    Throws<ArgumentNullException>(() => Utils.MinBy<string, int>(null, value => value.Length, null));
});

Test("BCL timeout wrappers propagate completion, failure and cancellation", () =>
{
    Check(Task.FromResult(7).TimeoutAfter(TimeSpan.Zero).GetAwaiter().GetResult() == 7, "Completed result");
    Task.CompletedTask.TimeoutAfter(TimeSpan.Zero).GetAwaiter().GetResult();
    Throws<InvalidOperationException>(() => Task.FromException<int>(new InvalidOperationException()).TimeoutAfter(TimeSpan.FromSeconds(1)).GetAwaiter().GetResult());
    Throws<TaskCanceledException>(() => Task.FromCanceled(new CancellationToken(true)).TimeoutAfter(TimeSpan.FromSeconds(1)).GetAwaiter().GetResult());
    var pending = new TaskCompletionSource<int>();
    Throws<TimeoutException>(() => pending.Task.TimeoutAfter(TimeSpan.Zero).GetAwaiter().GetResult());
    Check(!pending.Task.IsCompleted, "Timeout canceled underlying work");
    Check(!pending.Task.TryAwait(TimeSpan.Zero).GetAwaiter().GetResult(), "Pending TryAwait");
    Check(Task.CompletedTask.TryAwait(TimeSpan.Zero).GetAwaiter().GetResult(), "Completed TryAwait");
    Throws<TimeoutException>(() => Task.FromException(new TimeoutException()).TryAwait(TimeSpan.FromSeconds(1)).GetAwaiter().GetResult());
});

Test("public address is explicit external configuration without network discovery", () =>
{
    const string variable = "HORIZON_PUBLIC_IP";
    var previous = Environment.GetEnvironmentVariable(variable);
    try
    {
        Environment.SetEnvironmentVariable(variable, null);
        Throws<InvalidOperationException>(() => Utils.GetPublicIPAddress());
        Environment.SetEnvironmentVariable(variable, "invalid");
        Throws<InvalidOperationException>(() => Utils.GetPublicIPAddress());
        Environment.SetEnvironmentVariable(variable, "203.0.113.7");
        Check(Utils.GetPublicIPAddress() == "203.0.113.7", "Configured public IPv4");
    }
    finally { Environment.SetEnvironmentVariable(variable, previous); }
});

Test("advertised endpoints validate independently from listener ports", () =>
{
    var defaults = MagNetworkSettings.Load(_ => null);
    Check(defaults.Address.ToString() == "127.0.0.1" && defaults.AuthPort == 10475 &&
        defaults.LobbyPort == 10478 && defaults.GameTcpPort == 10473, "Legacy defaults");
    var values = new Dictionary<string, string> { ["MAG_ADVERTISE_IPV4"] = "203.0.113.10",
        ["MAG_AUTH_DNS"] = "auth.example.invalid", ["MAG_AUTH_PORT"] = "11475",
        ["MAG_LOBBY_PORT"] = "11478", ["MAG_GAME_TCP_PORT"] = "11473" };
    string Read(string key) => values.GetValueOrDefault(key);
    var configured = MagNetworkSettings.Load(Read);
    Check(configured.Address.ToString() == "203.0.113.10" && configured.AuthDns == "auth.example.invalid" &&
        configured.AuthPort == 11475 && configured.LobbyPort == 11478 && configured.GameTcpPort == 11473, "External endpoints");
    var previous = values.Keys.ToDictionary(key => key, Environment.GetEnvironmentVariable);
    try
    {
        foreach (var pair in values) Environment.SetEnvironmentVariable(pair.Key, pair.Value);
        var response = new MagLoginRegistry().Issue(1, 1, DateTime.UtcNow).CreateResponse(1);
        Check(System.Text.Encoding.ASCII.GetString(response.ServerAddress).TrimEnd('\0') == "203.0.113.10" &&
            response.ServerPort == 11478, "Login response uses advertised endpoint");
    }
    finally
    { foreach (var pair in previous) Environment.SetEnvironmentVariable(pair.Key, pair.Value); }
    foreach (string bad in new[] { "0.0.0.0", "127.1", "::1", "256.1.2.3", "224.1.2.3", " 203.0.113.10" })
    { values["MAG_ADVERTISE_IPV4"] = bad; Throws<ArgumentException>(() => MagNetworkSettings.Load(Read)); }
    values["MAG_ADVERTISE_IPV4"] = "203.0.113.10";
    foreach (string bad in new[] { "0", "65536", "-1", " 10475" })
    { values["MAG_AUTH_PORT"] = bad; Throws<ArgumentException>(() => MagNetworkSettings.Load(Read)); }
    values["MAG_AUTH_PORT"] = "10475";
    foreach (string bad in new[] { "", "https://auth.example.invalid/", "auth.example.invalid/path" })
    { values["MAG_AUTH_DNS"] = bad; Throws<ArgumentException>(() => MagNetworkSettings.Load(Read)); }
});

Console.WriteLine($"{passed} synthetic checks passed; no service or game started.");
