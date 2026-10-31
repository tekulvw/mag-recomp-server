using HpTimeStamps;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using MonotonicContext = HpTimeStamps.MonotonicStampContext;

namespace Server.Common
{
    using MonoStampSource = MonotonicTimeStampUtil<MonotonicContext>;

    public static class Utils
    {
        private static Stopwatch _swTicker = Stopwatch.StartNew();
        private static long _swTickerInitialTicks = DateTime.UtcNow.Ticks;

        public static byte[] ReverseEndian(byte[] ba)
        {
            byte[] ret = new byte[ba.Length];
            for (int i = 0; i < ba.Length; i += 4)
            {
                int max = i + 3;
                if (max >= ba.Length)
                    max = ba.Length - 1;

                for (int x = max; x >= i; x--)
                    ret[i + (max - x)] = ba[x];
            }
            return ret;
        }

        public static byte[] FromString(string str)
        {
            byte[] buffer = new byte[str.Length / 2];

            for (int i = 0; i < buffer.Length; ++i)
            {
                buffer[i] = byte.Parse(str.Substring(i * 2, 2), System.Globalization.NumberStyles.HexNumber);
            }

            return buffer;
        }

        public static byte[] FromStringFlipped(string str)
        {
            byte[] buffer = new byte[str.Length / 2];

            int strIndex = str.Length - 2;
            for (int i = 0; i < buffer.Length; ++i)
            {
                buffer[i] = byte.Parse(str.Substring(strIndex, 2), System.Globalization.NumberStyles.HexNumber);
                strIndex -= 2;
            }

            return buffer;
        }

        #region LINQ

        /// <summary>
        /// Emits consecutive groups. A candidate is accumulated before testing its
        /// boundary; a rejected candidate starts the next group with a fresh seed.
        /// The first item of every group is always retained, even if oversized.
        /// </summary>
        public static IEnumerable<IEnumerable<T>> GroupWhileAggregating<T, TAccume>(
            this IEnumerable<T> source,
            TAccume seed,
            Func<TAccume, T, TAccume> accumulator,
            Func<TAccume, T, bool> predicate)
        {
            var batch = new List<T>();
            var aggregate = seed;
            foreach (var item in source)
            {
                var candidate = accumulator(aggregate, item);
                if (batch.Count != 0 && !predicate(candidate, item))
                {
                    yield return batch;
                    batch = new List<T>();
                    candidate = accumulator(seed, item);
                }
                batch.Add(item);
                aggregate = candidate;
            }
            if (batch.Count != 0) yield return batch;
        }

        /// <summary>BCL minimum selection with the legacy empty-sequence exception.</summary>
        public static TSource MinBy<TSource, TKey>(this IEnumerable<TSource> source,
            Func<TSource, TKey> selector, IComparer<TKey> comparer)
        {
            ArgumentNullException.ThrowIfNull(source);
            ArgumentNullException.ThrowIfNull(selector);
            return Enumerable.MinBy(RequireElement(), selector, comparer);

            IEnumerable<TSource> RequireElement()
            {
                using var iterator = source.GetEnumerator();
                if (!iterator.MoveNext()) throw new InvalidOperationException("Sequence contains no elements.");
                do { yield return iterator.Current; } while (iterator.MoveNext());
            }
        }

        #endregion

        #region Time

        /// <summary>
        /// Monotonic stamp context used by monotonic clock.  Contains
        /// information about frequencies, conversions, a reference time, etc.
        /// </summary>
        public static ref readonly MonotonicContext MonotonicContext
        {
            [MethodImpl(MethodImplOptions.AggressiveInlining)]
            get => ref MonoStampSource.StampContext;
        }

        public static DateTime GetHighPrecisionUtcTime()
        {
            return new DateTime(_swTicker.Elapsed.Ticks + _swTickerInitialTicks, DateTimeKind.Utc);
        }

        public static long GetMillisecondsSinceStartup()
        {
            return _swTicker.ElapsedMilliseconds;
        }

        public static uint GetUnixTime()
        {
            return GetHighPrecisionUtcTime().ToUnixTime();
        }

        public static uint ToUnixTime(this DateTime time)
        {
            return (uint)(time.Subtract(new DateTime(1970, 1, 1))).TotalSeconds;
        }

