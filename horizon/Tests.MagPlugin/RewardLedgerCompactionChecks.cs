using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// Synthetic ledgers only: rows are cloned from a receipt the store itself committed, then padded
// with an extension field so a few rows reach the store's byte bounds.
internal static partial class LeadershipProgressionChecks
{
    private const string OpenTemplate = "template-open";

    private sealed class LedgerFixture
    {
        public string Path = "";
        public MagProgressionCatalog Catalog = null!;
        public JsonObject Document = null!;
        public JsonObject Receipt = null!;
        public JsonObject Progress = null!;
        public readonly List<string> Messages = new();
        // A fixed clock keeps row ages deterministic; checks move it explicitly.
        public DateTimeOffset Now = new DateTimeOffset(2026, 11, 25, 12, 0, 0, TimeSpan.Zero);
        public TimeSpan Ttl = LocalCharacterStore.DefaultOpenMatchRowTtl;
        public LocalCharacterStore Store() => new LocalCharacterStore(Path)
        { ProgressionCatalog = Catalog, LedgerMaintenance = Messages.Add, Clock = () => Now, OpenMatchRowTtl = Ttl };
        public JsonObject Receipts => Document["rewardReceipts"]!.AsObject();
        public JsonObject Rows => Document["matchRewardProgress"]!.AsObject();
        public static JsonObject Read(string path) => JsonNode.Parse(File.ReadAllText(path))!.AsObject();
        public void Write() => File.WriteAllText(Path, Document.ToJsonString(new System.Text.Json.JsonSerializerOptions { WriteIndented = true }));
    }

    private static LedgerFixture SeedLedger(string directory)
    {
        var fixture = new LedgerFixture { Path = System.IO.Path.Combine(directory, "characters.json"), Catalog = Load(directory, Catalog()) };
        var store = fixture.Store();
        Create(store, "alpha", "Alpha");
        Create(store, "bravo", "Bravo");
        Check(store.ApplyRewardForAccount("alpha", 1, "template-action", OpenTemplate, "enemy_killed", 1).Result == MagNetResult.Success,
            "Template reward was not committed");
        fixture.Document = LedgerFixture.Read(fixture.Path);
        fixture.Receipt = fixture.Receipts["template-action"]!.AsObject();
        fixture.Progress = fixture.Rows["alpha:1:" + OpenTemplate]!.AsObject();
        return fixture;
    }

    // Adds one match row for alpha with `receipts` action receipts; PadTo later sizes their padding.
    // `rewardedAt` dates the row's last reward; `undated` writes a row from before rows were dated.
    private static void AddLedgerRow(LedgerFixture fixture, string match, bool finalized, int receipts,
        DateTimeOffset? rewardedAt = null, bool undated = false)
    {
        var row = fixture.Progress.DeepClone().AsObject();
        row["matchKey"] = match;
        row["finalized"] = finalized;
        if (rewardedAt is DateTimeOffset at) row["lastRewardAt"] = at.ToUnixTimeSeconds();
        if (undated) row.Remove("lastRewardAt");
        fixture.Rows["alpha:1:" + match] = row;
        for (int i = 0; i < receipts; i++)
        {
            var receipt = fixture.Receipt.DeepClone().AsObject();
            receipt["matchKey"] = match;
            receipt["padding"] = "";
            fixture.Receipts[$"{match}-{i}"] = receipt;
        }
    }

    // Spreads padding evenly over the cloned receipts so the written store is close to `bytes`.
    private static void PadTo(LedgerFixture fixture, int bytes)
    {
        var padded = fixture.Receipts.Where(e => e.Value!.AsObject().ContainsKey("padding")).Select(e => e.Value!.AsObject()).ToArray();
        foreach (var receipt in padded) receipt["padding"] = "";
        fixture.Write();
        int each = (bytes - (int)new FileInfo(fixture.Path).Length) / padded.Length;
        Check(each > 0, "Ledger fixture rows are too large before padding");
        foreach (var receipt in padded) receipt["padding"] = new string('p', each);
        fixture.Write();
    }

