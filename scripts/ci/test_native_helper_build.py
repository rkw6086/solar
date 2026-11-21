"""Keep native compiler diagnostics visible without needing a macOS runner."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(shutil.which("rustc"), "Rust compiler is needed for the build-script fixture")
class NativeHelperDiagnosticsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="solar-native-build-test-")
        cls.root = Path(cls.directory.name)
        cls.binary = cls.root / "build-script"
        subprocess.run(
            ["rustc", "--edition=2024", str(ROOT / "crates/solar-tools/build.rs"), "-o", str(cls.binary)],
            check=True, capture_output=True, text=True,
        )
        compiler = cls.root / "xcrun"
        compiler.write_text(
            f"#!{sys.executable}\n"
            "import os, pathlib, sys\n"
            "assert sys.argv[1] == 'swiftc'\n"
            "print(os.environ.get('TEST_COMPILER_STDOUT', ''), end='')\n"
            "print(os.environ.get('TEST_COMPILER_STDERR', ''), end='', file=sys.stderr)\n"
            "if os.environ.get('TEST_COMPILER_FAIL'): sys.exit(1)\n"
            "pathlib.Path(sys.argv[sys.argv.index('-o') + 1]).write_text('fixture')\n"
        )
        compiler.chmod(0o700)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def build(self, **variables):
        environment = {
            **os.environ,
            "PATH": str(self.root) + os.pathsep + os.environ.get("PATH", ""),
            "CARGO_CFG_TARGET_OS": "macos",
            "CARGO_CFG_TARGET_ARCH": "aarch64",
            "OUT_DIR": str(self.root),
            **variables,
        }
        return subprocess.run([str(self.binary)], env=environment, capture_output=True, text=True)

    def test_success_preserves_compiler_warnings_and_helper_metadata(self):
        result = self.build(TEST_COMPILER_STDERR="source.swift:2: warning: fixture warning\n  sample()\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cargo:warning=source.swift:2: warning: fixture warning", result.stdout)
        self.assertIn("cargo:warning=  sample()", result.stdout)
        self.assertIn("cargo:rustc-env=SOLAR_NATIVE_HELPER_BUILT=", result.stdout)
        self.assertTrue((self.root / "solar-native-helper").exists())

    def test_failure_reports_compiler_stdout_and_stderr(self):
        result = self.build(
            TEST_COMPILER_STDOUT="compiler context",
            TEST_COMPILER_STDERR="source.swift:4: error: fixture error\n",
            TEST_COMPILER_FAIL="1",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stdout:\ncompiler context", result.stderr)
        self.assertIn("stderr:\nsource.swift:4: error: fixture error", result.stderr)
        self.assertNotIn("cargo:metadata=native_helper=", result.stdout)


if __name__ == "__main__":
    unittest.main()
