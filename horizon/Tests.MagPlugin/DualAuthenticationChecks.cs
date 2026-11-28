using System.Buffers.Binary;
using System.Security.Cryptography;
using System.Text;
using RT.Models;
using RT.Models.MAG;
using Server.Common.Stream;
using Server.Medius;

internal static partial class DualAuthenticationChecks
{
    const long Now = 1_900_000_000;
    const string Service = "MAG_SYNTHETIC_SERVICE";
    static DateTime Clock => DateTime.UnixEpoch.AddSeconds(Now);
    static void Check(bool value, string message) { if (!value) throw new Exception(message); }

    sealed class Identities : IMagExternalIdentityStore
    {
        public bool Linked = true, Available = true, Throws, ProofsAllowed = true;
        public int Proofs;
        public long ProofNowMilliseconds, ProofExpiryMilliseconds;
        public Action OnResolve;
        public MagExternalAccount ResolvePortal(uint id, string name)
        {
            if (Throws) throw new IOException("synthetic unavailable");
            OnResolve?.Invoke();
            return Available && id == 7 && name.Equals("PortalOwner", StringComparison.OrdinalIgnoreCase) ? new(7, "portalowner") : null;
        }
        public MagExternalAccount ResolvePsn(string subject)
        {
            if (Throws) throw new IOException("synthetic unavailable");
            OnResolve?.Invoke();
            return Available && Linked && subject == "256:42" ? new(7, "portalowner") : null;
        }
        public string IssueProof(string subject, string online, long now, long expiry)
        { Proofs++; ProofNowMilliseconds = now; ProofExpiryMilliseconds = expiry; return ProofsAllowed ? "0123456789abcdef0123456789abcdef" : null; }
    }
    sealed class Session : IMagPortalSession { public bool IsPortalSessionLive(DateTime now) => true; }
    sealed class Committer : ICharacterStoreCommitter
    { public void WriteAtomically(string path, byte[] bytes) => File.WriteAllBytes(path, bytes); }

    static MagPortalTicketSettings Settings(ECDsa portal, IReadOnlyDictionary<string, byte[]> psn)
    {
        string secret = Path.Combine(Path.GetTempPath(), "mag-dual-synthetic-" + Guid.NewGuid().ToString("N"));
        File.WriteAllText(secret, "Host=127.0.0.1;Database=synthetic;Username=synthetic");
        try
        {
            return MagPortalTicketSettings.Load(name => name switch
            {
                "MAG_PORTAL_TICKETS" => "dual",
                "MAG_PORTAL_TICKET_KEYS" => "1:" + Convert.ToBase64String(portal.ExportSubjectPublicKeyInfo()),
                "MAG_PORTAL_TICKET_AUDIENCE" => "game.example.test",
                "MAG_PSN_TICKET_KEYS" => string.Join(",", psn.Select(kv => kv.Key + ":" + Convert.ToBase64String(kv.Value))),
                "MAG_PSN_SERVICE_IDS" => Service,
                "MAG_IDENTITY_DB_CONNECTION_FILE" => secret,
                _ => null
            });
        }
        finally { File.Delete(secret); }
    }