    private static string[] RowMatches(JsonObject document) =>
        document["matchRewardProgress"]!.AsObject().Select(e => e.Value!["matchKey"]!.GetValue<string>()).ToArray();

    private static string[] ReceiptsOf(JsonObject document, string match) => document["rewardReceipts"]!.AsObject()
        .Where(e => e.Value!["matchKey"]!.GetValue<string>() == match).Select(e => e.Key).ToArray();

    public static void RewardLedgerCompaction() => WithDirectory(directory =>
    {
        var fixture = SeedLedger(directory);
        // Oldest first: an abandoned open match sits among settled ones and must survive.
        for (int i = 0; i < 30; i++)
        {
            AddLedgerRow(fixture, $"settled-{i:D2}", true, 10);
            if (i == 3) AddLedgerRow(fixture, "abandoned-open", false, 10);
        }
        PadTo(fixture, LocalCharacterStore.MaximumStoreBytes / 10 * 9);
        int before = (int)new FileInfo(fixture.Path).Length;
        Check(before > LocalCharacterStore.LedgerCompactionTriggerBytes && before < LocalCharacterStore.MaximumStoreBytes,
            $"Fixture size {before} is outside the compaction window");
        var bravo = fixture.Document["characters"]!.AsArray().Single(c => c!["account"]!.GetValue<string>() == "bravo")!.ToJsonString();
        string[] openReceipts = ReceiptsOf(fixture.Document, "abandoned-open").Concat(ReceiptsOf(fixture.Document, OpenTemplate)).ToArray();

        // The oldest row receives its final reward: it is finalized by this commit, kept, and becomes the newest row.
        var final = fixture.Store().ApplyRewardForAccount("alpha", 1, "template-final", OpenTemplate, "match_minor_defeat", 1);
        Check(final.Result == MagNetResult.Success, "A final reward was refused by a full settled ledger: " + final.Reason);
        var after = LedgerFixture.Read(fixture.Path);
        int size = (int)new FileInfo(fixture.Path).Length;
        Check(size <= LocalCharacterStore.LedgerCompactionTargetBytes, $"Compaction stopped at {size} bytes, above its target");
        Check(fixture.Messages.Count == 1 && fixture.Messages[0].Contains("evicted"), "Compaction was not reported to the operator");

        string[] rows = RowMatches(after);
        Check(rows.Last() == OpenTemplate && after["matchRewardProgress"]!["alpha:1:" + OpenTemplate]!["finalized"]!.GetValue<bool>(),
            "The finalized row was not moved to the newest position");
        Check(rows.Contains("abandoned-open") && openReceipts.Append("template-final").All(id => after["rewardReceipts"]!.AsObject().ContainsKey(id)),
            "An open match row or one of its receipts was evicted");
        var kept = Enumerable.Range(0, 30).Where(i => rows.Contains($"settled-{i:D2}")).ToArray();
        Check(kept.Length >= LocalCharacterStore.RetainedFinalizedMatchRows && kept.Length < 30, "Settled rows were not compacted");
        Check(kept.SequenceEqual(Enumerable.Range(30 - kept.Length, kept.Length)), "Compaction did not evict the least recent settled rows first");
        Check(ReceiptsOf(after, "settled-00").Length == 0 && ReceiptsOf(after, "settled-29").Length == 10,
            "Evicted rows left receipts behind, or retained rows lost them");
        Check(after["characters"]!.AsArray().Single(c => c!["account"]!.GetValue<string>() == "bravo")!.ToJsonString() == bravo,
            "Compaction changed an unrelated character");

        // A retained receipt still answers redelivery exactly, without a write.
        byte[] saved = File.ReadAllBytes(fixture.Path);
        var replay = fixture.Store().ApplyRewardForAccount("alpha", 1, "settled-29-0", "settled-29", "enemy_killed", 1);
        Check(replay.Result == MagNetResult.Success && replay.Duplicate && saved.SequenceEqual(File.ReadAllBytes(fixture.Path)),
            "A retained settled receipt did not replay as a duplicate");
        Check(fixture.Store().ReadAll().Count == 2, "The compacted store does not reopen");
    });

