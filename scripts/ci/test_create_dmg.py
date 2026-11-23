"""Installer presentation checks; Apple mounting is simulated, metadata is real."""

import copy
import datetime
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("solar_create_dmg", Path(__file__).with_name("create-dmg.py"))
dmg = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dmg)
try:
    from ds_store import DSStore
    from mac_alias import Alias, TargetInfo, VolumeInfo
    HAS_LAYOUT_TOOLS = True
except ImportError:
    HAS_LAYOUT_TOOLS = False


class CopyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.source = self.root / "source" / "Solar.app"
        (self.source / "Contents/MacOS").mkdir(parents=True)
        executable = self.source / "Contents/MacOS/solar"
        executable.write_bytes(b"executable")
        executable.chmod(0o755)
        (self.source / "Contents/resource").write_bytes(b"resource")
        (self.source / "Contents/link").symlink_to("resource")
        self.expected = dmg.bundle_manifest(self.source)
        self.mount = self.root / "mounted"
        self.mount.mkdir()
        shutil.copytree(self.source, self.mount / "Solar.app", symlinks=True)

    def tearDown(self):
        self.temporary.cleanup()

    def test_complete_copy_passes(self):
        dmg.verify_copy(self.mount, self.expected)

    def test_same_size_resource_corruption_fails(self):
        (self.mount / "Solar.app/Contents/resource").write_bytes(b"RESOURCE")
        with self.assertRaisesRegex(RuntimeError, "changed while copying"):
            dmg.verify_copy(self.mount, self.expected)

    def test_missing_file_changed_mode_and_link_fail(self):
        for kind in ("missing", "mode", "link"):
            with self.subTest(kind=kind):
                target = self.mount / "Solar.app"
                shutil.rmtree(target)
                shutil.copytree(self.source, target, symlinks=True)
                if kind == "missing":
                    (target / "Contents/resource").unlink()
                elif kind == "mode":
                    (target / "Contents/MacOS/solar").chmod(0o644)
                else:
                    (target / "Contents/link").unlink()
                    (target / "Contents/link").symlink_to("elsewhere")
                with self.assertRaisesRegex(RuntimeError, "changed while copying"):
                    dmg.verify_copy(self.mount, self.expected)

    def test_signature_is_checked_on_the_copied_app(self):
        with patch.object(dmg.subprocess, "run") as run:
            dmg.verify_copy(self.mount, self.expected, True)
        self.assertEqual(run.call_args.args[0][-1], str(self.mount / "Solar.app"))
        self.assertTrue(run.call_args.kwargs["check"])


