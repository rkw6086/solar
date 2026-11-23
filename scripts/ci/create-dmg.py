#!/usr/bin/env python3
"""Build and verify Solar's Finder installer without AppleScript or Finder."""

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile


PINNED = {"dmgbuild": "1.6.7", "ds-store": "1.3.1", "mac-alias": "2.2.2"}
BOUNDS = "{{180, 160}, {700, 500}}"
POSITIONS = {"Solar.app": (170, 250), "Applications": (530, 250)}
MANIFEST = ".solar-installer.json"
BACKGROUND = ".background.tiff"
ARTWORK = Path(__file__).resolve().parent / "dmg" / "background.png"


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def bundle_manifest(bundle):
    """Detect failed copies, changed resources, permissions, and symlink targets."""
    bundle = Path(bundle)
    if bundle.is_symlink() or not bundle.is_dir():
        raise RuntimeError("The application bundle is missing or is a symlink")
    result = {}
    for folder, directories, files in os.walk(bundle, followlinks=False):
        for name in sorted(directories + files):
            path = Path(folder) / name
            relative = path.relative_to(bundle).as_posix()
            if path.is_symlink():
                result[relative] = ("symlink", os.readlink(path))
            elif path.is_file():
                result[relative] = ("file", path.stat().st_size,
                                    path.stat().st_mode & 0o111, file_hash(path))
            elif path.is_dir():
                result[relative] = ("directory",)
            else:
                raise RuntimeError(f"Unsupported application item: {relative}")
    return result


def verify_copy(root, expected, verify_signature=False):
    app = Path(root) / "Solar.app"
    if bundle_manifest(app) != expected:
        raise RuntimeError("The application bundle changed while copying to the DMG")
    if verify_signature:
        subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)


def artwork_hashes():
    return {path.name: file_hash(path) for path in (ARTWORK, ARTWORK.with_name("background@2x.png"))}


def validate_layout_metadata(metadata, volume_name):
    """Validate decoded Finder records, independently of the creation settings."""
    from mac_alias import Alias

    root = metadata.get(".", {})
    if root.get("icvl") != (b"type", b"icnv"):
        raise RuntimeError("The installer does not open in icon view")
    window = root.get("bwsp", {})
    if window.get("WindowBounds") != BOUNDS:
        raise RuntimeError("The installer window bounds are missing or incorrect")
    for name in ("ShowStatusBar", "ShowTabView", "ShowToolbar", "ShowPathbar", "ShowSidebar"):
        if window.get(name) is not False:
            raise RuntimeError(f"Unexpected Finder window setting: {name}")
    view = root.get("icvp", {})
    if (view.get("backgroundType"), view.get("iconSize"), view.get("textSize"),
            view.get("arrangeBy"), view.get("labelOnBottom")) != (2, 112, 14, "none", True):
        raise RuntimeError("The installer icon or background settings are incorrect")
    try:
        alias = Alias.from_bytes(view["backgroundImageAlias"])
    except Exception as error:
        raise RuntimeError("The installer background alias is missing or invalid") from error
    if (alias.volume.name != volume_name or alias.target.filename != BACKGROUND
            or alias.target.posix_path != "/" + BACKGROUND):
        raise RuntimeError("The installer background alias points outside this volume")
    for name, position in POSITIONS.items():
        if metadata.get(name, {}).get("Iloc") != position:
            raise RuntimeError(f"The installer icon position is incorrect: {name}")


def verify_layout(root, volume_name):
    from ds_store import DSStore

    root = Path(root)
    visible = {path.name for path in root.iterdir() if not path.name.startswith(".")}
    if visible != set(POSITIONS) or not (root / "Applications").is_symlink():
        raise RuntimeError("The installer must show only Solar and Applications")
    if os.readlink(root / "Applications") != "/Applications":
        raise RuntimeError("The Applications shortcut has the wrong target")
    for name in (MANIFEST, BACKGROUND, ".DS_Store"):
        path = root / name
        if path.is_symlink() or not path.is_file() or not path.stat().st_size:
            raise RuntimeError(f"The installer is missing {name}")
    manifest = json.loads((root / MANIFEST).read_text())
    if manifest != {"version": 1, "artwork": artwork_hashes(), "background": file_hash(root / BACKGROUND)}:
        raise RuntimeError("The installer artwork failed its integrity check")
    metadata = {}
    # Hold the file ourselves too: a corrupt store can fail during DSStore.open.
    with (root / ".DS_Store").open("rb") as stream:
        with DSStore.open(stream, "r") as store:
            for entry in store:
                code = entry.code.decode("ascii")
                metadata.setdefault(entry.filename, {})[code] = (
                    (entry.type, entry.value) if code == "icvl" else entry.value
                )
    validate_layout_metadata(metadata, volume_name)


