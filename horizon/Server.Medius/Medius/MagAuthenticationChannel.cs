using System;
using System.Threading;

namespace Server.Medius
{
    /// <summary>One immutable transport binding, shared by tick dispatch and owned worker completion.</summary>
    public sealed class MagAuthenticationChannelBinding
    {
        private int _application;
        public bool TryBind(int application) => application > 0 && Interlocked.CompareExchange(ref _application, application, 0) == 0;
        public bool Matches(int application) => application > 0 && Volatile.Read(ref _application) == application;
    }

    /// <summary>A worker's old request timestamp cannot move activity behind a newer tick echo.</summary>
    internal sealed class MagSessionActivity
    {
        private long _ticks;
        public void Observe(DateTime now)
        {
            long ticks = now.ToUniversalTime().Ticks, previous = Volatile.Read(ref _ticks);
            while (ticks > previous)
            {
                long actual = Interlocked.CompareExchange(ref _ticks, ticks, previous);
                if (actual == previous) return;
                previous = actual;
            }
        }
        public bool IsExpired(DateTime now, int seconds)
        {
            long ticks = Volatile.Read(ref _ticks);
            return ticks != 0 && now.ToUniversalTime().Ticks - ticks > seconds * TimeSpan.TicksPerSecond;
        }
    }
}
