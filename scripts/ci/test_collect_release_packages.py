"""Release collection must reject incomplete or mismatched platform artifacts."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from release_packages import PackageError, package_names

SPEC = importlib.util.spec_from_file_location("collect_release_packages", Path(__file__).with_name("collect-release-packages.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CollectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.env = {"SOLAR_RELEASE_DIR": str(self.root), "SOLAR_RELEASE_VERSION": "0.1.8",
                    "SOLAR_RELEASE_SHA": "a" * 40, "SOLAR_BUILD_CHANNEL": "release"}
        self.populate()

    def tearDown(self):
        self.temp.cleanup()

    def populate(self):
        identity = "beta_" + self.env["SOLAR_RELEASE_SHA"] if self.env["SOLAR_BUILD_CHANNEL"] == "beta" else self.env["SOLAR_RELEASE_VERSION"]
        self.names = package_names(identity)
        self.groups = (("macos", "aarch64-apple-darwin", True, self.names[:1]),
                       ("windows", "x86_64-pc-windows-msvc", False, self.names[1:2]),
                       ("linux", "x86_64-unknown-linux-gnu", False, self.names[2:]))
        for folder, target, signed, packages in self.groups:
            directory = self.root / folder
            directory.mkdir(exist_ok=True)
            lines = []
            for name in packages:
                data = ("package: " + name).encode()
                (directory / name).write_bytes(data)
                lines.append(hashlib.sha256(data).hexdigest() + "  " + name + "\n")
            (directory / "SHA256SUMS").write_text("".join(lines))
            (directory / "BUILD-INFO.json").write_text(json.dumps({
                "version": self.env["SOLAR_RELEASE_VERSION"], "source": self.env["SOLAR_RELEASE_SHA"],
                "channel": self.env["SOLAR_BUILD_CHANNEL"], "target": target,
                "signed": signed, "packages": list(packages),
            }))

    def test_complete_release_and_idempotent_retry(self):
        self.assertEqual(MODULE.collect(self.env), self.names)
        self.assertEqual(MODULE.collect(self.env), self.names)
        self.assertEqual(len((self.root / "SHA256SUMS").read_text().splitlines()), 4)
        for name in self.names:
            self.assertTrue((self.root / name).is_file())

    def test_beta_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            self.root = Path(directory)
            self.env.update(SOLAR_RELEASE_DIR=directory, SOLAR_BUILD_CHANNEL="beta", SOLAR_BETA_ID="a" * 40)
            self.populate()
            MODULE.collect(self.env)
            self.assertTrue(all("beta_" + "a" * 40 in name for name in self.names))
            with self.assertRaises(PackageError):
                MODULE.collect({**self.env, "SOLAR_BETA_ID": "b" * 40})

    def test_missing_platform_is_rejected_before_output(self):
        (self.root / "linux" / self.names[3]).unlink()
        with self.assertRaises(PackageError):
            MODULE.collect(self.env)
        self.assertFalse((self.root / self.names[0]).exists())

    def test_metadata_identity_and_signed_status(self):
        path = self.root / "windows" / "BUILD-INFO.json"
        original = json.loads(path.read_text())
        for changes in ({"source": "b" * 40}, {"version": "0.1.9"}, {"channel": "preview"},
                        {"target": "aarch64-pc-windows-msvc"}, {"signed": True},
                        {"packages": ["unexpected.exe"]}, {"extra": "field"}):
            with self.subTest(changes=changes):
                path.write_text(json.dumps({**original, **changes}))
                with self.assertRaises(PackageError):
                    MODULE.collect(self.env)
                self.assertFalse((self.root / "SHA256SUMS").exists())
        path.write_text(json.dumps(original))

    def test_duplicate_metadata_keys_are_rejected(self):
        path = self.root / "linux" / "BUILD-INFO.json"
        path.write_text(path.read_text().replace('"version":', '"version":"0.1.8","version":'))
        with self.assertRaises(PackageError):
            MODULE.collect(self.env)

    def test_checksum_tampering_and_unsafe_names_are_rejected(self):
        path = self.root / "linux" / "SHA256SUMS"
        original = path.read_text()
        for text in (original.replace(original[:64], "0" * 64, 1), original + original,
                     original.replace(self.names[2], "../" + self.names[2]), original.splitlines()[0] + "\n"):
            with self.subTest(text=text):
                path.write_text(text)
                with self.assertRaises(PackageError):
                    MODULE.collect(self.env)
                self.assertFalse((self.root / "SHA256SUMS").exists())
        path.write_text(original)

    def test_unknown_artifact_is_rejected(self):
        (self.root / "windows" / "extra.exe").write_bytes(b"unknown")
        with self.assertRaises(PackageError):
            MODULE.collect(self.env)

    def test_symlink_package_is_rejected(self):
        path = self.root / "windows" / self.names[1]
        other = self.root / "windows" / "original"
        other.write_bytes(path.read_bytes())
        path.unlink()
        path.symlink_to(other)
        other.unlink()  # A dangling link is still never an acceptable artifact.
        with self.assertRaises(PackageError):
            MODULE.collect(self.env)

    def test_existing_different_output_is_never_overwritten(self):
        path = self.root / self.names[0]
        path.write_bytes(b"existing unrelated output")
        with self.assertRaises(PackageError):
            MODULE.collect(self.env)
        self.assertEqual(path.read_bytes(), b"existing unrelated output")

    def test_zero_size_and_non_file_metadata_are_rejected(self):
        path = self.root / "macos" / "BUILD-INFO.json"
        path.write_bytes(b"")
        with self.assertRaises(PackageError):
            MODULE.collect(self.env)


if __name__ == "__main__":
    unittest.main()
