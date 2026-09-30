#!/usr/bin/env python3
"""Send HMP commands to a QEMU `-monitor unix:` socket.

Used by `tools/vng-shot.sh` to press yserver's Ctrl+Alt+Enter scanout-dump
hotkey inside a headless guest: the emulated PS/2 keyboard is a real evdev
device to the guest, so the keys travel yserver's actual input path rather
than a test-only back door.

The monitor echoes every character back as it would to a human at a
terminal, so replies are dropped unless -v is passed.

    tools/qemu-monitor.py [-v] [--wait SECONDS] SOCKET CMD [CMD ...]
"""

import argparse
import os
import socket
import sys
import time


def connect(path: str, deadline: float) -> socket.socket:
    while True:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.connect(path)
            return sock
        except OSError:
            sock.close()
            if time.monotonic() >= deadline:
                sys.exit(f"qemu-monitor: could not connect to {path}")
            time.sleep(0.25)


def drain(sock: socket.socket) -> str:
    out = b""
    try:
        while True:
            chunk = sock.recv(4096)
            if not chunk:
                break
            out += chunk
    except TimeoutError:
        pass
    return out.decode(errors="replace")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("-v", "--verbose", action="store_true")
    parser.add_argument("--wait", type=float, default=30.0)
    parser.add_argument("socket")
    parser.add_argument("command", nargs="+")
    args = parser.parse_args()

    # A symlink keeps long artifact paths clear of the 107-byte sun_path limit.
    sock = connect(os.path.realpath(args.socket), time.monotonic() + args.wait)
    sock.settimeout(2)
    drain(sock)
    for command in args.command:
        sock.sendall(command.encode() + b"\n")
        reply = drain(sock)
        if args.verbose:
            print(f"$ {command}\n{reply}", flush=True)


if __name__ == "__main__":
    main()
