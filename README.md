# Solar

Your personal team of Orbits, at home on your desktop.

Solar is a local Rust and Tauri application with a React/TypeScript interface. Each Orbit has a role, personality, model connection, private memory, workspace, and tool permissions. A pixel-art sun gathers your Orbits and brings them onto the desktop.

## Develop

The primary target is Apple Silicon running macOS 14.2 or later. Install Rust, Node.js 22.22.2+ or 24.15+ (LTS), and Xcode command-line tools.

Use [Corepack](https://github.com/nodejs/corepack) to download and cache the exact pnpm version declared in `package.json`. If Corepack is missing or outdated, install it once with `npm install -g corepack@0.36.0` (Node.js 25+ no longer bundles Corepack).

```sh
corepack pnpm install --frozen-lockfile
corepack pnpm tauri dev
```

`packageManager` pins pnpm with an integrity hash; `pnpm-lock.yaml` pins dependencies. Corepack selects the same pnpm release on every machine. The `corepack pnpm` commands work without installing pnpm globally or enabling command shims. If you prefer plain `pnpm` commands, run `corepack enable` once.

The native preparation hook builds the Swift screen/accessibility helper and stages it as a Tauri sidecar. The helper is bundled with the application for distribution.

## First launch

Solar opens on black as white ripples gather into a new star. The welcome tour then introduces your Orbits and walks through model connections, the first Orbit, a project folder, and desktop preferences. Each setup step saves real settings and can be skipped or resumed; existing profiles and files are preserved when replaying the tour.

The welcome uses a restrained retro-console style: white vector geometry, amber/cyan accents, small monospaced labels, and faint stationary screen texture. Design references include the [Tektronix 4010 terminal](https://www.computerhistory.org/brochures/doc-4372956d27050/) and [vintage Roland synthesizers](https://www.roland.com/us/products/ju-06/); the artwork and synthesized score are original.

The optional soundtrack is synthesized locally: a seamless 25.714-second, 112 BPM electronic theme in E minor. Short plucked notes carry the syncopated melody; a quiet, low root-and-fifth layer supports it without sustained midrange chords. The progression follows 1–4–3–6–2 with a brief dominant turnaround: Em–Am–G–C–F♯ half-diminished–B7. Chords alternate **six beats, then two beats**, and the progression repeats with a melodic variation. The second-degree harmony follows [minor-key chord construction](https://musictheory.pugetsound.edu/mt21c/DiatonicChordsInMinor.html). The lead uses a fast attack and decay with [zero sustain](https://learningsynths.ableton.com/recipes/plucked-bass), restrained upper harmonics, and short stereo echoes. Soft half-time drums and gently swung percussion keep the groove moving. Welcome music has separate mute and volume controls. Music pauses when the welcome window hides and stops when you finish; reduced motion skips the moving starburst.

Find **Replay welcome tour**, **Welcome music**, and its volume in **Settings → Welcome experience**. Replay keeps your existing setup. **Settings → Start over → Reset Solar** instead shows one confirmation modal, then stops active work, clears Solar’s saved credentials and all app-managed data, and restarts from the opening animation. Files in external project folders are preserved.

For the browser-only UI preview, run the following and open the URL Vite prints.

```sh
corepack pnpm run dev
```

The browser preview uses clearly labeled, isolated local sample data. Model calls and operating-system tools execute in the desktop application.

## First session

1. Open **Connections** and configure LiteLLM, an OpenAI-compatible endpoint, or subscription sign-in.
2. Edit an Orbit and assign its connection and model. The starter roles are personal assistant, researcher, and QA.
3. Choose its tool categories and autonomy. Add a project folder and explicitly grant membership to Orbits that should share it.
4. Open an individual conversation or create a group with a lead Orbit. The lead can delegate to members of that group.
5. Use **Bring Orbits to desktop**, or click the sun. Drag characters to place them and click one to open a mini-chat.

The launcher closes into the menu bar. Quit from the menu-bar menu to stop Solar and its owned tool processes.

When an Orbit needs permission, encounters a task problem, or finishes your request, it peeks down from the top of the display. Solar uses the built-in display when available, reads the actual camera-housing safe area, and keeps the character and controls below the notch. Displays without a notch use a top-center panel below the menu bar. Opening the alert takes you to the relevant conversation; dismissing it never approves a tool. Multiple alerts queue, with permission requests first.

While the Orbits are collapsed, each Orbit plays its own stable, synthesized five-note signature. Every chime lasts **800 milliseconds**, including its release tail. **Settings → Attention sound** controls sound and volume, and the Orbit picker previews each character's alert. Reduced motion also applies to the peek animation. Sound playback uses macOS; browser/Linux previews show the animation without native audio playback.

This build presents a Solar-owned overlay, including on Golden Gate. Apple's native [ActivityKit](https://developer.apple.com/documentation/activitykit) integration on macOS 27.3+ requires a separate WidgetKit extension; Solar does not currently ship that extension or control the system Dynamic Island.

## Connections

- **OpenAI-compatible and LiteLLM** connections accept a base URL, optional API key, and discovered or manually entered model IDs. Existing local model servers can use the same interface.
- **ChatGPT subscription** uses native Rust/Rig streaming with Solar-owned OAuth and credential refresh, adapted from Galaxy.
- **Claude subscription** uses Galaxy's native Rig Messages compatibility path. It is experimental and unsupported by Anthropic for third-party subscription use. The application requires explicit enablement and displays the possible account/access restriction warning.

API keys and OAuth tokens stay in macOS Keychain. They are excluded from profile exports, SQLite records, and diagnostic error text. Linux development uses session-only credential storage. No model connection is automatically replaced with a paid alternative.

## Tools and permissions

Built-in tools cover scoped file listing, reading, writing and search, HTTP retrieval, a dedicated Chromium browser, shell commands, MCP tools, screen capture, and macOS application inspection/control. Browser automation requires an installed Chrome or Chromium and uses a separate profile for each Orbit.

**Ask** prompts for every tool. **Workspace** allows granted, scoped tools and asks for host execution. **Trusted** allows granted host tools automatically. Every delegated action remains subject to all participating Orbits' grants and shared-project membership.

Built-in file operations use directory capabilities and reject path/symlink escapes. Shell programs, browser processes, and locally installed MCP servers run with the user's host privileges after authorization. Their working directories are not an OS sandbox. Native OS containment of arbitrary third-party executables still requires separate macOS qualification.

Screen sharing begins only through a user action and macOS's content picker. Application/window sharing supports observation. Computer control requires whole-display sharing and Accessibility authorization, and is bounded to that display. Sharing stops at task completion or through Solar or the system stop control.

MCP supports local stdio and remote Streamable HTTP. Add the server in Connections, enable it, connect it, and grant MCP access to the desired Orbit. Server-initiated sampling and elicitation return explicit unsupported-capability errors.

## Local data and task recovery

Tauri's application-data directory contains `solar.sqlite`, private Orbit workspaces, and dedicated browser profiles. SQLite stores conversations, profiles, permissions, routines, and durable task checkpoints. Shared project memory is visible only to explicitly granted project members.

Scheduled, folder-change, and task-completion routines run while Solar is running and the computer is awake. Missed intervals coalesce into one due run. Folder polling checks nested paths, skips symlinks, and supports up to 50,000 entries per watched project.

Tasks have a 30-minute deadline, bounded model turns and retries, and a configurable model concurrency limit. A computer-sharing session has one owner. Cancelled or interrupted tools receive explicit transcript results. Application restart marks interrupted tasks for review and preserves their history without replaying external actions.

Orbit exports use versioned JSON. Imports receive a fresh identity, require new provider/project assignments, and start in Ask mode.

## Validate

```sh
corepack pnpm run build
cargo fmt --all -- --check
cargo test --workspace --no-default-features
cargo check -p solar
```

On Linux with Homebrew first in PATH, system GTK/WebKit discovery may need `PKG_CONFIG=/usr/bin/pkg-config`. The Rust application tests use loopback fixtures and dummy credentials.

Native acceptance needs an actual Mac. Verify screen-picker cancellation/revocation, Accessibility controls, keyboard focus, transparent hit regions, expand/collapse animation, attention chimes and camera-housing placement, Spaces/fullscreen behavior, and mixed-scale displays before distributing a build. The Swift helper and macOS Keychain paths cannot be exercised on Linux.

## Package for macOS

```sh
corepack pnpm tauri build --target aarch64-apple-darwin
```

Configure Developer ID signing and Apple notarization credentials using Tauri's macOS distribution documentation. Release artifacts are produced under the target's `release/bundle` directory. Signing credentials are intentionally supplied by the developer's release environment.

## Architecture and license

- `crates/solar-agent-core` contains Galaxy-derived, UI-independent runtime contracts and the durable provider-run state machine.
- `crates/solar-providers` contains selected Rig adapters, request/stream translation, credential services, and provider regression fixtures.
- `crates/solar-tools` contains local tool implementations, MCP transports, browser sessions, and the native macOS bridge.
- `src-tauri` owns authorization, persistence, scheduling, Orbit collaboration, desktop windows, and the frontend command boundary.
- `src` contains the React interface and code-drawn pixel-art characters.

Solar is licensed under AGPL-3.0-only. Galaxy-derived crates retain their provenance and license files. Rig is pinned to the Galaxy-compatible revision recorded in their manifests. Original Solar UI code and assets are distributed under the same project license.