@unittest.skipUnless(HAS_LAYOUT_TOOLS, "Install the pinned DMG tooling to exercise real Finder records")
class LayoutTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        (self.root / "Solar.app").mkdir()
        (self.root / "Applications").symlink_to("/Applications")
        (self.root / dmg.BACKGROUND).write_bytes(b"TIFF-test-pixels")
        self.artwork = {"background.png": "1" * 64, "background@2x.png": "2" * 64}
        (self.root / dmg.MANIFEST).write_text(json.dumps({
            "version": 1, "artwork": self.artwork,
            "background": dmg.file_hash(self.root / dmg.BACKGROUND),
        }))
        date = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
        alias = Alias(volume=VolumeInfo("Solar Beta", date, b"H+", 0, 0, b"\0\0"),
                      target=TargetInfo(0, dmg.BACKGROUND, 2, 3, date, b"\0" * 4, b"\0" * 4,
                                        posix_path="/" + dmg.BACKGROUND))
        self.metadata = {
            ".": {"icvl": (b"type", b"icnv"),
                  "bwsp": {"WindowBounds": dmg.BOUNDS, "ShowStatusBar": False,
                           "ShowTabView": False, "ShowToolbar": False,
                           "ShowPathbar": False, "ShowSidebar": False},
                  "icvp": {"backgroundType": 2, "iconSize": 112.0, "textSize": 14.0,
                           "arrangeBy": "none", "labelOnBottom": True,
                           "backgroundImageAlias": alias.to_bytes()}},
            **{name: {"Iloc": position} for name, position in dmg.POSITIONS.items()},
        }
        self.write_metadata()

    def tearDown(self):
        self.temporary.cleanup()

    def write_metadata(self):
        (self.root / ".DS_Store").unlink(missing_ok=True)
        with DSStore.open(str(self.root / ".DS_Store"), "w+") as store:
            for name, records in self.metadata.items():
                for code, value in records.items():
                    store[name][code] = value

    def verify(self):
        with patch.object(dmg, "artwork_hashes", return_value=self.artwork):
            dmg.verify_layout(self.root, "Solar Beta")

    def test_real_finder_records_and_alias_round_trip(self):
        self.verify()

    def test_missing_and_corrupt_layout_fail(self):
        for data in (None, b"not a DS Store"):
            with self.subTest(data=data):
                path = self.root / ".DS_Store"
                path.unlink(missing_ok=True)
                if data is not None:
                    path.write_bytes(data)
                with self.assertRaises(Exception):
                    self.verify()

    def test_incorrect_positions_bounds_view_and_background_fail(self):
        original = copy.deepcopy(self.metadata)
        for name, code, value in (
            ("Solar.app", "Iloc", (530, 250)),
            (".", "icvl", (b"type", b"Nlsv")),
            (".", "bwsp", {**original["."]["bwsp"], "WindowBounds": "{{0,0},{99,99}}"}),
            (".", "icvp", {**original["."]["icvp"], "backgroundImageAlias": b"broken"}),
            (".", "icvp", {**original["."]["icvp"], "iconSize": 12}),
        ):
            with self.subTest(name=name, code=code):
                self.metadata = copy.deepcopy(original)
                self.metadata[name][code] = value
                self.write_metadata()
                with self.assertRaises(RuntimeError):
                    self.verify()

    def test_background_corruption_wrong_shortcut_or_extra_visible_item_fail(self):
        (self.root / dmg.BACKGROUND).write_bytes(b"tampered pixels")
        with self.assertRaisesRegex(RuntimeError, "artwork"):
            self.verify()
        (self.root / "Applications").unlink()
        (self.root / "Applications").symlink_to("/tmp")
        with self.assertRaisesRegex(RuntimeError, "wrong target"):
            self.verify()
        (self.root / "readme.txt").touch()
        with self.assertRaisesRegex(RuntimeError, "only Solar"):
            self.verify()


class MountTests(unittest.TestCase):
    def invoke(self, fail_verify=False, invalid_attach=False, retry_detach=False, canonical_attach=False):
        calls = []

        def run(args, **_kwargs):
            calls.append(args)
            if "attach" in args:
                mount = args[args.index("-mountpoint") + 1]
                if canonical_attach:
                    alias = Path(mount).parent / "canonical-volume"
                    alias.symlink_to(mount, target_is_directory=True)
                    mount = str(alias)
                payload = b"broken" if invalid_attach else plistlib.dumps({
                    "system-entities": [{"mount-point": mount, "dev-entry": "/dev/test"}]})
                return subprocess.CompletedProcess(args, 0, payload, b"")
            return subprocess.CompletedProcess(args, int(retry_detach and "-force" not in args), b"", b"")

        with patch.object(dmg, "verify_copy", side_effect=RuntimeError("bad copy") if fail_verify else None), \
                patch.object(dmg, "verify_layout"):
            if fail_verify or invalid_attach:
                with self.assertRaises(RuntimeError):
                    dmg.verify_dmg(Path("test.dmg"), {}, "Solar", run=run)
            else:
                dmg.verify_dmg(Path("test.dmg"), {}, "Solar", run=run)
        return calls

    def test_read_only_mount_is_detached_on_success_and_failure(self):
        for failure in (False, True):
            calls = self.invoke(fail_verify=failure)
            self.assertIn("-readonly", calls[0])
            self.assertEqual(calls[-1][1], "detach")
            self.assertFalse(Path(calls[-1][-1]).exists())

    def test_failed_detach_is_retried_with_force(self):
        calls = self.invoke(retry_detach=True)
        self.assertEqual(calls[-1][1:3], ["detach", "-force"])

    def test_invalid_attach_without_mount_never_attempts_to_detach_other_devices(self):
        self.assertEqual(len(self.invoke(invalid_attach=True)), 1)

    def test_invalid_attach_that_mounted_is_still_detached(self):
        with patch.object(dmg.os.path, "ismount", return_value=True):
            calls = self.invoke(invalid_attach=True)
        self.assertEqual(calls[-1][1], "detach")

    def test_canonical_mount_path_alias_is_accepted(self):
        calls = self.invoke(canonical_attach=True)
        self.assertEqual(calls[-1][1], "detach")


if __name__ == "__main__":
    unittest.main()
