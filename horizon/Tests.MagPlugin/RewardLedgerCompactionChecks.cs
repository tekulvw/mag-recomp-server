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
        public LocalCharacterStore Store() => new LocalCharacterStore(Path) { ProgressionCatalog = Catalog, LedgerMaintenance = Messages.Add };
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
    private static void AddLedgerRow(LedgerFixture fixture, string match, bool finalized, int receipts)
    {
        var row = fixture.Progress.DeepClone().AsObject();
        row["matchKey"] = match;
        row["finalized"] = finalized;
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
        InvalidDataException? refused = null;
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
}
