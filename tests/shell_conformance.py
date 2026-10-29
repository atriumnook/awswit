#!/usr/bin/env python3
"""End-to-end check of the documented shell integration in real shells.

For each shell this starts an interactive session on a PTY, loads
`awswit init <shell>` the way the README says, drives awswit with real
keystrokes, and then asks botocore (the AWS SDK for Python) which
credentials the resulting environment resolves to. The SDK is the
independent judge: "the next AWS call uses the profile the user picked"
is the outcome awswit exists for, and only the SDK's own resolution
chain can confirm it.

Usage: shell_conformance.py PATH/TO/awswit [bash] [zsh] [fish]

Requires the listed shells and `botocore`. Credentials are fake and no
network call is made. Synchronisation waits for observable state (the
completion marker each command prints, the picker's alternate-screen
sequences), bounded by TIMEOUT; there are no fixed sleeps.
"""

from __future__ import annotations

import fcntl
import os
import pty
import select
import shutil
import signal
import subprocess
import sys
import struct
import tempfile
import termios
import time

TIMEOUT = 15.0
ALT_ENTER = b"\x1b[?1049h"
ALT_LEAVE = b"\x1b[?1049l"

CONFIG = """\
[default]
region = us-east-1

[profile prod]
region = ap-northeast-1

[profile dev]
region = us-west-2
"""

CREDENTIALS = """\
[default]
aws_access_key_id = AKIADEFAULT
aws_secret_access_key = x

[prod]
aws_access_key_id = AKIAPROD
aws_secret_access_key = x

[dev]
aws_access_key_id = AKIADEV
aws_secret_access_key = x
"""

# Credentials a previous tool (aws-vault, awsume, an SSO export) left behind.
# They outrank AWS_PROFILE in every SDK's resolution chain.
STALE = {
    "AWS_ACCESS_KEY_ID": "AKIASTALE",
    "AWS_SECRET_ACCESS_KEY": "stale",
    "AWS_SESSION_TOKEN": "stale",
}

# Names that are legal in ~/.aws/config but need quoting in every shell.
TRICKY_NAMES = [
    "my profile",
    "a'b",
    "$(touch PWNED)",
    "`touch PWNED`",
    "x;touch PWNED",
    "back\\slash",
    "dbl\\\\slash",
    "star*",
    "日本語",
]

SHELLS = {
    "bash": {
        "argv": ["bash", "--norc", "--noprofile", "-i"],
        "setup": 'eval "$(awswit init bash)"',
        "dump": 'printf "rc=%s\\n" "$?" > {out}; env >> {out}',
        "named": [
            "bash", "--norc", "--noprofile", "-c",
            'eval "$(awswit init bash)"; awswit -n "$1"; printf "%s" "$AWS_PROFILE" > "$2"',
            "sh",
        ],
    },
    "zsh": {
        "argv": ["zsh", "-f", "-i"],
        "setup": 'autoload -Uz compinit; compinit -u; eval "$(awswit init zsh)"',
        "dump": 'printf "rc=%s\\n" "$?" > {out}; env >> {out}',
        "named": [
            "zsh", "-f", "-c",
            'compdef() { :; }; eval "$(awswit init zsh)"; awswit -n "$1"; printf "%s" "$AWS_PROFILE" > "$2"',
            "zsh",
        ],
    },
    "fish": {
        "argv": ["fish", "--no-config", "-i"],
        "setup": "awswit init fish | source",
        "dump": 'printf "rc=%s\\n" $status > {out}; env >> {out}',
        "named": [
            "fish", "--no-config", "-c",
            'awswit init fish | source; awswit -n $argv[1]; printf "%s" "$AWS_PROFILE" > $argv[2]',
            "--",
        ],
    },
}


class Failure(Exception):
    pass


def make_home() -> str:
    home = tempfile.mkdtemp(prefix="awswit-conformance-")
    os.makedirs(os.path.join(home, ".aws"))
    with open(os.path.join(home, ".aws", "config"), "w") as f:
        f.write(CONFIG)
    with open(os.path.join(home, ".aws", "credentials"), "w") as f:
        f.write(CREDENTIALS)
    return home


