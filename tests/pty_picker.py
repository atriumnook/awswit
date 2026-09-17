#!/usr/bin/env python3
"""Drive awswit on a PTY with stdout captured as a pipe.

This is the `$()` / `$(awswit pick)` shape: stdin and stderr are a TTY,
stdout is not. The historical bug gated the picker on stdout being a TTY
and rendered the TUI there, so the wrapper never showed a picker and
`$(awswit pick)` captured escape codes.

Modes:
  wait   — assert the picker stays up (does not fall through to `default`)
  enter  — select the first entry and capture stdout
  esc    — cancel (expect exit 130, empty stdout)
  pick   — `awswit pick`, Enter, stdout is the profile name only
"""

from __future__ import annotations

import errno
import fcntl
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time


def set_winsize(fd: int, rows: int = 24, cols: int = 80) -> None:
    winsz = struct.pack("HHHH", rows, cols, 0, 0)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, winsz)


def read_available(fd: int, buf: bytearray, timeout: float) -> bool:
    r, _, _ = select.select([fd], [], [], timeout)
    if fd not in r:
        return False
    try:
        chunk = os.read(fd, 8192)
    except OSError as exc:
        if exc.errno == errno.EIO:
            return False
        raise
    if not chunk:
        return False
    buf.extend(chunk)
    return True


def looks_like_tui(data: bytes) -> bool:
    lower = data.lower()
    if b"awswit" in lower:
        return True
    if b"profiles" in lower:
        return True
    # Alternate-screen / CSI traffic from ratatui.
    if b"\x1b[" in data or b"\x1b(B" in data:
        return True
    return False


def emit_result(
    tui_seen: bool, alive: bool, exit_code: int, stdout: bytes, pty_buf: bytes
) -> None:
    sys.stdout.buffer.write(
        (
            f"tui={1 if tui_seen else 0}\n"
            f"alive={1 if alive else 0}\n"
            f"exit={exit_code}\n"
        ).encode()
    )
    sys.stdout.buffer.write(b"---STDOUT---\n")
    sys.stdout.buffer.write(stdout or b"")
    sys.stdout.buffer.write(b"\n---PTY---\n")
    sys.stdout.buffer.write(pty_buf)
    sys.stdout.buffer.flush()


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: pty_picker.py MODE BIN [args...]", file=sys.stderr)
        return 2
    mode = sys.argv[1]
    argv = sys.argv[2:]

    master, slave = pty.openpty()
    set_winsize(master)
    set_winsize(slave)

    env = os.environ.copy()
    env.setdefault("TERM", "xterm-256color")
    env.setdefault("AWSWIT_SHELL", "bash")

    proc = subprocess.Popen(
        argv,
        stdin=slave,
        stdout=subprocess.PIPE,
        stderr=slave,
        env=env,
        close_fds=True,
    )
    os.close(slave)

    flags = fcntl.fcntl(master, fcntl.F_GETFL)
    fcntl.fcntl(master, fcntl.F_SETFL, flags | os.O_NONBLOCK)

    pty_buf = bytearray()
    tui_seen = False
    deadline = time.time() + 4.0
    sent_key = False

    while time.time() < deadline:
        read_available(master, pty_buf, 0.1)
        if not tui_seen and looks_like_tui(bytes(pty_buf)):
            tui_seen = True
            if mode == "wait":
                break
            if mode in ("enter", "pick") and not sent_key:
                os.write(master, b"\r")
                sent_key = True
            elif mode == "esc" and not sent_key:
                os.write(master, b"\x1b")
                sent_key = True
        if proc.poll() is not None:
            break

    if mode == "wait":
        still_running = proc.poll() is None
        if still_running:
            proc.kill()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
        stdout = proc.stdout.read() if proc.stdout else b""
        while read_available(master, pty_buf, 0.05):
            pass
        os.close(master)
        emit_result(
            tui_seen,
            still_running,
            proc.returncode if proc.returncode is not None else -1,
            stdout,
            bytes(pty_buf),
        )
        if still_running and tui_seen and not stdout.strip():
            return 0
        return 1

    try:
        stdout, _ = proc.communicate(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        stdout, _ = proc.communicate()
    while read_available(master, pty_buf, 0.05):
        pass
    os.close(master)
    emit_result(
        tui_seen,
        False,
        proc.returncode if proc.returncode is not None else -1,
        stdout or b"",
        bytes(pty_buf),
    )
    return 0 if tui_seen else 1


if __name__ == "__main__":
    raise SystemExit(main())
