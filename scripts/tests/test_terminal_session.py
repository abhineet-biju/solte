import sys
from pathlib import Path
import tempfile
import unittest

import pyte

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from terminal_session import TerminalSession


class TerminalSessionTests(unittest.TestCase):
    def test_fragmented_cursor_updates_preserve_visible_text_and_unicode(self):
        screen = pyte.Screen(40, 6)
        stream = pyte.ByteStream(screen)
        stream.feed(b"\x1b[2;1HMint ")
        redraw = b"\x1b[2;6Hinspec\x1b[2;12Htor\x1b[3;1H\xe2\x86\x93 More below"
        for byte in redraw:
            stream.feed(bytes([byte]))
        self.assertNotIn(b"Mint inspector", redraw)
        self.assertEqual(screen.display[1].strip(), "Mint inspector")
        self.assertEqual(screen.display[2].strip(), "\u2193 More below")
        stream.feed(b"\x1b[2;1H\x1b[2KProject mints")
        self.assertEqual(screen.display[1].strip(), "Project mints")

    def test_waits_for_a_delayed_incremental_redraw(self):
        child = """
import os, time, tty
tty.setraw(0)
os.write(1, b'\\x1b[2;1HMint \\x1b[?25l')
os.read(0, 1)
time.sleep(0.45)
os.write(1, b'\\x1b[2;6Hinspec')
time.sleep(0.05)
os.write(1, b'\\x1b[2;12Htor\\x1b[?25l')
os.read(0, 1)
"""
        with TerminalSession([sys.executable, "-u", "-c", child], 40, 6) as terminal:
            terminal.wait_text("Mint ")
            terminal.send(b"m", "Mint inspector")
            self.assertNotIn(b"Mint inspector", terminal.capture)
            terminal.send(b"q")
            terminal.process.wait(timeout=5)
            self.assertEqual(terminal.process.returncode, 0)

    def test_timeout_saves_the_visible_screen_for_diagnosis(self):
        child = "import os, time; os.write(1, b'Ready'); time.sleep(30)"
        with tempfile.TemporaryDirectory() as temporary:
            with TerminalSession([sys.executable, "-u", "-c", child], 40, 6, artifact_dir=temporary) as terminal:
                terminal.wait_text("Ready")
                with self.assertRaisesRegex(AssertionError, "Visible screen:"):
                    terminal.wait_text("Missing", timeout=0.1)
                self.assertIn("Ready", (Path(temporary) / "screen.txt").read_text())
                self.assertIn(b"Ready", (Path(temporary) / "terminal.ansi").read_bytes())


if __name__ == "__main__":
    unittest.main()