    static byte[] Portal(ECDsa key, string name = "PortalOwner", uint accountId = 7)
    {
        byte[] audience = Encoding.ASCII.GetBytes("game.example.test"), player = Encoding.ASCII.GetBytes(name);
        byte[] prefix = new byte[44 + audience.Length + player.Length];
        "MAGT"u8.CopyTo(prefix); prefix[4] = 1; prefix[5] = 1;
        BinaryPrimitives.WriteUInt64BigEndian(prefix.AsSpan(6), Now - 1);
        BinaryPrimitives.WriteUInt64BigEndian(prefix.AsSpan(14), Now + 300);
        BinaryPrimitives.WriteUInt32BigEndian(prefix.AsSpan(22), accountId);
        RandomNumberGenerator.Fill(prefix.AsSpan(26, 16));
        prefix[42] = (byte)audience.Length; audience.CopyTo(prefix, 43);
        prefix[43 + audience.Length] = (byte)player.Length; player.CopyTo(prefix, 44 + audience.Length);
        return prefix.Concat(key.SignData(prefix, HashAlgorithmName.SHA256, DSASignatureFormat.IeeeP1363FixedFieldConcatenation)).ToArray();
    }
    static MagNpAuthenticateRequest Chunk(byte[] bytes) => new MagNpAuthenticateRequest
    { FullTicketSize = (ushort)bytes.Length, TicketChunkSize = (ushort)bytes.Length, TicketChunk = bytes.Concat(new byte[900 - bytes.Length]).ToArray(), LastMessage = true };
    static MagPortalAuthentication Authenticate(MagPortalTicketPolicy policy, byte[] bytes, IMagPortalSession session = null) =>
        policy.Authenticate(new MagPortalTicketAssembly(), Chunk(bytes), session, Clock);

    public static void Configuration()
    {
        try { MagPortalTicketSettings.Load(key => key == "MAG_PORTAL_TICKETS" ? "dual" : null); throw new Exception("Unconfigured dual mode admitted"); }
        catch (ArgumentException e) { Check(e.Message.Contains("MAG_PORTAL_TICKET_KEYS"), "Dual needs portal keys: " + e.Message); }
        using var portal = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        _ = PsnTicketChecks.Ticket(Now, 42, "ConsoleUser", Service, out var keys);
        var settings = Settings(portal, keys);
        Check(settings.Mode.ToString() == "Dual" && settings.Keys.Count == 1 && settings.PsnKeys.Count == 1, "Explicit dual keys/config");
        try { new MagPortalTicketPolicy(settings); throw new Exception("Dual without account resolver"); }
        catch (ArgumentException) { }
    }

    public static void Policy()
    {
        using var portal = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var sony = PsnTicketChecks.Ticket(Now, 42, "ConsoleUser", Service, out var keys);
        var identities = new Identities();
        var policy = new MagPortalTicketPolicy(Settings(portal, keys), () => Clock, identities: identities);
        var first = new Session();
        var accepted = Authenticate(policy, sony, first);
        Check(accepted.Accepted && accepted.Account == "portalowner" && accepted.AccountId == 7 && accepted.OnlineId == "ConsoleUser", "Sony must resolve explicit linked owner, not online name");
        Check(Authenticate(policy, Portal(portal), new Session()).Reason == MagPortalTicketReason.AccountSignedIn, "Portal and Sony must share live owner admission");
        policy.Sessions.Release(first);
        Check(Authenticate(policy, Portal(portal)).Accepted, "Existing portal accepted in dual mode");
        Check(!Authenticate(policy, Portal(portal, "ConsoleUser")).Accepted, "Same-name portal identity cannot select linked owner's storage");
        identities.Linked = false;
        var link = Authenticate(policy, sony);
        Check(link.Reason == MagPortalTicketReason.LinkRequired && link.LinkCode != null && !link.RefusalLog().Contains(link.LinkCode), "Unlinked verified ticket needs private proof, not game access/logged code");
        var bad = sony.ToArray(); bad[^1] ^= 1;
        int issued = identities.Proofs;
        Check(!Authenticate(policy, bad).Accepted && identities.Proofs == issued, "Forged Sony proof must not issue link code");
        Check(!Authenticate(policy, Encoding.ASCII.GetBytes("PortalOwner")).Accepted, "No plaintext fallback");
        var forgedPortal = Portal(portal); forgedPortal[^1] ^= 1;
        Check(Authenticate(policy, forgedPortal).Reason == MagPortalTicketReason.BadSignature && identities.Proofs == issued, "Failed MAGT must never enter Sony/linking path");
        identities.Throws = true;
        Check(Authenticate(policy, sony).Reason == MagPortalTicketReason.AccountUnavailable && Authenticate(policy, Portal(portal)).Reason == MagPortalTicketReason.AccountUnavailable, "Database outage must close both dual paths");
        identities.Throws = false; identities.Linked = true; identities.Available = false; identities.ProofsAllowed = false;
        Check(!Authenticate(policy, sony).Accepted && !Authenticate(policy, Portal(portal)).Accepted, "Unavailable/banned accounts denied");
        identities.Available = true;
        for (int i = 0; i < MagPortalNonceCache.MaximumUses - 1; i++) Check(Authenticate(policy, sony).Accepted, "Bounded Sony reconnect reuse");
        Check(Authenticate(policy, sony).Reason == MagPortalTicketReason.Replayed, "Sony replay bound");
    }

