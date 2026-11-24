using System;
using System.Globalization;
using System.Threading;

namespace Server.Medius
{
    /// <summary>
    /// Concurrent DME humans across every world of this process. The integrated
    /// DME gives each connected human its own UDP port, the lowest free one from
    /// <see cref="FirstPort"/> upward, so the number of humans that can actually
    /// exchange game traffic is the number of forwarded ports. Every gameplay
    /// reservation holds one lease from admission until its transport has stopped;
    /// admission beyond <see cref="Capacity"/> is refused before a DME client exists.
    /// </summary>
    public sealed class MagDmePortBudget
    {
        /// <summary>The integrated DME always hands out UDP ports from here upward.</summary>
        public const int FirstPort = 10474;
        public const string PortEndVariable = "MAG_DME_UDP_PORT_END";
        public const string CapacityVariable = "MAG_DME_HUMAN_CAPACITY";

        readonly object _gate = new();
        int _held;

        /// <summary>No configured range: the DME keeps its open-ended allocation.</summary>
        public static MagDmePortBudget Unbounded() => new(null, null);

        /// <param name="lastPort">Last forwarded UDP port, or null for no configured range.</param>
        /// <param name="capacity">Admission cap; null uses the range size.</param>
        public MagDmePortBudget(int? lastPort, int? capacity)
        {
            if (lastPort is int last && (last < FirstPort || last > ushort.MaxValue))
                throw new ArgumentException($"{PortEndVariable} must be a UDP port from {FirstPort} to {ushort.MaxValue}.");
            int? rangeSize = lastPort is int end ? end - FirstPort + 1 : null;
            if (capacity is int cap && (cap < 1 || cap > (rangeSize ?? ushort.MaxValue - FirstPort + 1)))
                throw new ArgumentException(rangeSize is int size
                    ? $"{CapacityVariable} must be from 1 to the {size} ports in {FirstPort}-{lastPort}."
                    : $"{CapacityVariable} must be from 1 to {ushort.MaxValue - FirstPort + 1}.");
            Capacity = capacity ?? rangeSize;
            // An override alone also bounds the ports the DME hands out.
            LastPort = lastPort ?? (capacity is int only ? FirstPort + only - 1 : null);
            Source = capacity != null ? "override" : lastPort != null ? "port_range" : "unbounded";
        }

        public static MagDmePortBudget FromEnvironment() => Load(Environment.GetEnvironmentVariable);

        public static MagDmePortBudget Load(Func<string, string> read)
        {
            ArgumentNullException.ThrowIfNull(read);
            return new MagDmePortBudget(Parse(read(PortEndVariable), PortEndVariable), Parse(read(CapacityVariable), CapacityVariable));
        }

        static int? Parse(string raw, string name)
        {
            if (raw == null) return null;
            if (!int.TryParse(raw, NumberStyles.None, CultureInfo.InvariantCulture, out int value))
                throw new ArgumentException(name + " requires a decimal integer.");
            return value;
        }

        /// <summary>Most humans admitted at once, or null when unbounded.</summary>
        public int? Capacity { get; }
        /// <summary>Last UDP port the DME may hand out, or null when unbounded.</summary>
        public int? LastPort { get; }
        /// <summary>Where the cap comes from: port_range, override or unbounded.</summary>
        public string Source { get; }
        public bool Bounded => Capacity.HasValue;
        public int InUse { get { lock (_gate) return _held; } }
        /// <summary>Free leases, or int.MaxValue when unbounded.</summary>
        public int Available { get { lock (_gate) return Capacity is int cap ? Math.Max(0, cap - _held) : int.MaxValue; } }

        /// <summary>Acquire <paramref name="count"/> leases together, or none.</summary>
        public Lease[] TryAcquire(int count)
        {
            if (count < 1) throw new ArgumentOutOfRangeException(nameof(count));
            lock (_gate)
            {
                if (Capacity is int cap && _held + count > cap) return null;
                _held += count;
            }
            var leases = new Lease[count];
            for (int i = 0; i < count; i++) leases[i] = new Lease(this);
            return leases;
        }

        public Lease TryAcquire() => TryAcquire(1)?[0];

        void Return()
        {
            lock (_gate)
            {
                if (_held <= 0) throw new InvalidOperationException("DME port lease released twice.");
                _held--;
            }
        }

        public string Describe() => Capacity is int cap
            ? $"in_use={InUse} capacity={cap} ports={FirstPort}-{LastPort} source={Source}"
            : $"in_use={InUse} capacity=unbounded ports={FirstPort}+ source={Source}";

        /// <summary>One human's port; disposing returns it once.</summary>
        public sealed class Lease : IDisposable
        {
            MagDmePortBudget _owner;
            internal Lease(MagDmePortBudget owner) => _owner = owner;
            public bool Held => Volatile.Read(ref _owner) != null;
            public void Dispose() => Interlocked.Exchange(ref _owner, null)?.Return();
        }
    }
}
