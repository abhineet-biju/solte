"""Read a PTY as a terminal screen, including incremental and fragmented redraws."""
import base64
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import termios
import time

import pyte


class TerminalSession:
    def __init__(self, command, columns, rows, env=None, artifact_dir=None):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        self.screen = pyte.Screen(columns, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.output = bytearray()
        self.capture_start = 0
        self.artifact_dir = artifact_dir
        self.process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                        env=dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor", **(env or {})))
        os.close(slave)

    @property
    def text(self):
        return "\n".join(self.screen.display)

    @property
    def capture(self):
        return self.output[self.capture_start:]

    def read(self, timeout=0.05):
        if not select.select([self.master], [], [], timeout)[0]:
            return False
        try:
            chunk = os.read(self.master, 65536)
        except OSError:
            return False
        if not chunk:
            return False
        self.output.extend(chunk)
        self.stream.feed(chunk)
        return True

    def wait_for(self, predicate, description, timeout=15):
        deadline = time.monotonic() + timeout
        while True:
            if predicate():
                return
            if self.process.poll() is not None:
                self.read(0)
                if predicate():
                    return
                self.fail(f"Application exited with {self.process.returncode} while waiting for {description}")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self.fail(f"Timed out waiting for {description}")
            self.read(min(0.05, remaining))

    def wait_text(self, text, timeout=15):
        self.wait_for(lambda: text in self.text, repr(text), timeout)

    def send(self, data, expected=None):
        while select.select([self.master], [], [], 0)[0]:
            if not self.read(0):
                break
        if self.process.poll() is not None:
            self.fail(f"Application exited with {self.process.returncode} before input")
        self.capture_start = len(self.output)
        os.write(self.master, data)
        # Crossterm hides the cursor at the end of each fullscreen draw.
        self.wait_for(lambda: b"\x1b[?25l" in self.capture or self.process.poll() == 0, "completed terminal redraw")
        if expected is not None:
            self.wait_text(expected)

    def escape(self, title):
        self.send(b"\x1b")
        self.wait_for(lambda: title not in self.text, f"closed {title!r}")

    def resize(self, columns, rows):
        while select.select([self.master], [], [], 0)[0]:
            if not self.read(0):
                break
        self.capture_start = len(self.output)
        self.screen.resize(lines=rows, columns=columns)
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        os.kill(self.process.pid, signal.SIGWINCH)
        self.wait_for(lambda: b"\x1b[?25l" in self.capture, f"redraw at {columns}x{rows}")

    def click_text(self, text, expected=None, max_column=None):
        def location():
            for row, line in enumerate(self.screen.display):
                column = line.find(text)
                if column >= 0 and (max_column is None or column < max_column):
                    return column, row
            return None
        self.wait_for(lambda: location() is not None, f"click target {text!r}")
        column, row = location()
        self.send(f"\x1b[<0;{column + 1};{row + 1}M".encode(), expected)

    def clipboard(self):
        def values():
            return re.findall(rb"\x1b\]52;[^;]*;([A-Za-z0-9+/=]+)", self.capture)
        self.wait_for(lambda: bool(values()), "OSC 52 clipboard request")
        return base64.b64decode(values()[-1]).decode()

    def save_artifacts(self):
        if self.artifact_dir:
            directory = Path(self.artifact_dir)
            directory.mkdir(parents=True, exist_ok=True)
            (directory / "screen.txt").write_text(self.text)
            (directory / "terminal.ansi").write_bytes(self.output)

    def fail(self, message):
        self.save_artifacts()
        raise AssertionError(f"{message}\nVisible screen:\n{self.text}")

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        os.close(self.master)

    def __enter__(self):
        return self

    def __exit__(self, exception_type, *_):
        try:
            if exception_type is not None:
                self.save_artifacts()
        finally:
            self.close()
