using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Server.Medius
{
    /// <summary>
    /// Settings for the internal admin API. Both variables must be set to enable it; neither set
    /// leaves it off. Error messages name the variables and the file path, never the secret.
    /// </summary>
    public sealed class MagAdminApiSettings
    {
        public const string ListenVariable = "MAG_ADMIN_API_LISTEN";
        public const string SecretFileVariable = "MAG_ADMIN_API_SECRET_FILE";
        public const int MinimumSecretBytes = 32;

        public IPEndPoint Listen { get; private set; }
        public byte[] Secret { get; private set; }

        public static MagAdminApiSettings FromEnvironment() => Load(Environment.GetEnvironmentVariable);

        /// <summary>Null when the API is off; throws <see cref="ArgumentException"/> for incomplete or invalid settings.</summary>
        public static MagAdminApiSettings Load(Func<string, string> read)
        {
            string listen = read(ListenVariable), secretFile = read(SecretFileVariable);
            bool hasListen = !string.IsNullOrWhiteSpace(listen), hasSecret = !string.IsNullOrWhiteSpace(secretFile);
            if (!hasListen && !hasSecret) return null;
            if (!hasListen || !hasSecret)
                throw new ArgumentException($"The admin API needs both {ListenVariable} and {SecretFileVariable}; only {(hasListen ? ListenVariable : SecretFileVariable)} is set.");
            listen = listen.Trim();
            int colon = listen.LastIndexOf(':');
            if (colon <= 0 || !IPEndPoint.TryParse(listen, out var endpoint) || endpoint.Port < 1 ||
                !int.TryParse(listen.AsSpan(colon + 1), System.Globalization.NumberStyles.None, null, out int port) || port != endpoint.Port)
                throw new ArgumentException($"{ListenVariable} must be an IP address and port, for example 0.0.0.0:10490.");
            byte[] secret;
            try { secret = Encoding.UTF8.GetBytes(File.ReadAllText(secretFile.Trim()).Trim()); }
            catch (Exception e) when (e is IOException || e is UnauthorizedAccessException || e is ArgumentException || e is NotSupportedException)
            { throw new ArgumentException($"{SecretFileVariable} names a file that cannot be read: {secretFile.Trim()} ({e.GetType().Name})."); }
            if (secret.Length < MinimumSecretBytes)
                throw new ArgumentException($"The admin API secret in {SecretFileVariable} must hold at least {MinimumSecretBytes} bytes.");
            return new MagAdminApiSettings { Listen = endpoint, Secret = secret };
        }

        /// <summary>The API belongs to the process that owns the character store: never the MAPS role.</summary>
        public static void RequireMainRole(Config.ServerSettings settings)
        {
            if (settings.EnableMagPluginAdapter)
                throw new ArgumentException($"{ListenVariable} is set for a MAPS process; the admin API runs only in the main medius role that owns the character store.");
            if (string.IsNullOrWhiteSpace(settings.MagCharacterStorePath))
                throw new ArgumentException($"{ListenVariable} needs MagCharacterStorePath: the admin API runs only in the main medius role that owns the character store.");
        }
    }

    /// <summary>
    /// HMAC request authentication for the admin API. The signature covers
    /// <c>METHOD\nPATH_AND_QUERY\nTIMESTAMP\nNONCE\nhex(SHA-256(body))</c>. Nonces are remembered for
    /// five minutes; the cache is bounded and refuses new requests rather than forget a live nonce.
    /// </summary>
    public sealed class MagAdminApiAuthenticator
    {
        public const string TimestampHeader = "X-Mag-Admin-Timestamp";
        public const string NonceHeader = "X-Mag-Admin-Nonce";
        public const string ActorHeader = "X-Mag-Admin-Actor";
        public const string SignatureHeader = "X-Mag-Admin-Signature";
        public static readonly TimeSpan MaximumSkew = TimeSpan.FromSeconds(60);
        public static readonly TimeSpan NonceLifetime = TimeSpan.FromMinutes(5);
        public const int NonceCapacity = 10000;

        private readonly byte[] _secret;
        private readonly object _gate = new object();
        private readonly Dictionary<string, DateTime> _nonces = new Dictionary<string, DateTime>(StringComparer.Ordinal);
        private readonly Queue<(string Nonce, DateTime Expires)> _expiry = new Queue<(string, DateTime)>();

        public MagAdminApiAuthenticator(byte[] secret)
        {
            if (secret == null || secret.Length < MagAdminApiSettings.MinimumSecretBytes) throw new ArgumentException("The admin API secret is too short.");
            _secret = (byte[])secret.Clone();
        }

        public static string BodyHash(ReadOnlySpan<byte> body) => Convert.ToHexString(SHA256.HashData(body)).ToLowerInvariant();

        public static string Sign(byte[] secret, string method, string pathAndQuery, string timestamp, string nonce, ReadOnlySpan<byte> body) =>
            Convert.ToHexString(HMACSHA256.HashData(secret,
                Encoding.UTF8.GetBytes(method + "\n" + pathAndQuery + "\n" + timestamp + "\n" + nonce + "\n" + BodyHash(body)))).ToLowerInvariant();

        /// <summary>Null when authenticated; otherwise the reason class (never header values).</summary>
        public string Verify(string method, string pathAndQuery, IReadOnlyDictionary<string, string> headers, ReadOnlySpan<byte> body, DateTime now, out string actor)
        {
            actor = null;
            if (!headers.TryGetValue(TimestampHeader, out var timestamp) || !headers.TryGetValue(NonceHeader, out var nonce) ||
                !headers.TryGetValue(ActorHeader, out var actorValue) || !headers.TryGetValue(SignatureHeader, out var signature))
                return "missing-header";
            if (timestamp.Length < 1 || timestamp.Length > 12 || !timestamp.All(char.IsAsciiDigit) ||
                nonce.Length < 16 || nonce.Length > 64 || !nonce.All(c => char.IsAsciiLetterOrDigit(c) || c == '_' || c == '-') ||
                actorValue.Length < 1 || actorValue.Length > 254 || !actorValue.All(c => c >= 0x20 && c <= 0x7e) ||
                signature.Length != 64 || !signature.All(c => char.IsAsciiDigit(c) || (c >= 'a' && c <= 'f')))
                return "malformed-header";
            long nowSeconds = new DateTimeOffset(now.ToUniversalTime()).ToUnixTimeSeconds();
            if (Math.Abs(nowSeconds - long.Parse(timestamp)) > (long)MaximumSkew.TotalSeconds) return "clock-skew";
            string expected = Sign(_secret, method, pathAndQuery, timestamp, nonce, body);
            if (!CryptographicOperations.FixedTimeEquals(Encoding.ASCII.GetBytes(expected), Encoding.ASCII.GetBytes(signature)))
                return "bad-signature";
            lock (_gate)
            {
                while (_expiry.Count > 0 && _expiry.Peek().Expires <= now)
                {
                    var old = _expiry.Dequeue();
                    if (_nonces.TryGetValue(old.Nonce, out var expires) && expires == old.Expires) _nonces.Remove(old.Nonce);
                }
                if (_nonces.TryGetValue(nonce, out var live) && live > now) return "replay";
                if (_nonces.Count >= NonceCapacity) return "nonce-capacity";
                var until = now + NonceLifetime;
                _nonces[nonce] = until;
                _expiry.Enqueue((nonce, until));
            }
            actor = actorValue;
            return null;
        }
    }

    /// <summary>A parsed admin API request; header names are case-insensitive.</summary>
    public sealed class MagAdminApiRequest
    {
        public string Method { get; init; }
        public string PathAndQuery { get; init; }
        public IReadOnlyDictionary<string, string> Headers { get; init; } = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        public byte[] Body { get; init; } = Array.Empty<byte>();
    }

    public sealed class MagAdminApiResponse
    {
        public int Status { get; init; }
        public byte[] Body { get; init; }
        public string Allow { get; init; }
        public static MagAdminApiResponse Json(int status, object value, string allow = null) =>
            new MagAdminApiResponse { Status = status, Body = JsonSerializer.SerializeToUtf8Bytes(value), Allow = allow };
        public static MagAdminApiResponse Error(int status, string code) => Json(status, new { error = code });
    }

    /// <summary>
    /// Internal, authenticated HTTP/1.1 admin API for character maintenance (docs/admin-cli.md).
    /// It is off unless both MAG_ADMIN_API_* variables are set, runs only in the main medius role, and
    /// applies the same rules and code path as <c>mag-admin</c> through <see cref="MagAdminOperations"/>.
    /// One request per connection; bodies are limited to 16 KB.
    /// </summary>
    public sealed class MagAdminApi : IAsyncDisposable
    {
        public const int MaximumBodyBytes = 16 * 1024;
        public const int MaximumHeadBytes = 16 * 1024;
        private const int MaximumConnections = 16;
        private static readonly TimeSpan RequestTimeout = TimeSpan.FromSeconds(10);

        private readonly IPEndPoint _listen;
        private readonly MagAdminApiAuthenticator _authenticator;
        private readonly LocalCharacterStore _store;
        private readonly string _backupDirectory, _markerDirectory;
        private readonly Func<DateTime> _clock;
        private readonly Action<string> _log;
        private readonly MagDmePortBudget _portBudget;
        private readonly Func<DateTime, object> _status;
        private readonly Func<IReadOnlyList<MagAdminGameplayHost>> _gameplayHosts;
        private readonly Action _voiceSanctionsChanged;
        private readonly SemaphoreSlim _connections = new SemaphoreSlim(MaximumConnections);
        private readonly CancellationTokenSource _stop = new CancellationTokenSource();
        private TcpListener _listener;
        private Task _acceptLoop;

        public MagAdminApi(IPEndPoint listen, byte[] secret, LocalCharacterStore store, string backupDirectory, string markerDirectory,
            Func<DateTime> clock, Action<string> log, MagDmePortBudget portBudget = null, Func<DateTime, object> status = null,
            Func<IReadOnlyList<MagAdminGameplayHost>> gameplayHosts = null, Action voiceSanctionsChanged = null)
        {
            _voiceSanctionsChanged = voiceSanctionsChanged ?? (() => { });
            _portBudget = portBudget ?? MagDmePortBudget.Unbounded();
            _status = status;
            _gameplayHosts = gameplayHosts ?? (() => Array.Empty<MagAdminGameplayHost>());
            _listen = listen ?? throw new ArgumentNullException(nameof(listen));
            _authenticator = new MagAdminApiAuthenticator(secret);
            _store = store ?? throw new ArgumentNullException(nameof(store));
            _backupDirectory = backupDirectory; _markerDirectory = markerDirectory;
            _clock = clock ?? (() => DateTime.UtcNow);
            _log = log ?? (_ => { });
        }

        public IPEndPoint LocalEndPoint => (IPEndPoint)_listener?.LocalEndpoint;

        public void Start()
        {
            if (_listener != null) throw new InvalidOperationException("The admin API is already started.");
            _listener = new TcpListener(_listen);
            _listener.Start();
            _acceptLoop = AcceptLoop();
        }

        public async ValueTask DisposeAsync()
        {
            _stop.Cancel();
            _listener?.Stop();
            if (_acceptLoop != null) { try { await _acceptLoop; } catch (Exception) { } }
        }

        private async Task AcceptLoop()
        {
            while (!_stop.IsCancellationRequested)
            {
                TcpClient client;
                try { client = await _listener.AcceptTcpClientAsync(_stop.Token); }
                catch (OperationCanceledException) { return; }
                catch (ObjectDisposedException) { return; }
                catch (SocketException) { if (_stop.IsCancellationRequested) return; continue; }
                if (!_connections.Wait(0)) { client.Dispose(); continue; }
                _ = Task.Run(async () =>
                {
                    try { await Serve(client); }
                    catch (Exception e) { _log("admin api: connection failed: " + e.GetType().Name); }
                    finally { client.Dispose(); _connections.Release(); }
                });
            }
        }

        private async Task Serve(TcpClient client)
        {
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token);
            timeout.CancelAfter(RequestTimeout);
            var stream = client.GetStream();
            MagAdminApiResponse response;
            try
            {
                var (request, failure) = await ReadRequest(stream, timeout.Token);
                response = failure ?? await Task.Run(() => Handle(request));
            }
            catch (OperationCanceledException) { return; }
            catch (IOException) { return; }
            await WriteResponse(stream, response, timeout.Token);
        }

        private static async Task<(MagAdminApiRequest, MagAdminApiResponse)> ReadRequest(NetworkStream stream, CancellationToken cancel)
        {
            var buffer = new byte[MaximumHeadBytes + MaximumBodyBytes];
            int length = 0, headEnd = -1;
            while (headEnd < 0)
            {
                if (length >= MaximumHeadBytes) return (null, MagAdminApiResponse.Error(431, "headers_too_large"));
                int read = await stream.ReadAsync(buffer.AsMemory(length, MaximumHeadBytes - length), cancel);
                if (read == 0) throw new IOException("Connection closed before the request head.");
                length += read;
                headEnd = buffer.AsSpan(0, length).IndexOf("\r\n\r\n"u8);
            }
            var invalid = MagAdminApiResponse.Error(400, "invalid_request");
            string head;
            try { head = new UTF8Encoding(false, true).GetString(buffer, 0, headEnd); }
            catch (DecoderFallbackException) { return (null, invalid); }
            if (head.Any(c => c > 0x7e)) return (null, invalid);
            var lines = head.Split("\r\n");
            var start = lines[0].Split(' ');
            if (start.Length != 3 || start[0].Length == 0 || !start[0].All(char.IsAsciiLetterUpper) ||
                !start[1].StartsWith('/') || start[1].Any(c => c <= 0x20) || (start[2] != "HTTP/1.1" && start[2] != "HTTP/1.0"))
                return (null, invalid);
            var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            foreach (var line in lines.Skip(1))
            {
                int colon = line.IndexOf(':');
                if (colon <= 0 || line[..colon].Any(c => c <= 0x20)) return (null, invalid);
                string name = line[..colon], value = line[(colon + 1)..].Trim(' ', '\t');
                if (!headers.TryAdd(name, value)) return (null, invalid);
            }
            if (headers.ContainsKey("Transfer-Encoding")) return (null, invalid);
            int bodyLength = 0;
            if (headers.TryGetValue("Content-Length", out var declared) &&
                (declared.Length == 0 || declared.Length > 9 || !declared.All(char.IsAsciiDigit) || !int.TryParse(declared, out bodyLength)))
                return (null, invalid);
            if (bodyLength > MaximumBodyBytes) return (null, MagAdminApiResponse.Error(413, "payload_too_large"));
            int bodyStart = headEnd + 4;
            if (length - bodyStart > bodyLength) return (null, invalid); // pipelined or undeclared bytes
            while (length - bodyStart < bodyLength)
            {
                int read = await stream.ReadAsync(buffer.AsMemory(length, bodyStart + bodyLength - length), cancel);
                if (read == 0) throw new IOException("Connection closed before the request body.");
                length += read;
            }
            return (new MagAdminApiRequest { Method = start[0], PathAndQuery = start[1], Headers = headers,
                Body = buffer.AsSpan(bodyStart, bodyLength).ToArray() }, null);
        }

        private static async Task WriteResponse(NetworkStream stream, MagAdminApiResponse response, CancellationToken cancel)
        {
            string reason = response.Status switch
            {
                200 => "OK", 400 => "Bad Request", 401 => "Unauthorized", 404 => "Not Found", 405 => "Method Not Allowed",
                409 => "Conflict", 413 => "Content Too Large", 431 => "Request Header Fields Too Large", 503 => "Service Unavailable",
                _ => "Internal Server Error"
            };
            var head = new StringBuilder()
                .Append("HTTP/1.1 ").Append(response.Status).Append(' ').Append(reason).Append("\r\n")
                .Append("Content-Type: application/json\r\n")
                .Append("Content-Length: ").Append(response.Body.Length).Append("\r\n")
                .Append("Cache-Control: no-store\r\n");
            if (response.Allow != null) head.Append("Allow: ").Append(response.Allow).Append("\r\n");
            head.Append("Connection: close\r\n\r\n");
            await stream.WriteAsync(Encoding.ASCII.GetBytes(head.ToString()), cancel);
            await stream.WriteAsync(response.Body, cancel);
            await stream.FlushAsync(cancel);
        }

        /// <summary>Authenticate and route one request. Public for synthetic checks without sockets.</summary>
        public MagAdminApiResponse Handle(MagAdminApiRequest request)
        {
            DateTime now = _clock();
            string refusal = _authenticator.Verify(request.Method, request.PathAndQuery, request.Headers, request.Body, now, out string actor);
            if (refusal != null)
            {
                _log("admin api: unauthorized request (" + refusal + ")");
                return MagAdminApiResponse.Error(401, "unauthorized");
            }
            string path = request.PathAndQuery;
            int query = path.IndexOf('?');
            if (query >= 0) path = path[..query];
            var segments = path.Split('/');
            try
            {
                switch (segments)
                {
                    case ["", "admin", "v1", "accounts"]:
                        return request.Method != "GET" ? MethodNotAllowed("GET") : Accounts(now);
                    case ["", "admin", "v1", "characters", var account] when account.Length > 0:
                        return request.Method != "GET" ? MethodNotAllowed("GET") : Character(account, now);
                    case ["", "admin", "v1", "characters", var account, "level"] when account.Length > 0:
                        return request.Method != "POST" ? MethodNotAllowed("POST") : SetLevel(account, request.Body, actor, now);
                    case ["", "admin", "v1", "characters", var account, "kill"] when account.Length > 0:
                        return request.Method != "POST" ? MethodNotAllowed("POST") : Kill(account, request.Body, actor, now);
                    case ["", "admin", "v1", "voice"]:
                        return request.Method != "GET" ? MethodNotAllowed("GET") : VoiceList(now);
                    case ["", "admin", "v1", "voice", var account, var action] when account.Length > 0 &&
                        action is "mute" or "ban" or "unmute" or "unban":
                        return request.Method != "POST" ? MethodNotAllowed("POST") : Voice(account, action, request.Body, actor, now);
                    case ["", "admin", "v1", "dme-ports"]:
                        return request.Method != "GET" ? MethodNotAllowed("GET") : DmePorts();
                    case ["", "admin", "v1", "status"] when _status != null:
                        return request.Method != "GET" ? MethodNotAllowed("GET") : MagAdminApiResponse.Json(200, _status(now));
                    default:
                        return MagAdminApiResponse.Error(404, "not_found");
                }
            }
            catch (Exception e)
            {
                _log($"admin api: {request.Method} {path} failed: {e.GetType().Name}: {e.Message}");
                return MagAdminApiResponse.Error(500, "internal_error");
            }
        }

        private static MagAdminApiResponse MethodNotAllowed(string allow) =>
            MagAdminApiResponse.Json(405, new { error = "method_not_allowed" }, allow);

        private MagAdminApiResponse Accounts(DateTime now) => MagAdminApiResponse.Json(200, new
        {
            accounts = MagAdminOperations.ListAccounts(_store, _markerDirectory, now).Select(c => new
            { account = c.Account, level = c.Level, experience = c.Experience, skill_points = c.SkillPoints, session = c.Session })
        });

        private MagAdminApiResponse DmePorts()
        {
            int inUse = _portBudget.InUse;
            return MagAdminApiResponse.Json(200, new
            {
                capacity = _portBudget.Capacity, in_use = inUse,
                available = _portBudget.Capacity is int cap ? Math.Max(0, cap - inUse) : (int?)null,
                first_port = MagDmePortBudget.FirstPort, last_port = _portBudget.LastPort, source = _portBudget.Source,
                udp_ports_bound = Server.Dme.UdpServer.PortsInUse
            });
        }

        private MagAdminApiResponse Character(string account, DateTime now)
        {
            var c = MagDevelopmentAccountPolicy.ValidAccount(account) ? MagAdminOperations.Show(_store, account, _markerDirectory, now) : null;
            if (c == null) return MagAdminApiResponse.Error(404, "not_found");
            return MagAdminApiResponse.Json(200, new
            {
                account = c.Account, level = c.Level, experience = c.Experience, skill_points = c.SkillPoints,
                next_level_experience = c.NextLevelExperience, session = c.Session
            });
        }

        private MagAdminApiResponse Kill(string account, byte[] body, string actor, DateTime now)
        {
            bool apply; string reason = null;
            try
            {
                using var document = JsonDocument.Parse(body);
                var root = document.RootElement;
                if (root.ValueKind != JsonValueKind.Object || root.EnumerateObject().Any(p => p.Name != "apply" && p.Name != "reason") ||
                    !root.TryGetProperty("apply", out var applyValue) || (applyValue.ValueKind != JsonValueKind.True && applyValue.ValueKind != JsonValueKind.False))
                    return MagAdminApiResponse.Error(400, "invalid_request");
                apply = applyValue.GetBoolean();
                if (root.TryGetProperty("reason", out var reasonValue) && reasonValue.ValueKind != JsonValueKind.Null)
                {
                    if (reasonValue.ValueKind != JsonValueKind.String) return MagAdminApiResponse.Error(400, "invalid_request");
                    reason = reasonValue.GetString();
                }
            }
            catch (JsonException) { return MagAdminApiResponse.Error(400, "invalid_request"); }
            if (!MagAdminKill.ValidReason(reason)) return MagAdminApiResponse.Error(400, "invalid_request");
            var result = MagAdminKill.KillAsync(_store, _gameplayHosts(), account, apply, reason, actor, _markerDirectory, now)
                .GetAwaiter().GetResult();
            string who = JsonSerializer.Serialize(actor), why = JsonSerializer.Serialize(reason);
            string where = $"worker={result.WorkerId ?? "none"} world={result.WorldId?.ToString() ?? "none"} player={result.PlayerId?.ToString() ?? "none"}";
            switch (result.Refusal)
            {
                case MagAdminKillRefusal.None:
                    break;
                case MagAdminKillRefusal.NotFound:
                    return MagAdminApiResponse.Error(404, "not_found");
                case MagAdminKillRefusal.InvalidReason:
                    return MagAdminApiResponse.Error(400, "invalid_request");
                default:
                    _log($"admin api: kill refused ({result.Code}) actor={who} account={account} {where} apply={apply} reason={why}" +
                        (result.WorkerReason == null ? "" : $" worker_reason={result.WorkerReason}"));
                    int status = result.Refusal is MagAdminKillRefusal.WorkerUnavailable or MagAdminKillRefusal.WorkerUnanswered ? 503 : 409;
                    return MagAdminApiResponse.Json(status, new
                    {
                        error = result.Code, message = result.Message, worker_id = result.WorkerId, world_id = result.WorldId,
                        player_id = result.PlayerId, worker_reason = result.WorkerReason
                    });
            }
            _log(result.Applied
                ? $"admin api: applied kill actor={who} account={account} {where} result={result.Result} reason={why}"
                : $"admin api: dry-run kill actor={who} account={account} {where} reason={why}");
            return MagAdminApiResponse.Json(200, new
            {
                applied = result.Applied, account = result.Account, worker_id = result.WorkerId, world_id = result.WorldId,
                player_id = result.PlayerId, result = result.Result, reason
            });
        }

        private MagAdminApiResponse VoiceList(DateTime now) => MagAdminApiResponse.Json(200, new
        {
            sanctions = MagVoiceModeration.Active(_store, new DateTimeOffset(DateTime.SpecifyKind(now, DateTimeKind.Utc))).Select(MagVoiceModeration.Json)
        });

        /// <summary>Voice mute, ban, unmute or unban (docs/admin-cli.md). The actor header is the operator.</summary>
        private MagAdminApiResponse Voice(string account, string action, byte[] body, string actor, DateTime now)
        {
            bool lift = action.StartsWith("un", StringComparison.Ordinal);
            var kind = action.EndsWith("ban", StringComparison.Ordinal) ? VoiceSanctionKind.Ban : VoiceSanctionKind.Mute;
            bool apply; string reason = null; TimeSpan? duration = null;
            try
            {
                using var document = JsonDocument.Parse(body);
                var root = document.RootElement;
                if (root.ValueKind != JsonValueKind.Object ||
                    root.EnumerateObject().Any(p => p.Name != "apply" && p.Name != "reason" && (lift || p.Name != "duration_minutes")) ||
                    !root.TryGetProperty("apply", out var applyValue) || (applyValue.ValueKind != JsonValueKind.True && applyValue.ValueKind != JsonValueKind.False))
                    return MagAdminApiResponse.Error(400, "invalid_request");
                apply = applyValue.GetBoolean();
                if (root.TryGetProperty("reason", out var reasonValue) && reasonValue.ValueKind != JsonValueKind.Null)
                {
                    if (reasonValue.ValueKind != JsonValueKind.String) return MagAdminApiResponse.Error(400, "invalid_request");
                    reason = reasonValue.GetString();
                }
                if (root.TryGetProperty("duration_minutes", out var minutesValue) && minutesValue.ValueKind != JsonValueKind.Null)
                {
                    if (minutesValue.ValueKind != JsonValueKind.Number || !minutesValue.TryGetInt64(out long minutes) || minutes < 1 ||
                        minutes > (long)LocalCharacterStore.MaximumVoiceSanctionDuration.TotalMinutes)
                        return MagAdminApiResponse.Error(400, "invalid_request");
                    duration = TimeSpan.FromMinutes(minutes);
                }
            }
            catch (JsonException) { return MagAdminApiResponse.Error(400, "invalid_request"); }
            var result = MagVoiceModeration.Change(_store, account, kind, lift, reason, actor, duration, apply, _backupDirectory);
            string who = JsonSerializer.Serialize(actor), why = JsonSerializer.Serialize(reason);
            switch (result.Refusal)
            {
                case MagVoiceRefusal.None:
                    break;
                case MagVoiceRefusal.NotFound:
                    return MagAdminApiResponse.Error(404, "not_found");
                case MagVoiceRefusal.InvalidRequest:
                    return MagAdminApiResponse.Error(400, "invalid_request");
                case MagVoiceRefusal.Failed:
                    _log($"admin api: voice {action} failed actor={who} account={account}: {result.Message}");
                    return MagAdminApiResponse.Error(500, "internal_error");
                default:
                    _log($"admin api: voice {action} refused ({result.Code}) actor={who} account={account} apply={apply}");
                    return MagAdminApiResponse.Json(409, new { error = result.Code, message = result.Message });
            }
            var change = result.Change;
            if (change.Applied) _voiceSanctionsChanged();
            string until = lift ? "" : " until=" + (change.After.ExpiresAt?.UtcDateTime.ToString("yyyy-MM-ddTHH:mm:ssZ") ?? "lifted");
            _log(change.Applied
                ? $"admin api: applied voice {action} actor={who} account={account}{until} reason={why} journal={change.Journal}"
                : $"admin api: dry-run voice {action} actor={who} account={account}{until} reason={why}");
            return MagAdminApiResponse.Json(200, new
            {
                applied = change.Applied, account = change.Account, action, kind = VoiceSanction.Name(kind),
                before = MagVoiceModeration.Json(change.Before), after = MagVoiceModeration.Json(change.After)
            });
        }

        private MagAdminApiResponse SetLevel(string account, byte[] body, string actor, DateTime now)
        {
            long level; bool apply;
            try
            {
                using var document = JsonDocument.Parse(body);
                var root = document.RootElement;
                if (root.ValueKind != JsonValueKind.Object ||
                    !root.TryGetProperty("level", out var levelValue) || levelValue.ValueKind != JsonValueKind.Number || !levelValue.TryGetInt64(out level) ||
                    !root.TryGetProperty("apply", out var applyValue) || (applyValue.ValueKind != JsonValueKind.True && applyValue.ValueKind != JsonValueKind.False))
                    return MagAdminApiResponse.Error(400, "invalid_request");
                apply = applyValue.GetBoolean();
            }
            catch (JsonException) { return MagAdminApiResponse.Error(400, "invalid_request"); }
            int requested = level < int.MinValue ? int.MinValue : level > int.MaxValue ? int.MaxValue : (int)level;
            var result = MagAdminOperations.SetLevel(_store, account, requested, apply, _backupDirectory, _markerDirectory, now);
            string who = JsonSerializer.Serialize(actor);
            switch (result.Refusal)
            {
                case MagAdminRefusal.None:
                    break;
                case MagAdminRefusal.NotFound:
                    return MagAdminApiResponse.Error(404, "not_found");
                case MagAdminRefusal.Failed:
                    _log($"admin api: set-level failed actor={who} account={account} level={level}: {result.Message}");
                    return MagAdminApiResponse.Error(500, "internal_error");
                default:
                    _log($"admin api: set-level refused ({result.Code}) actor={who} account={account} level={level} apply={apply}");
                    return MagAdminApiResponse.Json(409, new { error = result.Code, message = result.Message });
            }
            var change = result.Change;
            string backup = change.BackupPath == null ? null : Path.GetFileName(change.BackupPath);
            _log(change.Applied
                ? $"admin api: applied set-level actor={who} account={change.Account} from={change.PreviousLevel} to={change.Level} backup={backup}"
                : $"admin api: dry-run set-level actor={who} account={change.Account} from={change.PreviousLevel} to={change.Level}");
            return MagAdminApiResponse.Json(200, new
            {
                applied = change.Applied, account = change.Account, from_level = change.PreviousLevel, to_level = change.Level,
                experience_before = change.PreviousExperience, experience_after = change.Experience,
                skill_points_before = change.PreviousSkillPoints, skill_points_after = change.SkillPoints, backup
            });
        }
    }
}