    public static void RewardLedgerHardBound() => WithDirectory(directory =>
    {
        var fixture = SeedLedger(directory);
        for (int i = 0; i < LocalCharacterStore.RetainedFinalizedMatchRows; i++) AddLedgerRow(fixture, $"recent-{i}", true, 4);
        PadTo(fixture, LocalCharacterStore.MaximumStoreBytes - 6000);
        Check(new FileInfo(fixture.Path).Length < LocalCharacterStore.MaximumStoreBytes, "Hard-bound fixture is too large to load");
        var store = fixture.Store();
        for (int i = 0; i < 64 && fixture.Messages.Count == 0; i++)
            Check(store.ApplyRewardForAccount("alpha", 1, $"filler-{i}", OpenTemplate, "enemy_killed", 1).Result == MagNetResult.Success,
                "A reward failed while only retained settled rows filled the store");
        Check(fixture.Messages.Count == 1, "Retained settled rows were never evicted at the hard bound");
        var after = LedgerFixture.Read(fixture.Path);
        string[] rows = RowMatches(after);
        Check(new FileInfo(fixture.Path).Length <= LocalCharacterStore.MaximumStoreBytes, "The store exceeded its hard bound");
        Check(!rows.Contains("recent-0") && rows.Contains($"recent-{LocalCharacterStore.RetainedFinalizedMatchRows - 1}") && rows.Contains(OpenTemplate),
            "The hard bound did not evict only the least recent retained row");
    });

    public static void RewardLedgerOpenOverflow() => WithDirectory(directory =>
    {
        var fixture = SeedLedger(directory);
        for (int i = 0; i < 8; i++) AddLedgerRow(fixture, $"live-{i}", false, 4);
        PadTo(fixture, LocalCharacterStore.MaximumStoreBytes - 6000);
        AddLedgerRow(fixture, "settled-small", true, 1);
        fixture.Write();
        var store = fixture.Store();
        InvalidDataException refused = null;
        byte[] before = File.ReadAllBytes(fixture.Path);
        for (int i = 0; i < 64 && refused == null; i++)
        {
            before = File.ReadAllBytes(fixture.Path);
            try { store.ApplyRewardForAccount("alpha", 1, $"filler-{i}", OpenTemplate, "enemy_killed", 1); }
            catch (InvalidDataException e) { refused = e; }
        }
        Check(refused != null && refused.Message.Contains("open match rows hold"), "Open matches overflowing the store gave no clear reason");
        Check(before.SequenceEqual(File.ReadAllBytes(fixture.Path)), "A refused commit changed the store");
        var after = LedgerFixture.Read(fixture.Path);
        Check(Enumerable.Range(0, 8).All(i => ReceiptsOf(after, $"live-{i}").Length == 4), "Open match receipts were evicted");
    });

    private static long? RowRewardedAt(JsonObject document, string match) =>
        document["matchRewardProgress"]!.AsObject().Single(e => e.Value!["matchKey"]!.GetValue<string>() == match).Value!["lastRewardAt"]?.GetValue<long>();

    private static JsonObject RowOf(JsonObject document, string match) =>
        document["matchRewardProgress"]!.AsObject().Single(e => e.Value!["matchKey"]!.GetValue<string>() == match).Value!.AsObject();