        public static DateTime ToUtcDateTime(this uint unixTime)
        {
            return new DateTime(1970, 1, 1) + TimeSpan.FromSeconds(unixTime);
        }

#endregion

#region SHA-256

        public static string ComputeSHA256(string input)
        {
            // Create a SHA256   
            using (SHA256 sha256Hash = SHA256.Create())
            {
                // ComputeHash - returns byte array  
                byte[] bytes = sha256Hash.ComputeHash(Encoding.UTF8.GetBytes(input));

                // Convert byte array to a string   
                StringBuilder builder = new StringBuilder();
                for (int i = 0; i < bytes.Length; i++)
                    builder.Append(bytes[i].ToString("x2"));

                return builder.ToString();
            }
        }

        public static string ComputeSHA512(string input)
        {
            // Convert input string to a byte array
            byte[] inputBytes = Encoding.UTF8.GetBytes(input);

            // Compute hash value
            SHA512 sha512 = SHA512.Create();
            byte[] hashBytes = sha512.ComputeHash(inputBytes);

            // Convert hash bytes to a string
            StringBuilder sb = new StringBuilder();
            foreach (byte b in hashBytes)
            {
                sb.Append(b.ToString("x2"));
            }
            return sb.ToString();
        }

#endregion

#region Ip

        public static IPAddress GetIp(string hostname)
        {
            if (hostname == "localhost")
                return IPAddress.Loopback;

            switch (Uri.CheckHostName(hostname))
            {
                case UriHostNameType.IPv4: return IPAddress.Parse(hostname);
                case UriHostNameType.Dns: return Dns.GetHostAddresses(hostname).FirstOrDefault()?.MapToIPv4() ?? IPAddress.Any;
                default:
                    {
                        return null;
                    }
            }
        }

        /// <summary>Reads an operator-supplied IPv4 address without an external discovery request.</summary>
        public static string GetPublicIPAddress()
        {
            var configured = Environment.GetEnvironmentVariable("HORIZON_PUBLIC_IP");
            if (string.IsNullOrWhiteSpace(configured) ||
                !IPAddress.TryParse(configured, out var address) || address.AddressFamily != AddressFamily.InterNetwork)
                throw new InvalidOperationException("Set PublicIpOverride or HORIZON_PUBLIC_IP to an explicit IPv4 address when UsePublicIp is enabled.");
            return address.ToString();
        }

        public static IPAddress GetLocalIPAddress()
        {
            if (!System.Net.NetworkInformation.NetworkInterface.GetIsNetworkAvailable())
                return null;

            // Get all active interfaces
            var interfaces = NetworkInterface.GetAllNetworkInterfaces()
                .Where(c => c.NetworkInterfaceType != NetworkInterfaceType.Loopback && c.OperationalStatus == OperationalStatus.Up);

            // Find our local ip
            foreach (var i in interfaces)
            {
                var props = i.GetIPProperties();
                var inter = props.UnicastAddresses.Where(x => x.Address.AddressFamily == AddressFamily.InterNetwork);
                if (props.GatewayAddresses.Count > 0 && inter.Count() > 0)
                {
                    return inter.FirstOrDefault().Address;
                }
            }

            return null;
        }

        #endregion

        #region Tasks

        /// <summary>Waits using the BCL timeout policy; timeout does not cancel the operation.</summary>
        public static Task<TResult> TimeoutAfter<TResult>(this Task<TResult> task, TimeSpan timeout) =>
            task.WaitAsync(timeout);

        public static Task TimeoutAfter(this Task task, TimeSpan timeout) => task.WaitAsync(timeout);

        public static async Task<bool> TryAwait(this Task task, TimeSpan timeout)
        {
            // Wrap completion as a successful Task<Task> so an operation's own
            // TimeoutException remains distinct from expiration of this wait.
            var completion = task.ContinueWith(finished => finished, CancellationToken.None,
                TaskContinuationOptions.ExecuteSynchronously, TaskScheduler.Default);
            Task finished;
            try { finished = await completion.WaitAsync(timeout).ConfigureAwait(false); }
            catch (TimeoutException) { return false; }
            await finished.ConfigureAwait(false);
            return true;
        }

        #endregion

    }
}