def base_env(home: str, bindir: str) -> dict[str, str]:
    return {
        "HOME": home,
        "PATH": bindir + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "XDG_CONFIG_HOME": os.path.join(home, ".config"),
        "XDG_DATA_HOME": os.path.join(home, ".local", "share"),
    }


class Session:
    """An interactive shell on a PTY, synchronised on observable output."""

    def __init__(self, shell: str, home: str, env: dict[str, str]):
        self.spec = SHELLS[shell]
        self.home = home
        self.buf = bytearray()
        self.pos = 0
        self.seq = 0
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(home)
            os.execvpe(self.spec["argv"][0], self.spec["argv"], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 200, 0, 0))
        self.send(self.spec["setup"])
        self.wait_done()

    def send(self, line: str) -> None:
        """Type `line`, followed by a command that prints a completion marker.

        The marker is assembled by printf, so the echoed command line never
        contains it and a prompt redraw cannot be mistaken for completion.
        """
        self.seq += 1
        os.write(self.fd, f"{line}; printf 'AWSWIT-DONE-%s\\n' {self.seq}\r".encode())

    def wait_done(self) -> None:
        self.wait_for(f"AWSWIT-DONE-{self.seq}".encode())

    def keys(self, data: bytes) -> None:
        os.write(self.fd, data)

    def wait_for(self, needle: bytes) -> None:
        """Consume output until `needle` appears after the last match."""
        deadline = time.monotonic() + TIMEOUT
        while True:
            idx = self.buf.find(needle, self.pos)
            if idx >= 0:
                self.pos = idx + len(needle)
                return
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                tail = bytes(self.buf[-600:]).decode(errors="replace")
                raise Failure(f"timed out waiting for {needle!r}; output tail:\n{tail}")
            ready, _, _ = select.select([self.fd], [], [], remaining)
            if ready:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    chunk = b""
                if not chunk:
                    raise Failure(f"shell exited while waiting for {needle!r}")
                self.buf.extend(chunk)

    def run_and_dump(self, command: str, interact=None) -> tuple[int, dict[str, str]]:
        """Run `command`, optionally drive the picker, return ($?, AWS_* env)."""
        out = os.path.join(self.home, "dump.txt")
        if os.path.exists(out):
            os.remove(out)
        self.send(command + "; " + self.spec["dump"].format(out=out))
        if interact is not None:
            interact(self)
        self.wait_done()
        with open(out) as f:
            lines = f.read().splitlines()
        rc = int(lines[0].split("=", 1)[1])
        env = dict(l.split("=", 1) for l in lines[1:] if l.startswith("AWS_"))
        return rc, env

    def close(self) -> None:
        """Kill the session; a failed scenario may leave the picker running."""
        try:
            os.killpg(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        os.waitpid(self.pid, 0)
        os.close(self.fd)


def picker(*keys: bytes):
    """Wait for the picker to take the screen, type `keys`, wait for it to leave."""

    def drive(s: Session) -> None:
        s.wait_for(ALT_ENTER)
        for k in keys:
            s.keys(k)
        s.wait_for(ALT_LEAVE)

    return drive


def sdk_access_key(home: str, aws_env: dict[str, str]) -> str:
    """Ask botocore which access key this environment resolves to."""
    code = (
        "import botocore.session\n"
        "c = botocore.session.get_session().get_credentials()\n"
        "print(c.access_key if c else 'NONE')\n"
    )
    env = {"HOME": home, "PATH": os.environ.get("PATH", ""), **aws_env}
    r = subprocess.run(
        [sys.executable, "-c", code], env=env, capture_output=True, text=True, timeout=60
    )
    if r.returncode != 0:
        raise Failure(f"botocore failed: {r.stderr}")
    return r.stdout.strip()


def expect(cond: bool, msg: str) -> None:
    if not cond:
        raise Failure(msg)


def no_stale(env: dict[str, str]) -> None:
    left = sorted(set(STALE) & set(env))
    expect(not left, f"stale credentials survived the switch: {left}")


def scenario_picker_switch(shell, home, env):
    s = Session(shell, home, {**env, **STALE})
    try:
        rc, aws = s.run_and_dump("awswit", picker(b"prod", b"\r"))
    finally:
        s.close()
    expect(rc == 0, f"awswit exited {rc}")
    expect(aws.get("AWS_PROFILE") == "prod", f"AWS_PROFILE={aws.get('AWS_PROFILE')!r}")
    expect(aws.get("AWS_REGION") == "ap-northeast-1", f"AWS_REGION={aws.get('AWS_REGION')!r}")
    no_stale(aws)
    key = sdk_access_key(home, aws)
    expect(key == "AKIAPROD", f"botocore resolved {key}, expected the prod profile's key")


def scenario_picker_cancel(shell, home, env):
    s = Session(shell, home, {**env, **STALE, "AWS_PROFILE": "dev"})
    try:
        rc, aws = s.run_and_dump("awswit", picker(b"\x1b"))
    finally:
        s.close()
    expect(rc == 130, f"cancel should exit 130, got {rc}")
    expect(aws.get("AWS_PROFILE") == "dev", f"cancel changed AWS_PROFILE to {aws.get('AWS_PROFILE')!r}")
    expect(aws.get("AWS_ACCESS_KEY_ID") == "AKIASTALE", "cancel must leave the environment untouched")


def scenario_unknown_name_confirms(shell, home, env):
    s = Session(shell, home, env)
    try:
        # `prd` is a typo for prod; the picker opens seeded with it and the
        # user confirms. Nothing is switched without that Enter.
        rc, aws = s.run_and_dump("awswit prd", picker(b"\r"))
    finally:
        s.close()
    expect(rc == 0, f"awswit exited {rc}")
    expect(aws.get("AWS_PROFILE") == "prod", f"AWS_PROFILE={aws.get('AWS_PROFILE')!r}")


def scenario_unset(shell, home, env):
    s = Session(shell, home, {**env, **STALE, "AWS_PROFILE": "dev", "AWS_REGION": "us-west-2"})
    try:
        rc, aws = s.run_and_dump("awswit -u")
    finally:
        s.close()
    expect(rc == 0, f"awswit -u exited {rc}")
    left = sorted(set(aws) & ({"AWS_PROFILE", "AWS_REGION"} | set(STALE)))
    expect(not left, f"awswit -u left {left}")


def scenario_named_quoting(shell, home, env):
    bad = []
    for name in TRICKY_NAMES:
        h = tempfile.mkdtemp(prefix="awswit-names-")
        os.makedirs(os.path.join(h, ".aws"))
        with open(os.path.join(h, ".aws", "config"), "w") as f:
            f.write(f"[profile {name}]\nregion = us-east-1\n")
        out = os.path.join(h, "out")
        e = {**env, "HOME": h}
        subprocess.run(
            SHELLS[shell]["named"] + [name, out],
            env=e, cwd=h, capture_output=True, timeout=TIMEOUT,
        )
        got = open(out, encoding="utf-8").read() if os.path.exists(out) else None
        pwned = [f for f in os.listdir(h) if f.startswith("PWNED")]
        if got != name or pwned:
            bad.append(f"{name!r} -> {got!r}{' (command executed!)' if pwned else ''}")
        shutil.rmtree(h, ignore_errors=True)
    expect(not bad, "named switch mangled profile names: " + "; ".join(bad))


SCENARIOS = [
    scenario_picker_switch,
    scenario_picker_cancel,
    scenario_unknown_name_confirms,
    scenario_unset,
    scenario_named_quoting,
]


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    binary = os.path.abspath(sys.argv[1])
    shells = sys.argv[2:] or list(SHELLS)
    missing = [s for s in shells if shutil.which(s) is None]
    if missing:
        print(f"missing shells: {missing}", file=sys.stderr)
        return 2
    try:
        import botocore  # noqa: F401
    except ImportError:
        print("botocore is required (pip install botocore)", file=sys.stderr)
        return 2

    failed = 0
    for shell in shells:
        for scenario in SCENARIOS:
            home = make_home()
            env = base_env(home, os.path.dirname(binary))
            label = f"{shell}: {scenario.__name__.removeprefix('scenario_')}"
            try:
                scenario(shell, home, env)
                print(f"ok   {label}")
            except Failure as e:
                failed += 1
                print(f"FAIL {label}\n     {e}")
            finally:
                shutil.rmtree(home, ignore_errors=True)
    print(f"{failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
