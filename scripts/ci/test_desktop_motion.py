"""Run the companion clock regressions without building the whole desktop app."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(shutil.which("rustc"), "Rust is needed for the desktop motion regressions")
class DesktopMotionTests(unittest.TestCase):
    def test_desktop_motion_regressions(self):
        with tempfile.TemporaryDirectory(prefix="solar-motion-test-") as directory:
            executable = Path(directory) / "desktop-motion-tests"
            compilation = subprocess.run(
                ["rustc", "--edition=2024", "--test", "-D", "warnings",
                 str(ROOT / "src-tauri/src/desktop_motion.rs"), "-o", str(executable)],
                capture_output=True, text=True,
            )
            self.assertEqual(compilation.returncode, 0, compilation.stdout + compilation.stderr)
            tests = subprocess.run([str(executable)], capture_output=True, text=True)
            self.assertEqual(tests.returncode, 0, tests.stdout + tests.stderr)


if __name__ == "__main__":
    unittest.main()
