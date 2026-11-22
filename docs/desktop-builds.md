# Windows and Linux builds

The **Build Windows and Linux** GitHub Actions workflow builds native x64 packages on Windows and Ubuntu runners. It runs for pushes to `main`, pull requests targeting `main`, and manual dispatches. Use **Actions → Build Windows and Linux → Run workflow** to test both platforms without publishing a release.

Each successful job uploads its packages, `SHA256SUMS`, and `BUILD-INFO.json` as a one-day Actions artifact:

| Platform | Packages | Runner |
| --- | --- | --- |
| Windows x64 | `Solar_VERSION_x64-setup.exe` | Windows Server 2025 / MSVC |
| Linux x64 | `Solar_VERSION_amd64.deb`, `Solar_VERSION_x86_64.AppImage` | Ubuntu 22.04 |

The existing **Build Solar** workflow also calls these jobs for new version tags and nightly/manual beta builds. Publication waits for all three platforms, validates their source/version/channel metadata and checksums, then publishes one combined `SHA256SUMS`. Stable versions retain the version in their filenames; beta filenames use `beta_FULL_COMMIT`. Older downloads are retired only after the complete replacement is verified.

## Install and test

Windows installers are currently unsigned. Use the NSIS installer for your own testing; Windows may display an unrecognized-app warning. The installer targets the current user and installs WebView2 when needed. Windows builds currently support the desktop interface and model connections, but workspace file tools and isolated shell, browser, and local MCP processes remain unavailable. Those tools fail closed. Native screen/computer tools and notification audio currently require macOS.

On a compatible Debian/Ubuntu system, install the `.deb` with `sudo apt install ./Solar_VERSION_amd64.deb`. Its dependencies include Bubblewrap for process isolation. For the AppImage, install your distribution's `bubblewrap` package, make the file executable, and run it:

```sh
chmod +x Solar_VERSION_x86_64.AppImage
./Solar_VERSION_x86_64.AppImage
```

Linux packages are built against Ubuntu 22.04's system libraries; newer distributions with compatible libraries are the intended targets. Host policies must permit Bubblewrap user namespaces for Sprite processes. Native screen/computer tools and notification audio are not implemented on Linux yet. Desktop widget behavior still needs testing on each window manager, especially Wayland.

Verify downloaded packages using `sha256sum --ignore-missing -c SHA256SUMS` on Linux, or compare `Get-FileHash .\Solar_VERSION_x64-setup.exe -Algorithm SHA256` with its entry in `SHA256SUMS` in PowerShell. The release checksum file covers all platforms.

## Build locally

Install the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/), Rust through rustup, and Node.js 24.17.0. Corepack selects the pnpm version pinned in `package.json`.

```sh
npm install --global corepack@0.36.0
corepack pnpm install --frozen-lockfile
```

On Windows, use the MSVC toolchain and run:

```sh
corepack pnpm tauri build --target x86_64-pc-windows-msvc --bundles nsis --no-sign -- --locked
```

On Linux, run:

```sh
corepack pnpm tauri build --target x86_64-unknown-linux-gnu --bundles deb,appimage --no-sign -- --locked
```

Tauri automatically merges `tauri.windows.conf.json` or `tauri.linux.conf.json`. Windows uses its own ICO resource; the Swift helper is built and bundled only on macOS. These build jobs have no signing credentials or release-write permissions. The separate publication job owns release updates.
