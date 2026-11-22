"""Exact downloadable package inventory and bounded checksum validation."""
from __future__ import annotations

import re
import stat
from pathlib import Path

MAX_ASSET_BYTES = 512 * 1024 * 1024
MAX_CHECKSUM_BYTES = 4096


class PackageError(Exception):
    pass


def package_names(identity):
    return (
        f"Solar_{identity}_aarch64.dmg",
        f"Solar_{identity}_x64-setup.exe",
        f"Solar_{identity}_amd64.deb",
        f"Solar_{identity}_x86_64.AppImage",
    )


def regular_file(path: Path, limit=MAX_ASSET_BYTES):
    try:
        info = path.lstat()
    except OSError:
        raise PackageError("Expected a nonempty regular package file.") from None
    if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= limit:
        raise PackageError("Expected a nonempty regular package file within its size limit.")
    return info.st_size


def parse_checksums(data: bytes, names):
    if not 0 < len(data) <= MAX_CHECKSUM_BYTES:
        raise PackageError("SHA256SUMS exceeds its size limit.")
    try:
        lines = data.decode("ascii").splitlines()
    except UnicodeError:
        raise PackageError("SHA256SUMS must be ASCII.") from None
    expected = set(names)
    found = {}
    for line in lines:
        match = re.fullmatch(r"([0-9a-f]{64}) [ *]([A-Za-z0-9_.-]+)", line)
        if not match or match[2] not in expected or match[2] in found:
            raise PackageError("SHA256SUMS must list each expected package exactly once.")
        found[match[2]] = match[1]
    if set(found) != expected:
        raise PackageError("SHA256SUMS is missing expected packages.")
    return found
