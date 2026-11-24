"""Wait for configured gameplay workers before Medius starts.

Medius marks a worker it cannot reach at startup unavailable until Medius itself restarts, so the
Medius Pod waits until each worker Service accepts a TCP connection (a ClusterIP Service only does
once the worker is ready). Each check opens and closes a connection without sending a request; the
worker logs it as an incomplete request. After the time limit Medius starts anyway with the workers
that are reachable, so one broken worker cannot keep the lobby down.

Usage: wait_for_workers.py SECONDS HOST:PORT [HOST:PORT ...]
"""
import socket
import sys
import time


def wait(seconds, targets, connect=socket.create_connection, clock=time.monotonic, sleep=time.sleep):
    """Return the targets still unreachable when every target answered or the time ran out."""
    pending = list(targets)
    deadline = clock() + seconds
    while True:
        for target in list(pending):
            host, _, port = target.rpartition(':')
            try:
                connect((host, int(port)), timeout=2).close()
            except OSError:
                continue
            pending.remove(target)
            print(f'gameplay worker ready: {target}', flush=True)
        if not pending or clock() >= deadline:
            return pending
        sleep(2)


def main(argv):
    if len(argv) < 2 or not argv[0].isdecimal() or any(':' not in target for target in argv[1:]):
        raise SystemExit('usage: wait_for_workers.py SECONDS HOST:PORT [HOST:PORT ...]')
    missing = wait(int(argv[0]), argv[1:])
    if missing:
        print('starting Medius without unreachable gameplay workers: ' + ' '.join(missing), file=sys.stderr, flush=True)


if __name__ == '__main__':
    main(sys.argv[1:])
