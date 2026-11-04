using System.Text;
using System.Text.Json.Nodes;
using RT.Models.MAG;
using Server.Medius;

// Invented account names and NP identities built in memory; no client data or real stores.
internal static class NpIdentityChecks
{
    static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    static byte[] NpId(string onlineId)
    {
        var value = new byte[36];
        Encoding.ASCII.GetBytes(onlineId).CopyTo(value, 0);
        return value;
    }

    static MagCreateCharacterRequest Request(string name, byte[] npId) =>
        new MagCreateCharacterRequest { AccountName = name, Faction = 1, HeadId = 2, VoiceId = 3, UniverseId = 26, NpId = npId };

    static void WithStore(Action<string, LocalCharacterStore> action)
    {
        string directory = Path.Combine(Path.GetTempPath(), "mag-np-identity-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            string path = Path.Combine(directory, "characters.json");
            action(path, new LocalCharacterStore(path));
        }
        finally { Directory.Delete(directory, true); }
    }

    public static void Run()
    {
        // Zero identities remain accepted for existing clients and stores.
        WithStore((path, store) =>
        {
            var created = store.CreateForAccount(Request("ExampleZero", new byte[36]), "examplezero");
            Check(created.Result == MagNetResult.Success && created.Reason == "local-profile-committed", "Zero NP identity must remain accepted");
            Check(new LocalCharacterStore(path).ReadForAccount("examplezero").Single().NpId.All(b => b == 0), "Zero NP identity must persist unchanged");
        });

        // An online ID matching the authenticated account is accepted in any ASCII case, persisted
        // exactly, read back by a fresh store and returned in the character record.
        WithStore((path, store) =>
        {
            var npId = NpId("ExampleLinux");
            var created = store.CreateForAccount(Request("ExampleLinux", npId), "examplelinux");
            Check(created.Result == MagNetResult.Success && created.Reason == "local-profile-committed", "Account-bound NP identity must be accepted");
            var stored = new LocalCharacterStore(path).ReadForAccount("examplelinux").Single();
            Check(stored.NpId.SequenceEqual(npId), "Account-bound NP identity must persist exactly");
            Check(stored.ToRecord().NpId.SequenceEqual(npId), "Character record must carry the persisted NP identity");
            var retry = store.CreateForAccount(Request("ExampleLinux", NpId("ExampleLinux")), "examplelinux");
            Check(retry.Result == MagNetResult.Success && retry.Reason == "existing-exact-profile", "Exact retry with the same NP identity must succeed");
            Check(store.CreateForAccount(Request("ExampleLinux", new byte[36]), "examplelinux").Result == MagNetResult.NameTaken,
                "A different NP identity for an occupied account slot must not replace it");
            var other = store.CreateForAccount(Request("ExampleOther", NpId("EXAMPLEOTHER")), "exampleother");
            Check(other.Result == MagNetResult.Success, "Online ID comparison must ignore ASCII case");
            Check(new LocalCharacterStore(path).ReadAll().Count == 2, "Store with non-zero NP identities must remain readable");
        });

        // The binding is to the authenticated account, not to the requested character name.
        WithStore((path, store) =>
        {
            var rejected = new[]
            {
                ("ExampleLinux", NpId("ExampleLinux"), "exampleother"),          // name matches, account does not
                ("ExampleLinux", NpId("ExampleLinu"), "examplelinux"),           // prefix of the account
                ("ExampleLinux", NpId("ExampleLinuxX"), "examplelinux"),         // longer than the account
            }.ToList();
            var trailing = NpId("ExampleLinux"); trailing[35] = 1;               // reserved byte set
            rejected.Add(("ExampleLinux", trailing, "examplelinux"));
            var terminator = NpId("ExampleLinux"); terminator[16] = 1;           // terminator byte set
            rejected.Add(("ExampleLinux", terminator, "examplelinux"));
            var embedded = NpId("ExampleLinux"); embedded[14] = (byte)'x';       // data after the NUL padding starts
            rejected.Add(("ExampleLinux", embedded, "examplelinux"));
            var highBit = NpId("ExampleLinux"); highBit[0] = 0xC5;               // non-ASCII online ID byte
            rejected.Add(("ExampleLinux", highBit, "examplelinux"));
            rejected.Add(("ExampleLinux", new byte[35], "examplelinux"));         // wrong length
            foreach (var (name, npId, account) in rejected)
            {
                var outcome = store.CreateForAccount(Request(name, npId), account);
                Check(outcome.Result == MagNetResult.Error && outcome.Reason == "np-id-account-mismatch", "Unbound NP identity must be rejected: " + account);
            }
            Check(!File.Exists(path), "Rejected NP identities must not create a store");
            var unsupported = Request("ExampleLinux", NpId("ExampleLinux")); unsupported.UniverseId = 27;
            Check(store.CreateForAccount(unsupported, "examplelinux").Reason == "unsupported-local-profile-fields",
                "Universe and faction rejections must keep their existing reason");
        });

        // A store whose NP identity is bound to another account is rejected as a whole.
        WithStore((path, store) =>
        {
            Check(store.CreateForAccount(Request("ExampleLinux", NpId("ExampleLinux")), "examplelinux").Result == MagNetResult.Success, "Store setup");
            var document = JsonNode.Parse(File.ReadAllText(path));
            document["characters"][0]["account"] = "exampleother";
            File.WriteAllText(path, document.ToJsonString());
            try
            {
                new LocalCharacterStore(path).ReadAll();
                throw new Exception("Persisted NP identity bound to another account must be rejected");
            }
            catch (InvalidDataException) { }
        });
    }
}