    // Only open rows fill the store: without expiry nothing could be compacted (see RewardLedgerOpenOverflow).
    public static void RewardLedgerAbandonedRowExpiry() => WithDirectory(directory =>
    {
        var fixture = SeedLedger(directory);
        Check(RowRewardedAt(fixture.Document, OpenTemplate) == fixture.Now.ToUnixTimeSeconds(), "A committed reward did not date its match row");
        var ttl = fixture.Ttl;
        // Abandoned rows, oldest first, each last rewarded more than the TTL ago.
        for (int i = 0; i < 20; i++) AddLedgerRow(fixture, $"abandoned-{i:D2}", false, 10, fixture.Now - ttl - TimeSpan.FromMinutes(60 - i));
        // A quiet but active match one minute inside the TTL, and an undated row from an older build.
        AddLedgerRow(fixture, "active-quiet", false, 3, fixture.Now - ttl + TimeSpan.FromMinutes(1));
        AddLedgerRow(fixture, "undated-open", false, 3, undated: true);
        PadTo(fixture, LocalCharacterStore.MaximumStoreBytes / 10 * 9);
        string[] activeReceipts = ReceiptsOf(fixture.Document, "active-quiet").Concat(ReceiptsOf(fixture.Document, "undated-open")).ToArray();

        var store = fixture.Store();
        var reward = store.ApplyRewardForAccount("alpha", 1, "template-action-2", OpenTemplate, "enemy_killed", 1);
        Check(reward.Result == MagNetResult.Success, "A reward was refused while abandoned rows filled the store: " + reward.Reason);
        var after = LedgerFixture.Read(fixture.Path);
        int size = (int)new FileInfo(fixture.Path).Length;
        Check(size <= LocalCharacterStore.LedgerCompactionTargetBytes, $"Expiry compaction stopped at {size} bytes, above its target");
        Check(fixture.Messages.Count == 1 && fixture.Messages[0].Contains("abandoned open"), "Abandoned row eviction was not reported");

        string[] rows = RowMatches(after);
        Check(rows.Contains("active-quiet") && rows.Contains("undated-open") && rows.Contains(OpenTemplate) &&
            activeReceipts.All(id => after["rewardReceipts"]!.AsObject().ContainsKey(id)), "An active open row or one of its receipts was evicted");
        Check(RowRewardedAt(after, "undated-open") == fixture.Now.ToUnixTimeSeconds(), "An undated row did not start its TTL at the next reward commit");
        var kept = Enumerable.Range(0, 20).Where(i => rows.Contains($"abandoned-{i:D2}")).ToArray();
        Check(kept.Length >= LocalCharacterStore.RetainedFinalizedMatchRows && kept.Length < 20, "Abandoned rows were not compacted");
        Check(kept.SequenceEqual(Enumerable.Range(20 - kept.Length, kept.Length)), "Expiry did not evict the least recently rewarded rows first");
        Check(ReceiptsOf(after, "abandoned-00").Length == 0, "An evicted abandoned row left receipts behind");
        Check(after["characters"]!.AsArray().Single(c => c!["account"]!.GetValue<string>() == "alpha")!["experience"]!.GetValue<int>() == reward.Experience,
            "Expiry changed credited character XP");

        // A late reward for an evicted match is not dropped: it opens a fresh row and is credited once.
        var fresh = store.ApplyRewardForAccount("alpha", 1, "late-evicted", "abandoned-00", "enemy_killed", 1);
        Check(fresh.Result == MagNetResult.Success && !fresh.Duplicate && fresh.MatchExperience == fresh.ExperienceAwarded &&
            fresh.Experience == reward.Experience + fresh.ExperienceAwarded, "A late reward for an evicted row was not credited once on a fresh row");
        var again = store.ApplyRewardForAccount("alpha", 1, "late-evicted", "abandoned-00", "enemy_killed", 1);
        Check(again.Result == MagNetResult.Success && again.Duplicate && again.Experience == fresh.Experience,
            "A redelivered late reward was credited twice");
        Check(fixture.Store().ReadAll().Count == 2, "The compacted store does not reopen");
    });

