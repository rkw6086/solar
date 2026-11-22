#!/usr/bin/env python3
"""Validate all platform build outputs before assembling one release inventory."""
from __future__ import annotations

import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import shutil
import tempfile

from release_packages import MAX_CHECKSUM_BYTES, PackageError, package_names, parse_checksums, regular_file


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            result.update(chunk)
    return result.hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise PackageError("Build metadata contains duplicate keys.")
        result[key] = value
    return result


def collect(env):
    version = env.get("SOLAR_RELEASE_VERSION", "")
    source = env.get("SOLAR_RELEASE_SHA", "")
    channel = env.get("SOLAR_BUILD_CHANNEL", "")
    if (not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version)
            or not re.fullmatch(r"[0-9a-f]{40}", source) or set(source) == {"0"}
            or channel not in ("release", "beta")):
        raise PackageError("Collection requires a validated release version, source and channel.")
    if channel == "beta" and env.get("SOLAR_BETA_ID") != source:
        raise PackageError("Beta package identity differs from its source.")
    root = Path(env.get("SOLAR_RELEASE_DIR", ""))
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise PackageError("Use an absolute regular release directory.")
    identity = "beta_" + source if channel == "beta" else version
    names = package_names(identity)
    groups = (
        ("macos", "aarch64-apple-darwin", True, names[:1]),
        ("windows", "x86_64-pc-windows-msvc", False, names[1:2]),
        ("linux", "x86_64-unknown-linux-gnu", False, names[2:]),
    )
    allowed = {"macos", "windows", "linux", "SHA256SUMS", *names}
    if {entry.name for entry in root.iterdir()} - allowed:
        raise PackageError("Unexpected files in release collection directory.")
    packages = {}
    for folder, target, signed, expected in groups:
        directory = root / folder
        if directory.is_symlink() or not directory.is_dir():
            raise PackageError("Missing regular platform artifact directory.")
        if {entry.name for entry in directory.iterdir()} != {*expected, "SHA256SUMS", "BUILD-INFO.json"}:
            raise PackageError("Platform artifact inventory differs from expected packages.")
        metadata = directory / "BUILD-INFO.json"
        regular_file(metadata, 16 * 1024)
        try:
            info = json.loads(metadata.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
        except (UnicodeError, json.JSONDecodeError):
            raise PackageError("Invalid platform build metadata.") from None
        listed = info.get("packages") if isinstance(info, dict) else None
        if (not isinstance(info, dict) or set(info) != {"version", "source", "target", "channel", "signed", "packages"}
                or info.get("version") != version or info.get("source") != source
                or info.get("target") != target or info.get("channel") != channel
                or info.get("signed") is not signed or not isinstance(listed, list)
                or any(not isinstance(name, str) for name in listed)
                or len(listed) != len(expected) or set(listed) != set(expected)):
            raise PackageError("Platform build metadata does not match the validated release.")
        checksum = directory / "SHA256SUMS"
        regular_file(checksum, MAX_CHECKSUM_BYTES)
        hashes = parse_checksums(checksum.read_bytes(), expected)
        for name in expected:
            path = directory / name
            regular_file(path)
            value = digest(path)
            if not hmac.compare_digest(value, hashes[name]):
                raise PackageError("Platform package failed SHA256SUMS verification.")
            packages[name] = (path, value)
    # Validate all sources before writing any final package. Stage copies, then
    # atomically replace individual files; a retry revalidates the same inventory.
    with tempfile.TemporaryDirectory(prefix=".solar-collect-", dir=root) as temporary:
        staged = Path(temporary)
        for name, (source_path, value) in packages.items():
            destination = staged / name
            shutil.copyfile(source_path, destination, follow_symlinks=False)
            regular_file(destination)
            if not hmac.compare_digest(digest(destination), value):
                raise PackageError("Package changed while copying verified output.")
        (staged / "SHA256SUMS").write_text(
            "".join(f"{packages[name][1]}  {name}\n" for name in names), encoding="ascii")
        for name in (*names, "SHA256SUMS"):
            destination = root / name
            if destination.exists() or destination.is_symlink():
                regular_file(destination)
                if not hmac.compare_digest(digest(destination), digest(staged / name)):
                    raise PackageError("Existing collected output differs; refusing to overwrite it.")
        for name in (*names, "SHA256SUMS"):
            os.replace(staged / name, root / name)
    return names


if __name__ == "__main__":
    try:
        names = collect(os.environ)
    except (OSError, PackageError) as error:
        message = str(error) if isinstance(error, PackageError) else "Could not read or stage platform packages."
        raise SystemExit("Solar package collection failed: " + message) from None
    print(f"Verified and collected {len(names)} platform packages with combined SHA256SUMS.")
