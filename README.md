# Solar

Your personal team of Orbits, at home on your desktop.

Solar is a local Rust and Tauri application with a React/TypeScript interface. Each Orbit has a role, personality, model connection, private memory, workspace, and approval preferences. A pixel-art sun gathers your Orbits and brings them onto the desktop.

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

Solar opens a borderless, screen-sized window on black as white ripples gather into a new star. The welcome tour then introduces your Orbits and walks through model connections, the first Orbit, a project folder, and desktop preferences. The whole tour uses that window without entering macOS fullscreen or creating a fullscreen Space, then restores the launcher's previous size, position, and window controls. Each setup step saves real settings and can be skipped or resumed; existing profiles and files are preserved when replaying the tour.

The welcome uses a restrained retro-console style: white vector geometry, amber/cyan accents, small monospaced labels, and faint stationary screen texture. Design references include the [Tektronix 4010 terminal](https://www.computerhistory.org/brochures/doc-4372956d27050/) and [vintage Roland synthesizers](https://www.roland.com/us/products/ju-06/); the artwork and synthesized sound effects are original.

There is no background music. Each ripple reaching the center makes a quiet glass-like drop, followed by a brief airy major chord as the sun forms. The sun then winks and plays a randomly varied Orbit-style chime. Controls have a short, dry click. These effects are synthesized locally and timed from the animation's own timeline; they stop when you hide, skip, or leave the introduction. The design follows Apple's guidance for [subtle, varied interaction sounds that match visual transitions](https://developer.apple.com/videos/play/wwdc2023/10271/). Reduced motion shows a still sun with one short formation cue and skips the animated wink and ripple sounds.

The welcome's **Sound Effects On / Off** button controls effects throughout Solar, including Orbit chimes. Find **Replay welcome tour**, **Sound effects**, and its volume in **Settings → Welcome experience**. Orbit chimes also retain their own category switch and volume. Replay keeps your existing setup. **Settings → Start over → Reset Solar** instead shows one confirmation modal, then stops active work, clears Solar’s saved credentials and all app-managed data, and restarts from the opening animation. Files in external project folders are preserved.

For the browser-only UI preview, run the following and open the URL Vite prints.

```sh
corepack pnpm run dev
```

The browser preview uses isolated local data and starts with no Orbits, just like the desktop app. Model calls and operating-system tools execute in the desktop application.

## First session

1. Open **Connections** and configure LiteLLM, an OpenAI-compatible endpoint, or subscription sign-in.
2. Create your first Orbit with a name, optional gender, personality, and tasks. Search all model connections in one field; model labels include their provider. Solar does not create Orbits automatically.
3. Choose when it should ask for approval. Every Orbit can request all available tools. Add a project folder and grant membership to Orbits that should share it; assigned projects are available even in individual chats with no selected project.
4. Open an individual conversation or create a group with a lead Orbit. The lead can delegate to members of that group.
5. Use **Bring Orbits to desktop**, or click the sun. Planets expand into fixed slots around it; drag the sun to move the whole group, and click a planet to open its mini-chat.

Press **Create Orbit** and the selected model automatically builds its role and private system instructions from your details. Changing its name, gender, role, personality, or tasks regenerates those instructions; changing only the model, permissions, or project access keeps them intact and does not call a model. The generated prompt stays hidden; there is no prompt review or approval step. While instructions are being prepared, a progress overlay keeps the form open until the save succeeds or an error lets you retry. A model connection is required only when generating instructions. Your original tasks are kept separately from the generated instructions. Model search matches both IDs and provider names, supports multiple search terms, and accepts `*` and `?` wildcards.

Every chat has an Orbit settings button, including desktop popup chats. You can also right-click a desktop planet to open its settings. Edit its name, gender, role, personality, tasks, model, project access, and approval preferences. **Reset Orbit** clears its private conversations, memories, and activity while keeping its profile, workspace files, and shared group conversations. **Delete Orbit** removes it from the team. Both actions ask once for confirmation. Earlier saved profiles remain until you remove them; startup and the welcome tour never add default Orbits.

Tool cards show what an Orbit is doing without raw protocol data. Shell cards show the command, working folder, live output, and exit status; output follows the latest line until you scroll up. Web cards summarize the search or source, and MCP cards identify the service and command. Cards can be collapsed while work continues. File changes observed during shell commands, file writes, and MCP calls appear as readable diffs for accessible Git projects. These previews compare against the files immediately before the call, preserve existing edits, and do not change Git's index. Ignored files, symlinks, large/binary files, and very large changes may be omitted or shortened; simultaneous edits by another program may also appear.

The launcher closes into the menu bar. Sun and planet widgets appear while the launcher is hidden; opening the launcher hides them and closes popup chats. **Bring Orbits to desktop** hides the launcher and expands the planets. Each expansion uses the same slots for the same Orbit roster, and the whole group stays together at display edges. Quit from the menu-bar menu to stop Solar and its owned tool processes.

The launcher's **Solar system** home shows your star and the Orbits you create moving on paths with resonant periods. Select a planet to open its chat. With no Orbits, the star invites you to create your first teammate. Returning from a conversation also shows the system above your conversation history. Motion pauses when you interact with the planets or hide the page, and respects reduced-motion preferences. Existing Orbit profiles are preserved.

When an Orbit needs permission, encounters a task problem, or finishes your request, it peeks down from the top of the display. Solar uses the built-in display when available, reads the actual camera-housing safe area, and keeps the character and controls below the notch. Displays without a notch use a top-center panel below the menu bar. Opening the alert takes you to the relevant conversation; dismissing it never approves a tool. Multiple alerts queue, with permission requests first.

When that conversation is already visible in the launcher or a popup chat, Solar skips its alert and chime. Approval controls remain in the chat, and unrelated conversations can still notify you.

While the Orbits are collapsed, each Orbit plays its own stable, synthesized five-note signature. Every chime lasts **800 milliseconds**, including its release tail. **Settings → Attention sound** controls sound and volume, and the Orbit picker previews each character's alert. Reduced motion also applies to the peek animation. Sound playback uses macOS; browser/Linux previews show the animation without native audio playback.

This build presents a Solar-owned overlay, including on Golden Gate. Apple's native [ActivityKit](https://developer.apple.com/documentation/activitykit) integration on macOS 27.3+ requires a separate WidgetKit extension; Solar does not currently ship that extension or control the system Dynamic Island.

## Connections

- **OpenAI-compatible and LiteLLM** connections accept a base URL, optional API key, and discovered or manually entered model IDs. Existing local model servers can use the same interface.
- **ChatGPT subscription** uses native Rust/Rig streaming with Solar-owned OAuth and credential refresh, adapted from Galaxy.
- **Claude subscription** uses Galaxy's native Rig Messages compatibility path. It is experimental and unsupported by Anthropic for third-party subscription use. The application requires explicit enablement and displays the possible account/access restriction warning.

Provider API keys and OAuth tokens stay in macOS Keychain. They are excluded from TOML configuration, profile exports, SQLite records, and diagnostic error text. Linux development uses session-only credential storage. No model connection is automatically replaced with a paid alternative.

OpenAI-compatible requests serialize text-only system instructions as plain strings. This preserves their system role while allowing [LiteLLM's ChatGPT bridge](https://github.com/BerriAI/litellm/blob/v1.94.0/litellm/completion_extras/litellm_responses_transformation/transformation.py) to extract them as instructions; tools, images, and conversation roles are unchanged.

## Remote diagnostics

The desktop app sends operational diagnostics to `https://logging.ryserve.net/api/logs`, using Galaxy's `x-api-key` protocol and the `solar` service name. This includes startup/shutdown, frontend errors, task/model/tool outcomes, approval decisions, durations, usage counts, and correlation IDs. Each event identifies the build and process session. Chat messages, prompts, tool arguments/results, and provider credentials are excluded; errors are bounded and credential-like values are redacted.

For this development build, the shared RyServe logging credential is bundled in the Rust code. Set `RYSERVE_LOGGING_KEY` in the app process's environment to override it; explicitly setting it to an empty string disables remote logging. A Finder-launched app uses the bundled fallback unless that environment variable is supplied to the app process.

```sh
export RYSERVE_LOGGING_KEY='your-logging-key'
corepack pnpm tauri dev
# Or disable remote logging for this launch:
RYSERVE_LOGGING_KEY='' corepack pnpm tauri dev
```

Delivery runs on a separate worker with a bounded in-memory queue, short timeouts, and limited retries. An unavailable logging service does not block the UI or tasks, and undelivered events are not written to disk. Transport tests use a local mock server, without sending development test data to RyServe.

Rust panics use a separate priority delivery attempt capped at 700 milliseconds. The panic hook waits at most 900 milliseconds before continuing normal panic reporting, without depending on the crashing thread having a Tokio runtime. Delivery remains best effort if the service is unreachable or the logging worker itself fails.

## Tools and permissions

Built-in tools cover scoped file listing, reading, writing and search, HTTP retrieval, a dedicated Chromium browser, shell commands, MCP tools, screen capture, and macOS application inspection/control. Browser automation requires an installed Chrome or Chromium and uses a separate profile for each Orbit.

Every Orbit receives the built-in and connected MCP tool definitions; legacy tool-category selections no longer hide tools. **Ask** prompts for every tool. **Workspace** allows scoped tools and asks for host execution. **Trusted** allows host tools automatically. Delegated work inherits approval requirements and can access only project folders shared by the participating Orbits. Screen capture still requires explicit screen sharing.

An Orbit’s assigned projects are listed in its context and through `project_list`, with names, IDs, descriptions, and folder paths. File tools can access those folders without selecting a project for the conversation. `shell_run` accepts `workingDirectory` for running commands in an assigned project.

Built-in file operations use directory capabilities and reject path/symlink escapes. Shell programs, browser processes, and locally installed MCP servers run with the user's host privileges after authorization. Their working directories are not an OS sandbox. Native OS containment of arbitrary third-party executables still requires separate macOS qualification.

Screen sharing begins only through a user action and macOS's content picker. Application/window sharing supports observation. Computer control requires whole-display sharing and Accessibility authorization, and is bounded to that display. Sharing stops at task completion or through Solar or the system stop control.

MCP supports local stdio and remote Streamable HTTP. Add the server in Connections, enable it, and connect it; its tools become available to every Orbit. Server-initiated sampling and elicitation return explicit unsupported-capability errors.

## Local data and task recovery

Solar stores its managed files in `~/.solar`:

```text
~/.solar/
  config.toml                 # Master settings, connections, MCP, projects, routines
  solar.sqlite                # Shared conversations, project memory, task indexes
  skills/                     # Shared skill files
  mcp/<server-id>/             # Each local MCP server's home, cache, data and temp files
  projects/                   # Managed shared-project data
  cache/                      # Application cache
  logs/                       # Application logs
  orbits/<orbit-name>/
    config.toml               # Role, personality, model, permissions, project grants
    orbit.sqlite              # Private chat, memory, model history and run journals
    workspace/                # Agent-created working files
    skills/                   # Orbit-specific skill files
    memory/                   # Additional local memory files
    browser/                  # Dedicated browser profile
    cache/                    # Tool caches (config/data/state/tmp are created as needed)
```

The master TOML contains `[settings]` and `[[providers]]`, `[[projects]]`, `[[mcp]]`, `[[routines]]`, and `[[orbits]]` entries. Each master Orbit entry maps its stable `id` to a directory; the corresponding `config.toml` contains `version = 1` and an `[orbit]` table. Configuration files are authoritative on startup, and settings changes made in Solar update them atomically. Close Solar before editing them directly, then restart to load changes. Invalid configuration is reported rather than overwritten.

Orbit folder names come from their names, normalized to safe lowercase names; collisions receive a suffix. Changing an Orbit's display name keeps its directory stable so files, database handles, and browser sessions stay together. Shared conversations and project memory remain in the master database; private conversations, memories, and provider transcripts live in each Orbit's database. Shared project memory is visible only to explicitly granted project members. Directories use owner-only permissions on Unix. Connection secrets remain in Keychain as described above.

Tauri cache/log paths also point beneath `~/.solar`. Launcher and widget webviews use nonpersistent browser storage; their actual state is saved by Rust. Shell and local MCP process home/cache/config/temp locations stay in the managed directories. This redirects normal storage paths, and does not sandbox host executables or move files in external project folders. Skill folders provide file storage; this change does not add automatic skill loading.

Scheduled, folder-change, and task-completion routines run while Solar is running and the computer is awake. Missed intervals coalesce into one due run. Folder polling checks nested paths, skips symlinks, and supports up to 50,000 entries per watched project.

Tasks have a 30-minute deadline, bounded model turns and retries, and a configurable model concurrency limit. A computer-sharing session has one owner. Cancelled or interrupted tools receive explicit transcript results. Application restart marks interrupted tasks for review and preserves their history without replaying external actions.

Orbit exports use versioned JSON. Imports receive a fresh identity, require new provider/project assignments, and start in Ask mode.

## Validate

```sh
corepack pnpm run build
corepack pnpm run test:intro
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

Remote Orbit hosting is planned in [the native host and relay design](docs/remote-orbits.md), including guided SSH setup using existing SSH configuration, password fallback, launchd services, encrypted pairing, activity, and notifications. The [CloudFormation host template](infra/cloudformation/README.md) prepares an AWS machine for later deployment. Remote services are not implemented or deployed by this change.

- `crates/solar-agent-core` contains Galaxy-derived, UI-independent runtime contracts. Its old provider-run format remains readable for interrupted-task recovery; new Orbit tasks use Rig’s agent state machine.
- `crates/solar-providers` contains selected Rig adapters, request/stream translation, credential services, and provider regression fixtures. `RigOrbitRun` uses Rig’s serializable `AgentRun` for the model/tool loop and canonical conversation history, with a small receipt journal for host actions.
- `crates/solar-tools` contains local tool implementations, MCP transports, browser sessions, and the native macOS bridge.
- `src-tauri` owns authorization, persistence, scheduling, Orbit collaboration, desktop windows, and the frontend command boundary.
- `src` contains the React interface and code-drawn pixel-art characters.

Solar drives Rig’s documented step interface so each tool call still passes through current project access, approval preferences, and cancellation. The host supplies the Orbit preamble and tools, and explicitly executes approved work; `AgentRunner` hooks are not implicitly invoked by this step interface. On restart, unfinished work is closed with correlated tool results and is never automatically replayed.

Solar is licensed under AGPL-3.0-only. Galaxy-derived crates retain their provenance and license files. Rig is pinned to the Galaxy-compatible revision recorded in their manifests. Original Solar UI code and assets are distributed under the same project license.