    static IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP> Wire(Func<RT_MSG_CLIENT_APP_TO_PLUGIN, IReadOnlyList<RT_MSG_SERVER_PLUGIN_TO_APP>> handle, MagMessage message, int app)
    {
        byte[] payload = MagMessageCodec.SerializeForApplication(message, app);
        using var stream = new MemoryStream();
        using (var writer = new BinaryWriter(stream, Encoding.ASCII, true)) { writer.Write((ushort)payload.Length); writer.Write((byte)0); writer.Write(payload); }
        stream.Position = 0;
        var envelope = new RT_MSG_CLIENT_APP_TO_PLUGIN();
        using var reader = new MessageReader(stream); envelope.Deserialize(reader);
        return handle(envelope);
    }

    public static void WireSessions()
    {
        using var portal = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var sony = PsnTicketChecks.Ticket(Now, 42, "ConsoleUser", Service, out var keys);
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        try
        {
            foreach (int app in new[] { 20230, 20231 })
            {
                Server.Medius.Program.Settings.MagApplicationId = app;
                var identities = new Identities();
                // Real deployments intentionally have independent MAPS/MAS service policies.
                var mapsPolicy = new MagPortalTicketPolicy(Settings(portal, keys), () => Clock, identities: identities);
                var masPolicy = new MagPortalTicketPolicy(Settings(portal, keys), () => Clock, identities: identities);
                MagNpAuthenticateRequest Request(byte[] bytes)
                {
                    MagNpAuthenticateRequest request = app == 20231 ? new MagRetailNpAuthenticateRequest() : new MagNpAuthenticateRequest();
                    request.FullTicketSize = (ushort)bytes.Length; request.TicketChunkSize = (ushort)bytes.Length;
                    request.TicketChunk = bytes.Concat(new byte[900-bytes.Length]).ToArray(); request.LastMessage = true;
                    return request;
                }
                var maps = new MagMapsSession { PortalTickets = mapsPolicy };
                Wire(e => maps.HandleMany(true, app, e, Clock), new MagHello(), app);
                if (app == 20230) Wire(e => maps.HandleMany(true, app, e, Clock), new MagUniverseListRequest(), app);
                var mapsReplies = Wire(e => maps.HandleMany(true, app, e, Clock), Request(sony), app);
                Check(mapsReplies.Count == 1 && maps.AuthenticatedAccount == "portalowner" && maps.AuthenticatedOnlineId == "ConsoleUser", "Sony MAPS shared owner/identity");
                maps.EndPortalSession();
                var mas = new MagMasSession { PortalTickets = masPolicy };
                Wire(e => mas.HandleMany(true, app, e, Clock), new MagHello(), app);
                Wire(e => mas.HandleMany(true, app, e, Clock), new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, app);
                var masReplies = Wire(e => mas.HandleMany(true, app, e, Clock), Request(sony), app);
                Check(masReplies.Count == 1 && mas.AuthenticatedAccount == "portalowner" && mas.AuthenticatedOnlineId == "ConsoleUser", "Sony MAS shared owner/identity");
                mas.EndPortalSession();
                identities.Linked = false;
                var unlinked = new MagMasSession { PortalTickets = masPolicy };
                Wire(e => unlinked.HandleMany(true, app, e, Clock), new MagHello(), app);
                Wire(e => unlinked.HandleMany(true, app, e, Clock), new MagProtocolInfo { ProtocolVersion = Server.Medius.Program.Settings.MagProtocolVersion }, app);
                Wire(e => unlinked.HandleMany(true, app, e, Clock), Request(sony), app);
                Check(unlinked.AuthenticatedAccount == null && unlinked.ConsoleLinkCode != null && unlinked.PortalTicketStatus == "link_required", "Unlinked console admitted or received no proof");
                var display = MagConsoleLinkMessages.Create(unlinked.ConsoleLinkCode);
                Check(display.Message.Contains(unlinked.ConsoleLinkCode) && !display.ToString().Contains(unlinked.ConsoleLinkCode), "Link code not private to wire message");
                using var wire = new MemoryStream();
                using (var writer = new MessageWriter(wire)) display.Serialize(writer);
                Check(Encoding.UTF8.GetString(wire.ToArray()).Contains(unlinked.ConsoleLinkCode), "Link code missing from serialized connection message");
            }
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    }

    public static void ExpiryAfterLookup()
    {
        using var portal = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        var sony = PsnTicketChecks.Ticket(Now, 42, "ConsoleUser", Service, out var keys);
        long clock = Now;
        var identities = new Identities { OnResolve = () => clock = Now + 1000 };
        var policy = new MagPortalTicketPolicy(Settings(portal, keys), () => DateTime.UnixEpoch.AddSeconds(clock), identities: identities);
        Check(Authenticate(policy, sony).Reason == MagPortalTicketReason.Expired, "Sony proof expired while database lookup waited");
        clock = Now;
        Check(Authenticate(policy, Portal(portal)).Reason == MagPortalTicketReason.Expired, "Portal proof expired while database lookup waited");
        var fractional = PsnTicketChecks.TicketAtMilliseconds(Now*1000-1000, Now*1000+10, 42, "ConsoleUser", Service, out var fractionalKeys);
        DateTime preciseClock = Clock.AddMilliseconds(5);
        var preciseIdentities = new Identities { Linked = false };
        var precisePolicy = new MagPortalTicketPolicy(Settings(portal, fractionalKeys), () => preciseClock, identities: preciseIdentities);
        Check(Authenticate(precisePolicy, fractional).Reason == MagPortalTicketReason.LinkRequired &&
            preciseIdentities.ProofNowMilliseconds == Now*1000+5 && preciseIdentities.ProofExpiryMilliseconds == Now*1000+10,
            "Proof issuance must preserve the exact signed expiry and current milliseconds");
        preciseIdentities.Linked = true; preciseIdentities.OnResolve = () => preciseClock = Clock.AddMilliseconds(11);
        Check(Authenticate(precisePolicy, fractional).Reason == MagPortalTicketReason.Expired, "Subsecond expiry after database lookup must deny admission");
    }

    public static void ActivityAndBinding()
    {
        int previous = Server.Medius.Program.Settings.MagApplicationId;
        try
        {
            Server.Medius.Program.Settings.MagApplicationId = 20230;
            var mas = new MagMasSession();
            Wire(e => mas.HandleMany(true, 20230, e, Clock), new MagHello(), 20230);
            mas.ObserveActivity(Clock.AddSeconds(10)); mas.ObserveActivity(Clock);
            Check(!mas.IsExpired(Clock.AddSeconds(11), 2), "A pre-database timestamp must not overwrite a more recent tick echo");
            var maps = new MagMapsSession();
            Wire(e => maps.HandleMany(true, 20230, e, Clock), new MagHello(), 20230);
            maps.ObserveActivity(Clock.AddSeconds(10)); maps.ObserveActivity(Clock);
            Check(!maps.IsExpired(Clock.AddSeconds(11), 2), "MAPS activity must remain monotonic across threads");
            var bindingType = typeof(MagPortalTicketPolicy).Assembly.GetType("Server.Medius.MagAuthenticationChannelBinding");
            Check(bindingType != null, "Dual transport needs immutable application binding");
            dynamic binding = Activator.CreateInstance(bindingType);
            Check(binding.TryBind(20231) && !binding.TryBind(20230) && !binding.TryBind(20231) && binding.Matches(20231), "Dual transport rebind must fail without changing original app");
        }
        finally { Server.Medius.Program.Settings.MagApplicationId = previous; }
    }

    public static void CredentialDiagnostics()
    {
        const string session = "synthetic-secret-session", access = "synthetic-secret-access";
        Check(!new RT_MSG_CLIENT_CONNECT_TCP { SessionKey = session, AccessToken = access }.ToString().Contains(session), "TCP diagnostic exposes session bearer");
        Check(!new RT_MSG_CLIENT_CONNECT_TCP_AUX_UDP { SessionKey = session, AccessToken = access }.ToString().Contains(access), "Auxiliary diagnostic exposes access bearer");
        Check(MagPortalTicketPolicy.IsSensitiveTicketMessage(0x31) && MagPortalTicketPolicy.IsSensitiveTicketMessage(0x34), "Cross-profile auth payload must be redacted");
        var instantiate = typeof(BaseScertMessage).GetMethod("Instantiate", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic,
            null, new[] { typeof(Type), typeof(RT.Common.RT_MSG_TYPE), typeof(byte[]), typeof(int) }, null);
        // A malformed second subframe must not dump credentials from the aggregate plaintext.
        byte[] plaintext = new byte[] { 0, 0xff, 0x7f }.Concat(Encoding.ASCII.GetBytes(session)).ToArray();
        var output = new StringWriter(); var previous = Console.Out;
        try
        {
            Console.SetOut(output);
            instantiate.Invoke(null, new object[] { typeof(RT_MSG_CLIENT_MULTI_APP_TOSERVER), RT.Common.RT_MSG_TYPE.RT_MSG_CLIENT_MULTI_APP_TOSERVER, plaintext, 112 });
        }
        finally { Console.SetOut(previous); }
        Check(!output.ToString().Contains(Convert.ToHexString(plaintext[..6]), StringComparison.OrdinalIgnoreCase) &&
            !output.ToString().Contains(BitConverter.ToString(plaintext)), "Malformed aggregate diagnostic dumps bearer plaintext");
    }

    public static void CharacterIdentity()
    {
        var method = typeof(LocalCharacterStore).GetMethod("CreateForAccount", new[] { typeof(MagCreateCharacterRequest), typeof(string), typeof(string) });
        Check(method != null, "Character creation needs verified online identity separate from owner");
        string directory = Path.Combine(Path.GetTempPath(), "mag-dual-character-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        try
        {
            var store = new LocalCharacterStore(Path.Combine(directory, "characters.json"), new Committer());
            byte[] np = new byte[36]; Encoding.ASCII.GetBytes("ConsoleUser").CopyTo(np, 0);
            var request = new MagCreateCharacterRequest { AccountName = "ConsoleUser!0", Faction = 1, UniverseId = 26, NpId = np };
            var result = (CharacterCreationOutcome)method.Invoke(store, new object[] { request, "portalowner", "ConsoleUser" });
            Check(result.Result == MagNetResult.Success, "Verified different-name Sony user could not create canonical owner's character: " + result.Reason);
            Check(store.ReadForAccount("portalowner").Count == 1 && store.ReadForAccount("consoleuser").Count == 0, "Canonical owner must own character");
            Check(request.NpId.SequenceEqual(np), "Do not mutate caller's request");
            var denied = (CharacterCreationOutcome)method.Invoke(store, new object[] { request, "otherowner", "SomeoneElse" });
            Check(denied.Result != MagNetResult.Success, "Mismatched NP identity must fail");
        }
        finally { Directory.Delete(directory, true); }
    }
}
