#!/usr/bin/env python3
"""Exercise the real macOS image tools without compiling or signing an app."""
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile


def main():
    if sys.platform != 'darwin':
        raise SystemExit('The installer smoke test requires macOS.')
    with tempfile.TemporaryDirectory(prefix='solar-installer-smoke-') as folder:
        root = Path(folder)
        app = root / 'Solar.app'
        binary = app / 'Contents/MacOS/solar'
        binary.parent.mkdir(parents=True)
        binary.write_text('#!/bin/sh\nexit 0\n')
        binary.chmod(0o755)
        with (app / 'Contents/Info.plist').open('wb') as destination:
            plistlib.dump({'CFBundleExecutable': 'solar', 'CFBundleName': 'Solar',
                          'CFBundleIdentifier': 'net.solar.installer-smoke',
                          'CFBundlePackageType': 'APPL'}, destination)
        helper = Path(__file__).with_name('create-dmg.py')
        for volume in ('Solar', 'Solar Beta'):
            image = root / f'{volume}.dmg'
            command = [sys.executable, '-I', str(helper), '--app', str(app),
                       '--output', str(image), '--volume-name', volume]
            subprocess.run(command, check=True)
            subprocess.run(command + ['--verify-only'], check=True)
    print('Native installer smoke test passed for stable and beta layouts.')


if __name__ == '__main__':
    main()
