using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace Server.Medius
{
    /// <summary>
    /// Bounded dispatch for synchronous authentication/store work. A channel owns
    /// its callbacks in admission order; different channels can run concurrently.
    /// Capacity includes callbacks already dispatched to a worker.
    /// </summary>
    public sealed class MagAuthenticationWorkQueue
    {
        sealed class Work
        {
            public Func<bool> IsLive;
            public Action Action;
            public Action<Exception> OnError;
        }

        sealed class Channel
        {
            public string Id;
            public int Count;
            public readonly Queue<Work> Pending = new Queue<Work>();
        }

        readonly object _gate = new object();
        readonly Dictionary<string, Channel> _channels = new Dictionary<string, Channel>(StringComparer.Ordinal);
        readonly Queue<Channel> _ready = new Queue<Channel>();
        readonly int _concurrency;
        readonly int _capacity;
        readonly int _perChannel;
        int _count;
        int _active;

        public MagAuthenticationWorkQueue(int concurrency = 4, int capacity = 128, int perChannel = 8)
        {
            if (concurrency < 1) throw new ArgumentOutOfRangeException(nameof(concurrency));
            if (capacity < 1) throw new ArgumentOutOfRangeException(nameof(capacity));
            if (perChannel < 1) throw new ArgumentOutOfRangeException(nameof(perChannel));
            _concurrency = concurrency;
            _capacity = capacity;
            _perChannel = perChannel;
        }

        /// <summary>
        /// Admits work without invoking any supplied callback on the caller.
        /// False means no work was accepted; the caller decides how to refuse it.
        /// </summary>
        public bool TryEnqueue(string channelId, Func<bool> isLive, Action action, Action<Exception> onError = null)
        {
            if (channelId == null) throw new ArgumentNullException(nameof(channelId));
            if (isLive == null) throw new ArgumentNullException(nameof(isLive));
            if (action == null) throw new ArgumentNullException(nameof(action));
            lock (_gate)
            {
                if (_count >= _capacity) return false;
                _channels.TryGetValue(channelId, out var channel);
                if (channel != null && channel.Count >= _perChannel) return false;
                if (channel == null)
                {
                    channel = new Channel { Id = channelId };
                    _channels.Add(channelId, channel);
                    _ready.Enqueue(channel);
                }
                channel.Pending.Enqueue(new Work { IsLive = isLive, Action = action, OnError = onError });
                channel.Count++;
                _count++;
                DispatchLocked();
                return true;
            }
        }

        // A channel is either ready once or has exactly one dispatched callback.
        // Reserving both the channel and worker under the gate prevents idle races.
        void DispatchLocked()
        {
            while (_active < _concurrency && _ready.Count != 0)
            {
                var channel = _ready.Dequeue();
                var work = channel.Pending.Dequeue();
                _active++;
                Task.Run(() => Execute(channel, work));
            }
        }

        void Execute(Channel channel, Work work)
        {
            try
            {
                if (work.IsLive()) work.Action();
            }
            catch (Exception error)
            {
                try { work.OnError?.Invoke(error); }
                catch (Exception) { /* A reporter cannot prevent queue cleanup. */ }
            }
            finally
            {
                lock (_gate)
                {
                    _active--;
                    _count--;
                    channel.Count--;
                    if (channel.Count == 0) _channels.Remove(channel.Id);
                    else _ready.Enqueue(channel);
                    DispatchLocked();
                }
            }
        }
    }
}
