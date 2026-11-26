using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.Threading;

namespace Server.Medius
{
    /// <summary>
    /// Signed client for this process's own admin API, used by <c>mag-admin</c> for operations that act on
    /// the running server (an operator kill reaches the live gameplay worker only through the server
    /// that admitted the player). It reads the same MAG_ADMIN_API_* variables as the server, so it works
    /// inside the main medius container, and connects to loopback when the API listens on every address.
    /// </summary>
    public sealed class MagAdminApiClient
    {
        static readonly TimeSpan Deadline = TimeSpan.FromSeconds(15);
        const int MaximumResponseBytes = 64 * 1024;
        readonly IPEndPoint _endpoint;
        readonly byte[] _secret;

        public MagAdminApiClient(IPEndPoint endpoint, byte[] secret)
        {
            _endpoint = endpoint ?? throw new ArgumentNullException(nameof(endpoint));
            _secret = secret ?? throw new ArgumentNullException(nameof(secret));
            if (_endpoint.Address.Equals(IPAddress.Any)) _endpoint = new IPEndPoint(IPAddress.Loopback, _endpoint.Port);
            else if (_endpoint.Address.Equals(IPAddress.IPv6Any)) _endpoint = new IPEndPoint(IPAddress.IPv6Loopback, _endpoint.Port);
        }

        /// <summary>Null when the API is not configured; throws <see cref="ArgumentException"/> for invalid settings.</summary>
        public static MagAdminApiClient FromEnvironment()
        {
            var settings = MagAdminApiSettings.FromEnvironment();
            return settings == null ? null : new MagAdminApiClient(settings.Listen, settings.Secret);
        }

        /// <summary>One signed request. Returns the status and the parsed JSON body (null when empty or not JSON).</summary>
        public (int Status, JsonElement? Body) Send(string method, string path, byte[] body, string actor, DateTime now)
        {
            body ??= Array.Empty<byte>();
            string timestamp = new DateTimeOffset(now.ToUniversalTime()).ToUnixTimeSeconds().ToString();
            string nonce = Convert.ToHexString(System.Security.Cryptography.RandomNumberGenerator.GetBytes(16)).ToLowerInvariant();
            string signature = MagAdminApiAuthenticator.Sign(_secret, method, path, timestamp, nonce, body);
            var head = new StringBuilder()
                .Append(method).Append(' ').Append(path).Append(" HTTP/1.1\r\n")
                .Append("Host: ").Append(_endpoint).Append("\r\n")
                .Append(MagAdminApiAuthenticator.TimestampHeader).Append(": ").Append(timestamp).Append("\r\n")
                .Append(MagAdminApiAuthenticator.NonceHeader).Append(": ").Append(nonce).Append("\r\n")
                .Append(MagAdminApiAuthenticator.ActorHeader).Append(": ").Append(actor).Append("\r\n")
                .Append(MagAdminApiAuthenticator.SignatureHeader).Append(": ").Append(signature).Append("\r\n")
                .Append("Content-Type: application/json\r\n")
                .Append("Content-Length: ").Append(body.Length).Append("\r\n")
                .Append("Connection: close\r\n\r\n");
            using var cancel = new CancellationTokenSource(Deadline);
            using var client = new TcpClient(_endpoint.AddressFamily);
            client.ConnectAsync(_endpoint, cancel.Token).AsTask().GetAwaiter().GetResult();
            using var stream = client.GetStream();
            stream.WriteAsync(Encoding.ASCII.GetBytes(head.ToString()), cancel.Token).AsTask().GetAwaiter().GetResult();
            stream.WriteAsync(body, cancel.Token).AsTask().GetAwaiter().GetResult();
            using var received = new MemoryStream();
            var buffer = new byte[8192];
            while (true)
            {
                int read = stream.ReadAsync(buffer, cancel.Token).AsTask().GetAwaiter().GetResult();
                if (read == 0) break;
                if (received.Length + read > MaximumResponseBytes) throw new InvalidDataException("The admin API response is too large.");
                received.Write(buffer, 0, read);
            }
            var bytes = received.ToArray();
            int end = bytes.AsSpan().IndexOf("\r\n\r\n"u8);
            if (end < 0) throw new InvalidDataException("The admin API response has no header.");
            var status = Encoding.ASCII.GetString(bytes, 0, end).Split("\r\n")[0].Split(' ');
            if (status.Length < 2 || !status[0].StartsWith("HTTP/1.", StringComparison.Ordinal) || !int.TryParse(status[1], out int code))
                throw new InvalidDataException("The admin API response has no status.");
            var content = bytes.AsMemory(end + 4);
            if (content.Length == 0) return (code, null);
            try
            {
                using var document = JsonDocument.Parse(content);
                return (code, document.RootElement.Clone());
            }
            catch (JsonException) { return (code, null); }
        }
    }
}