def verify_dmg(output, expected, volume_name, verify_signature=False, run=None):
    """Mount only our image read-only and detach even when verification fails."""
    run = run or subprocess.run
    temporary = Path(tempfile.mkdtemp(prefix="solar-dmg-check-")).resolve()
    mount = temporary / "volume"
    mount.mkdir()
    attached = False
    failure = None
    try:
        result = run(["/usr/bin/hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen",
                      "-owners", "off", "-mountpoint", str(mount), "-plist", str(output)],
                     capture_output=True, check=False)
        try:
            entities = plistlib.loads(result.stdout).get("system-entities", [])
        except Exception:
            entities = []
        # macOS reports /private/var for paths created through /var (including
        # the standard TMPDIR). Compare filesystem identities, not spellings.
        reported_mount = any(isinstance(entity, dict) and isinstance(entity.get("mount-point"), str)
                             and Path(entity["mount-point"]).resolve() == mount.resolve()
                             for entity in entities)
        # An interrupted attach can mount a volume without returning valid plist.
        attached = reported_mount or os.path.ismount(mount)
        if result.returncode or not reported_mount:
            detail = result.stderr.decode("utf-8", errors="replace").strip()[:1000]
            mount_paths = [entity.get("mount-point") for entity in entities
                           if isinstance(entity, dict) and "mount-point" in entity][:8]
            raise RuntimeError("Could not mount the installer read-only for verification "
                               f"(exit={result.returncode!r}, reported={repr(mount_paths)[:1000]}, "
                               f"expected={str(mount)!r}, stderr={detail!r})")
        verify_copy(mount, expected, verify_signature)
        verify_layout(mount, volume_name)
    except BaseException as error:
        failure = error
        raise
    finally:
        detached = not attached
        if attached:
            try:
                result = run(["/usr/bin/hdiutil", "detach", str(mount)], capture_output=True, check=False)
                if result.returncode:
                    result = run(["/usr/bin/hdiutil", "detach", "-force", str(mount)],
                                 capture_output=True, check=False)
                if result.returncode:
                    raise RuntimeError("Could not detach the verified installer volume")
                detached = True
            except BaseException as error:
                if failure is None:
                    raise
                if hasattr(failure, "add_note"):
                    failure.add_note(str(error))
        if detached:
            shutil.rmtree(temporary)


def create_dmg(app, output, volume_name, verify_signature=False):
    from dmgbuild import build_dmg

    expected = bundle_manifest(app)
    with tempfile.TemporaryDirectory(prefix="solar-dmg-artwork-") as folder:
        background = Path(folder) / "background.tiff"
        subprocess.run(["/usr/bin/tiffutil", "-cathidpicheck", str(ARTWORK),
                        str(ARTWORK.with_name("background@2x.png")), "-out", str(background)], check=True)
        identity = {"version": 1, "artwork": artwork_hashes(), "background": file_hash(background)}

        def verify_stage(mount_point, _settings):
            verify_copy(mount_point, expected, verify_signature)
            if file_hash(Path(mount_point) / BACKGROUND) != identity["background"]:
                raise RuntimeError("The installer background changed during staging")
            (Path(mount_point) / MANIFEST).write_text(json.dumps(identity, sort_keys=True) + "\n")

        settings = {
            "format": "UDZO", "compression_level": 9, "filesystem": "APFS",
            "files": [str(app)], "symlinks": {"Applications": "/Applications"},
            "background": str(background), "window_rect": ((180, 160), (700, 500)),
            "default_view": "icon-view", "include_icon_view_settings": True,
            "show_status_bar": False, "show_tab_view": False, "show_toolbar": False,
            "show_pathbar": False, "show_sidebar": False, "arrange_by": None,
            "grid_spacing": 80, "icon_size": 112, "text_size": 14,
            "icon_locations": POSITIONS.copy(), "create_hook": verify_stage,
        }
        try:
            build_dmg(str(output), volume_name, settings=settings)
            verify_dmg(output, expected, volume_name, verify_signature)
        except BaseException:
            output.unlink(missing_ok=True)
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--volume-name", choices=("Solar", "Solar Beta"), default="Solar")
    parser.add_argument("--verify-signature", action="store_true")
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("DMG creation and verification require macOS")
    for package, version in PINNED.items():
        if importlib.metadata.version(package) != version:
            parser.error(f"Use the pinned {package} {version} packaging tool")
    app = args.app.resolve(strict=True)
    output = args.output.absolute()
    if app.name != "Solar.app" or not app.is_dir() or args.app.is_symlink():
        parser.error("--app must be the complete Solar.app bundle")
    with (app / "Contents/Info.plist").open("rb") as stream:
        executable = plistlib.load(stream).get("CFBundleExecutable")
    if executable != "solar" or not (app / "Contents/MacOS/solar").is_file():
        parser.error("Solar.app has no valid main executable")
    if args.verify_only:
        if output.is_symlink() or not output.is_file():
            parser.error("--verify-only requires an existing regular DMG")
        verify_dmg(output, bundle_manifest(app), args.volume_name, args.verify_signature)
    else:
        if output.exists() or output.is_symlink() or not output.parent.is_dir():
            parser.error("--output requires a fresh path in an existing directory")
        create_dmg(app, output, args.volume_name, args.verify_signature)
    print(f"Verified Solar installer artwork, icon positions, and application: {output.name}")


if __name__ == "__main__":
    main()