    // A reward for an abandoned row that is still kept resumes that row.
    public static void RewardLedgerLateRewardForAbandonedRow() => WithDirectory(directory =>
    {
        var fixture = SeedLedger(directory);
        var old = fixture.Now - fixture.Ttl - TimeSpan.FromHours(1);
        AddLedgerRow(fixture, "abandoned-kept", false, 2, old);
        fixture.Rows["alpha:1:abandoned-kept"]!["experience"] = 10;
        fixture.Write();
        var store = fixture.Store();
        int xpBefore = store.ReadAll().Single(c => c.Account == "alpha").Experience ?? 0;

        // Redelivery of a receipt the abandoned row still holds replays exactly, without a write.
        byte[] saved = File.ReadAllBytes(fixture.Path);
        var replay = store.ApplyRewardForAccount("alpha", 1, "abandoned-kept-0", "abandoned-kept", "enemy_killed", 1);
        Check(replay.Result == MagNetResult.Success && replay.Duplicate && saved.SequenceEqual(File.ReadAllBytes(fixture.Path)),
            "A kept abandoned receipt did not replay as a duplicate");

        // A new reward revives the kept row: its match XP continues and it is dated now.
        var late = store.ApplyRewardForAccount("alpha", 1, "late-kept", "abandoned-kept", "enemy_killed", 1);
        Check(late.Result == MagNetResult.Success && !late.Duplicate && late.MatchExperience == 10 + late.ExperienceAwarded &&
            late.Experience == xpBefore + late.ExperienceAwarded, "A late reward for a kept abandoned row was not credited once on its row");
        var revived = LedgerFixture.Read(fixture.Path);
        Check(!RowOf(revived, "abandoned-kept")["finalized"]!.GetValue<bool>() && RowRewardedAt(revived, "abandoned-kept") == fixture.Now.ToUnixTimeSeconds(),
            "A late reward did not revive its abandoned row");
    });

    // Rows expire by age alone: inside the TTL they stay however full the store is, past it they compact.
    public static void RewardLedgerOpenRowTtlSetting() => WithDirectory(directory =>
    {
        var fixture = SeedLedger(directory);
        for (int i = 0; i < 12; i++) AddLedgerRow(fixture, $"quiet-{i:D2}", false, 4, fixture.Now - TimeSpan.FromHours(23) - TimeSpan.FromMinutes(12 - i));
        PadTo(fixture, LocalCharacterStore.MaximumStoreBytes / 10 * 9);
        bool AllKept() { var d = LedgerFixture.Read(fixture.Path); return Enumerable.Range(0, 12).All(i => ReceiptsOf(d, $"quiet-{i:D2}").Length == 4); }
        int sequence = 0;
        void Reward() => Check(fixture.Store().ApplyRewardForAccount("alpha", 1, $"template-action-{++sequence}", OpenTemplate, "enemy_killed", 1).Result ==
            MagNetResult.Success, "A reward below the hard bound was refused");

        Reward();
        Check(fixture.Messages.Count == 0 && AllKept(), "A row inside the TTL was evicted");
        // Two hours later the rows are 25 hours old: a configured 48 hour TTL still keeps them.
        fixture.Now += TimeSpan.FromHours(2);
        fixture.Ttl = TimeSpan.FromHours(48);
        Reward();
        Check(fixture.Messages.Count == 0 && AllKept(), "A row inside a configured 48 hour TTL was evicted");
        // The default TTL expires them; the most recently rewarded compactable rows are retained.
        fixture.Ttl = LocalCharacterStore.DefaultOpenMatchRowTtl;
        Reward();
        var after = LedgerFixture.Read(fixture.Path);
        var kept = Enumerable.Range(0, 12).Where(i => RowMatches(after).Contains($"quiet-{i:D2}")).ToArray();
        Check(fixture.Messages.Count == 1 && kept.SequenceEqual(Enumerable.Range(12 - LocalCharacterStore.RetainedFinalizedMatchRows,
            LocalCharacterStore.RetainedFinalizedMatchRows)), "Rows past the TTL were not compacted oldest first");

        Check(Server.Medius.Program.ParseOpenMatchRowTtl(null) == LocalCharacterStore.DefaultOpenMatchRowTtl &&
            Server.Medius.Program.ParseOpenMatchRowTtl(" ") == TimeSpan.FromHours(24), "An unset TTL did not default to 24 hours");
        Check(Server.Medius.Program.ParseOpenMatchRowTtl("6") == TimeSpan.FromHours(6) &&
            Server.Medius.Program.ParseOpenMatchRowTtl("720") == TimeSpan.FromDays(30), "A valid TTL did not parse");
        foreach (var invalid in new[] { "0", "721", "-1", "1.5", "24h", "abc" })
        {
            bool refused = false;
            try { Server.Medius.Program.ParseOpenMatchRowTtl(invalid); }
            catch (InvalidDataException e) { refused = e.Message.Contains("MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS"); }
            Check(refused, $"TTL {invalid} was not refused by name");
        }
        bool unbounded = false;
        try { _ = new LocalCharacterStore(fixture.Path) { OpenMatchRowTtl = TimeSpan.Zero }; }
        catch (ArgumentOutOfRangeException) { unbounded = true; }
        Check(unbounded, "The store accepted a zero TTL");
    });

