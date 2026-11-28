using Server.Medius;
using RT.Models.MAG;

internal static class PortalAccountLookupChecks
{
    static void Check(bool yes, string message) { if (!yes) throw new Exception(message); }
    public static void Cache() => CacheAsync().GetAwaiter().GetResult();
    static async Task CacheAsync()
    {
        DateTime now = DateTime.UtcNow;
        IReadOnlyDictionary<uint, string> rows = new Dictionary<uint, string> { [17] = "original", [18] = "service_owner" };
        bool fail = false;
        using var cache = new MagPortalAccountCache(_ => fail ? Task.FromException<IReadOnlyDictionary<uint, string>>(new IOException("synthetic outage")) : Task.FromResult(rows), () => now);
        Check(!cache.TryResolve(17, out _), "Uninitialized cache granted an owner");
        await cache.RefreshAsync();
        Check(cache.TryResolve(17, out var owner) && owner == "original", "Stable ID lookup");
        Check(cache.TryResolve(18, out owner) && owner == "service_owner", "Service account lost");
        Check(!cache.TryResolve(19, out _), "Unknown ID was invented");
        now = now.AddSeconds(5);
        Check(!cache.TryResolve(17, out _), "Stale snapshot accepted");
        await cache.RefreshAsync();
        fail = true;
        try { await cache.RefreshAsync(); } catch (InvalidOperationException) { }
        Check(!cache.TryResolve(17, out _), "Failed refresh retained admission");
        fail = false;
        rows = new Dictionary<uint, string> { [18] = "service_owner" };
        await cache.RefreshAsync();
        Check(!cache.TryResolve(17, out _) && cache.TryResolve(18, out _), "Deletion/ban refresh not atomic");
        rows = new Dictionary<uint, string> { [17] = "invalid-owner!" };
        try { await cache.RefreshAsync(); throw new Exception("Invalid owner accepted"); } catch (InvalidOperationException) { }
        Check(!cache.TryResolve(18, out _), "Invalid refresh left partial snapshot");
        rows = Enumerable.Range(1, 100001).ToDictionary(i => (uint)i, i => "owner" + i);
        try { await cache.RefreshAsync(); throw new Exception("Oversized snapshot accepted"); } catch (InvalidOperationException) { }
        rows = new Dictionary<uint, string> { [17] = "original", [18] = "original" };
        try { await cache.RefreshAsync(); throw new Exception("Duplicate owner accepted"); } catch (InvalidOperationException) { }
    }
    public static void PublicCards()
    {
        var clock = DateTime.UtcNow;
        var registry = new MagLoginRegistry();
        var first = registry.IssueForAccount("oldowner", 41, 1, clock, "NewPublic");
        var second = registry.IssueForAccount("oldowner", 42, 1, clock, "NewPublic");
        Check(registry.TryBind(second.AccessToken, second.SessionKey, second.ApplicationId, 1, "other-player", clock, out _), "Other live credential setup");
        var rows = new[] {
            new StoredCharacter { Account = "oldowner", Id = 41, Slot = 0, Name = "OldPublic!0", NpId = new byte[36] },
            new StoredCharacter { Account = "oldowner", Id = 42, Slot = 1, Name = "OldPublic!1", NpId = new byte[36] },
            new StoredCharacter { Account = "unrelated", Id = 43, Slot = 0, Name = "NewPublic", NpId = new byte[36] }
        };
        var card = MagMlsSession.FindCharacter(rows, new MagCharacterDataRequest { QueryKind = 1, AccountName = "NewPublic" },
            id => registry.IsAccountOnline(id, clock), owner => registry.PublicNameForOwner(owner, clock));
        Check(card?.Id == 42 && card.Account == "oldowner", "New public name did not find another active player's selected character");
        var record = MagPublicIdentity.DisplayRecord(card, registry.PublicNameForOwner(card.Account, clock));
        Check(StoredCharacter.DisplayNameFor(record.Name, record.NpId) == "NewPublic" && card.NpId.All(b => b == 0) && card.Name == "OldPublic!1", "Card display modified persisted identity");
        var foreign = registry.IssueForAccount("unrelated", 43, 1, clock, "NewPublic");
        Check(registry.TryBind(foreign.AccessToken, foreign.SessionKey, foreign.ApplicationId, 1, "ambiguous-player", clock, out _), "Ambiguous credential setup");
        Check(MagMlsSession.FindCharacter(rows, new MagCharacterDataRequest { QueryKind = 1, AccountName = "NewPublic" },
            id => true, owner => registry.PublicNameForOwner(owner, clock)) == null, "Ambiguous authenticated public name selected another owner");
        Check(registry.IssueForAccount("oldowner", 44, 1, clock, "bad\nname") == null, "Unsafe public identity reached credentials");
    }
    public static void Configuration()
    {
        Check(MagPortalAccountLookupSettings.Load(_ => null, MagPortalTicketMode.Off) == null, "Off default changed");
        Check(MagPortalAccountLookupSettings.Load(_ => null, MagPortalTicketMode.Required) == null, "Required legacy default changed");
        string Read(string name) => name == "MAG_PORTAL_ACCOUNT_LOOKUP" ? "required" : "/synthetic/secret";
        Check(MagPortalAccountLookupSettings.Load(Read, MagPortalTicketMode.Required).ConnectionFile == "/synthetic/secret", "Required setting");
        foreach (var mode in new[] { "required", "junk" })
        {
            try { MagPortalAccountLookupSettings.Load(n => n == "MAG_PORTAL_ACCOUNT_LOOKUP" ? mode : null, MagPortalTicketMode.Off); throw new Exception("Unsafe lookup configuration accepted"); }
            catch (ArgumentException) { }
        }
        try { MagPortalAccountLookupSettings.Load(n => n == "MAG_PORTAL_ACCOUNT_LOOKUP" ? "required" : null, MagPortalTicketMode.Required); throw new Exception("Missing secret file accepted"); } catch (ArgumentException) { }
        try { MagPortalAccountCache.FromConnectionFile("/not-present/synthetic-secret"); throw new Exception("Missing file accepted"); }
        catch (InvalidOperationException e) { Check(!e.ToString().Contains("/not-present"), "Secret path leaked"); }
    }
    public static void PublicNameCreation()
    {
        string dir = Path.Combine(Path.GetTempPath(), "mag-names-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        try
        {
            var store = new LocalCharacterStore(Path.Combine(dir, "characters.json"));
            var np = new byte[36]; System.Text.Encoding.ASCII.GetBytes("NewPublic").CopyTo(np, 0);
            var request = new MagCreateCharacterRequest { AccountName = "NewPublic!1", NpId = np, Faction = 1, HeadId = 2, VoiceId = 3, UniverseId = 26 };
            Check(store.CreateForAccount(request, "oldowner").Reason == "np-id-account-mismatch", "Legacy validation changed");
            Check(store.CreateForAccount(request, "oldowner", "NewPublic").Result == MagNetResult.Success, "New public NP ID rejected for stable owner");
            Check(store.ReadForAccount("newpublic").Count == 0 && store.ReadForAccount("oldowner").Single().Name == "NewPublic!1", "Public name changed owner");
            Check(StoredCharacter.DisplayNameFor("", store.ReadForAccount("oldowner").Single().NpId) == "oldowner" &&
                System.Text.Encoding.ASCII.GetString(np, 0, 9) == "NewPublic", "Canonical NP normalization changed request or persisted unbound identity");
            Check(store.CreateForAccount(request, "oldowner", "NewPublic").Reason == "existing-exact-profile", "Canonical NP exact retry changed");
            request.AccountName = "Other!1";
            Check(store.CreateForAccount(request, "oldowner", "OtherPublic").Reason == "np-id-account-mismatch", "Unrelated NP ID accepted");
            Check(store.CreateForAccount(request, "oldowner", "bad\nname").Result != MagNetResult.Success, "Invalid authenticated public name accepted");
        }
        finally { Directory.Delete(dir, true); }
    }
    public static void AtomicRefresh() => AtomicRefreshAsync().GetAwaiter().GetResult();
    static async Task AtomicRefreshAsync()
    {
        var next = new TaskCompletionSource<IReadOnlyDictionary<uint, string>>(TaskCreationOptions.RunContinuationsAsynchronously);
        bool deferred = false;
        using var cache = new MagPortalAccountCache(_ => deferred ? next.Task : Task.FromResult<IReadOnlyDictionary<uint, string>>(new Dictionary<uint, string> { [17] = "oldowner" }));
        await cache.RefreshAsync(); deferred = true;
        var refresh = cache.RefreshAsync();
        Check(cache.TryResolve(17, out var owner) && owner == "oldowner" && !cache.TryResolve(18, out _), "Incomplete refresh exposed partial data or blocked reader");
        next.SetResult(new Dictionary<uint, string> { [18] = "newowner" });
        await refresh;
        Check(!cache.TryResolve(17, out _) && cache.TryResolve(18, out owner) && owner == "newowner", "Atomic replacement kept removed rows");
    }

}