    // An undated row whose match never rewards again is dated by another row's reward and expires one TTL later.
    public static void RewardLedgerUndatedRowExpiry() => WithDirectory(directory =>
    {
        var fixture = SeedLedger(directory);
        AddLedgerRow(fixture, "undated-ended", false, 20, undated: true);
        PadTo(fixture, LocalCharacterStore.LedgerCompactionTriggerBytes / 2);
        int sequence = 0;
        void Reward() => Check(fixture.Store().ApplyRewardForAccount("alpha", 1, $"template-action-{++sequence}", OpenTemplate, "enemy_killed", 1).Result ==
            MagNetResult.Success, "A reward to another row was refused");

        // A preference save leaves the undated row undated; a reward to another row dates it.
        Check(fixture.Store().SaveNominationsForAccount("alpha", 1, 1).Reason == "nominations-committed" &&
            RowRewardedAt(LedgerFixture.Read(fixture.Path), "undated-ended") == null, "A non-reward save dated a reward row");
        long dated = fixture.Now.ToUnixTimeSeconds();
        Reward();
        Check(RowRewardedAt(LedgerFixture.Read(fixture.Path), "undated-ended") == dated, "Another row's reward did not date an undated row");

        // Later, newer finalized rows fill the store past the trigger; the undated row is never rewarded again.
        fixture.Now += fixture.Ttl - TimeSpan.FromMinutes(1);
        fixture.Document = LedgerFixture.Read(fixture.Path);
        for (int i = 0; i < LocalCharacterStore.RetainedFinalizedMatchRows; i++) AddLedgerRow(fixture, $"recent-final-{i}", true, 4, fixture.Now);
        PadTo(fixture, LocalCharacterStore.MaximumStoreBytes / 10 * 9);
        Reward();
        Check(fixture.Messages.Count == 0 && RowMatches(LedgerFixture.Read(fixture.Path)).Contains("undated-ended"),
            "A once-undated row was evicted inside its TTL");

        // One TTL after it was dated, the next reward to another row compacts it, ahead of the newer retained rows.
        fixture.Now += TimeSpan.FromMinutes(2);
        Reward();
        var after = LedgerFixture.Read(fixture.Path);
        Check(fixture.Messages.Count == 1 && fixture.Messages[0].Contains("1 abandoned open") && !RowMatches(after).Contains("undated-ended") &&
            ReceiptsOf(after, "undated-ended").Length == 0, "An undated row from an ended match never expired");
        Check(Enumerable.Range(0, LocalCharacterStore.RetainedFinalizedMatchRows).All(i => RowMatches(after).Contains($"recent-final-{i}")),
            "Expiring the undated row evicted retained finalized rows");
    });
}
