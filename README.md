# Solar

Your personal team of Sprites, at home on your desktop.

[Download Solar](https://github.com/rkw6086/solar/releases) · [Public source](https://github.com/rkw6086/solar)

Solar is a local Rust and Tauri application with a React/TypeScript interface. Each Sprite has a role, personality, model connection, private memory, workspace, and approval preferences. A pixel-art sun gathers your Sprites and brings them onto the desktop.

## Develop

The primary target is Apple Silicon running macOS 14.2 or later. Install Rust, Node.js 22.22.2+ or 24.15+ (LTS), and Xcode command-line tools.

Use [Corepack](https://github.com/nodejs/corepack) to download and cache the exact pnpm version declared in `package.json`. If Corepack is missing or outdated, install it once with `npm install -g corepack@0.36.0` (Node.js 25+ no longer bundles Corepack).

```sh
corepack pnpm install --frozen-lockfile
corepack pnpm tauri dev
```

`packageManager` pins pnpm with an integrity hash; `pnpm-lock.yaml` pins dependencies. Corepack selects the same pnpm release on every machine. The `corepack pnpm` commands work without installing pnpm globally or enabling command shims. If you prefer plain `pnpm` commands, run `corepack enable` once.

Linux development also requires Bubblewrap (`bwrap`) for agent process isolation, alongside Tauri’s native system dependencies. A browser installation must be permitted to use its own sandbox by the host’s security policy. Solar does not fall back to an unsandboxed process.

The native preparation hook builds the Swift screen/accessibility helper and stages it as a Tauri sidecar. The helper is bundled with the application for distribution.

## First launch

Solar opens a borderless, screen-sized window on black as white ripples gather into a new star. The welcome tour then introduces your Sprites and walks through model connections, the first Sprite, a project folder, and desktop preferences. The whole tour uses that window without entering macOS fullscreen or creating a fullscreen Space, then restores the launcher's previous size, position, and window controls. Each setup step saves real settings and can be skipped or resumed; existing profiles and files are preserved when replaying the tour.

The welcome uses a restrained retro-console style: white vector geometry, amber/cyan accents, small monospaced labels, and faint stationary screen texture. Design references include the [Tektronix 4010 terminal](https://www.computerhistory.org/brochures/doc-4372956d27050/) and [vintage Roland synthesizers](https://www.roland.com/us/products/ju-06/); the artwork and synthesized sound effects are original.

There is no background music. Each ripple reaching the center makes a quiet glass-like drop, followed by a brief airy major chord as the sun forms. The sun then winks and plays a randomly varied Sprite-style chime. Controls have a short, dry click. These effects are synthesized locally and timed from the animation's own timeline; they stop when you hide, skip, or leave the introduction. The design follows Apple's guidance for [subtle, varied interaction sounds that match visual transitions](https://developer.apple.com/videos/play/wwdc2023/10271/). Reduced motion shows a still sun with one short formation cue and skips the animated wink and ripple sounds.

The welcome's **Sound Effects On / Off** button controls effects throughout Solar, including Sprite chimes. Find **Replay welcome tour**, **Sound effects**, and its volume in **Settings → Welcome experience**. Sprite chimes also retain their own category switch and volume. Replay keeps your existing setup. **Settings → Start over → Reset Solar** instead shows one confirmation modal, then stops active work, clears Solar’s saved credentials and all app-managed data, and restarts from the opening animation. Files in external project folders are preserved.

For the browser-only UI preview, run the following and open the URL Vite prints.

```sh
corepack pnpm run dev
```

The browser preview uses isolated local data and starts with no Sprites, just like the desktop app. Model calls and operating-system tools execute in the desktop application.

## First session

1. Open **Connections** and configure LiteLLM, an OpenAI-compatible endpoint, or subscription sign-in.
2. Create your first Sprite with a name, personality, and tasks (type your own, or shuffle name/personality suggestions). Search all model connections in one field; model labels include their provider. Solar does not create Sprites automatically.
3. Choose when it should ask for approval. Every Sprite can request all available tools. Choose a project folder in **Projects**; Solar uses the Git repository root when the folder is inside a repository, and names the project after its root folder. Assign access in the Sprite's settings. Assigned projects are available even in individual chats with no selected project.
4. Open an individual conversation or create a group with a lead Sprite. The lead can delegate to members of that group.
5. Use **Bring Sprites to desktop**, or click the sun. Planets expand onto resonant paths around it; drag the sun to move the whole group, and click a planet to open its mini-chat. Each Sprite has stable facial features.

Press **Create Sprite** and the selected model automatically builds its role and private system instructions from your details. Changing its name, role, personality, or tasks regenerates those instructions; changing only the model, permissions, or project access keeps them intact and does not call a model. The generated prompt stays hidden; there is no prompt review or approval step. While instructions are being prepared, a progress overlay keeps the form open until the save succeeds or an error lets you retry. A model connection is required only when generating instructions. Your original tasks are kept separately from the generated instructions. Model search matches both IDs and provider names, supports multiple search terms, and accepts `*` and `?` wildcards.

Every chat has a Sprite settings button, including desktop popup chats. You can also right-click a desktop planet to open its settings. Edit its name, role, personality, tasks, model, project access, and approval preferences. **Reset Sprite** clears its private conversations, memories, and activity while keeping its profile, workspace files, and shared group conversations. **Delete Sprite** removes it from the team. Both actions ask once for confirmation. Earlier saved profiles remain until you remove them; startup and the welcome tour never add default Sprites.

Tool cards show what a Sprite is doing without raw protocol data. Shell cards show the command, working folder, live output, and exit status; output follows the latest line until you scroll up. Web cards summarize the search or source, and MCP cards identify the service and command. Cards can be collapsed while work continues. File changes observed during shell commands, file writes, and MCP calls appear as readable diffs for accessible Git projects. These previews compare against the files immediately before the call, preserve existing edits, and do not change Git's index. Ignored files, symlinks, large/binary files, and very large changes may be omitted or shortened; simultaneous edits by another program may also appear.

The launcher closes into the menu bar. Sun and planet widgets appear while the launcher is hidden; opening the launcher hides them and closes popup chats. **Bring Sprites to desktop** hides the launcher and expands the planets. Each expansion starts from the same positions for the same Sprite roster, then slowly orbits using the same resonance as the launcher. Multiple rings accommodate larger teams such as 18 Sprites; the group stays together at display edges. Hover pauses the formation. Desktop companions have no text labels or extra launcher button; right-click the sun to open Solar. Clicking away from a chat closes it back into its Sprite and leaves the planets visible. Clicking the sun, clicking away with no chats open, or leaving the group idle for 30 seconds once all chats close gathers the Sprites. Open chats pause motion. The collapsed sun occasionally rolls, yawns, smiles, or welcomes a larger pixel rocket, a comet, or a friendly flying saucer. These silent visits stay out of the way and respect reduced motion. Quit from the menu-bar menu to stop Solar and its owned tool processes.

The launcher's **Solar system** home shows your star and the Sprites you create moving on paths with resonant periods. Select a planet to open its chat. With no Sprites, the star invites you to create your first teammate. Returning from a conversation also shows the system above your conversation history. Motion pauses when you interact with the planets or hide the page, and respects reduced-motion preferences. Existing Sprite profiles are preserved.

When a Sprite needs permission, encounters a task problem, or finishes your request, it peeks down from the top of the display. Solar uses the built-in display when available, reads the actual camera-housing safe area, and keeps the character and controls below the notch. Displays without a notch use a top-center panel below the menu bar. Opening the alert takes you to the relevant conversation; dismissing it never approves a tool. Multiple alerts queue, with permission requests first.

When that conversation is already visible in the launcher or a popup chat, Solar skips its alert and chime. Approval controls remain in the chat, and unrelated conversations can still notify you.

While the Sprites are collapsed, each Sprite plays its own stable, synthesized five-note signature. Every chime lasts **800 milliseconds**, including its release tail. **Settings → Attention sound** controls sound and volume, and the Sprite picker previews each character's alert. Reduced motion also applies to the peek animation. Sound playback uses macOS; browser/Linux previews show the animation without native audio playback.

This build presents a Solar-owned overlay, including on Golden Gate. Apple's native [ActivityKit](https://developer.apple.com/documentation/activitykit) integration on macOS 27.3+ requires a separate WidgetKit extension; Solar does not currently ship that extension or control the system Dynamic Island.

## Connections

- **OpenAI-compatible and LiteLLM** connections accept a base URL, optional API key, and discovered or manually entered model IDs. Existing local model servers can use the same interface.
- **ChatGPT subscription** uses native Rust/Rig streaming with Solar-owned OAuth and credential refresh, adapted from Galaxy.
- **Claude subscription** uses Galaxy's native Rig Messages compatibility path. It is experimental and unsupported by Anthropic for third-party subscription use. The application requires explicit enablement and displays the possible account/access restriction warning.

Provider API keys, OAuth tokens, and MCP secrets stay in private files under `~/.solar/credentials` on every platform. Unix folders are owner-only (0700) and files are owner-read/write (0600), with atomic replacement and symlink checks. These files are not encrypted by an operating-system vault. They are excluded from TOML configuration, profile exports, SQLite records, and diagnostic error text. Solar does not read or migrate earlier Keychain entries; reconnect existing providers and re-enter MCP secrets once after updating. No model connection is automatically replaced with a paid alternative. macOS's Security framework remains a transitive TLS dependency for server-certificate validation, not credential lookup. Release signing still uses its temporary build-runner keychain; the installed app does not use it.

OpenAI-compatible requests serialize text-only system instructions as plain strings. This preserves their system role while allowing [LiteLLM's ChatGPT bridge](https://github.com/BerriAI/litellm/blob/v1.94.0/litellm/completion_extras/litellm_responses_transformation/transformation.py) to extract them as instructions; tools, images, and conversation roles are unchanged.

## Remote diagnostics

When configured, the desktop app sends operational diagnostics to `https://logging.ryserve.net/api/logs`, using Galaxy's `x-api-key` protocol and the `solar` service name. This includes startup/shutdown, frontend errors, task/model/tool outcomes, approval decisions, durations, usage counts, and correlation IDs. Each event identifies the build and process session. Chat messages, prompts, tool arguments/results, and provider credentials are excluded; errors are bounded and credential-like values are redacted.

Remote logging is disabled by default. To enable it, supply your own `RYSERVE_LOGGING_KEY` in the app process's environment before startup. An unset, blank, or invalid value leaves logging disabled. No logging credential is bundled in the source or published app. Finder-launched apps need the variable supplied to their process; exporting it in a shell only configures apps launched from that shell.

```sh
export RYSERVE_LOGGING_KEY='your-logging-key'
corepack pnpm tauri dev
# Or disable remote logging for this launch:
RYSERVE_LOGGING_KEY='' corepack pnpm tauri dev
```

Delivery runs on a separate worker with a bounded in-memory queue, short timeouts, and limited retries. An unavailable logging service does not block the UI or tasks, and undelivered events are not written to disk. Transport tests use a local mock server, without sending development test data to RyServe.

Rust panics use a separate priority delivery attempt capped at 700 milliseconds. The panic hook waits at most 900 milliseconds before continuing normal panic reporting, without depending on the crashing thread having a Tokio runtime. Delivery remains best effort if the service is unreachable or the logging worker itself fails.

## Tools and permissions

Solar advertises 30 built-in tools, plus tools discovered from connected MCP servers. Built-in tools cover scoped file listing, reading, writing and search, HTTP retrieval, a dedicated Chromium browser, shell commands, screen capture, macOS application inspection/control, projects, Sprite collaboration, memory, and routines. Browser automation requires an installed Chrome or Chromium and uses a separate profile for each Sprite.

Solar's dedicated automation browser also avoids macOS Safe Storage prompts using Chromium's documented automation switch. Its profile remains private to the Sprite, and password/passkey saving is disabled. Browser cookies use Chromium's mock storage encryption, which is not a secret-backed vault; existing browser sessions may need a fresh sign-in. Your normal browser profile is not changed. HTTPS still validates certificates against the platform's trusted roots.

Every Sprite receives the built-in and connected MCP tool definitions; legacy tool-category selections no longer hide tools. **Ask** prompts for every tool. **Workspace** allows scoped tools and asks for host execution. **Trusted** allows host tools automatically, except requests to add home files/folders, which always need explicit approval of the live links. Delegated work inherits approval requirements and can access only project folders shared by the participating Sprites. Screen capture still requires explicit screen sharing.

A Sprite’s assigned projects are listed in its context and through `project_list`, with names, IDs, descriptions, and workspace paths under `projects/<project-id>`. Projects are live symlinks, not copies. File tools and `shell_run.workingDirectory` accept paths inside the Sprite workspace, including these links; raw host paths are rejected. A chat does not need a selected project to use an assigned project.

Shell commands use the account's default shell with interactive login startup, while keeping each Sprite's own home, configuration, and CLI credentials. They load the Sprite's startup files, including its own `.zshrc` or Bash login profile; new Bash homes get a profile that loads their own `.bashrc`. Unselected personal startup files, authentication variables, and SSH agents are not inherited. Explicitly selected shared configuration and profiles are used through their workspace links. Open **Sprite settings → CLI sign-ins → Open Sprite terminal** to configure a tool in that Sprite's environment, then close the terminal and retry the Sprite. For GitLab, use `glab auth login --git-protocol https --insecure-storage` so the login uses the selected file configuration rather than the system keychain. It stays private unless you explicitly select a writable shared configuration; a read-only share cannot save a new login. Solar sets `GLAB_USE_KEYRING=false` for these sessions; Git Credential Manager uses a private file store. Git/SSH use private configuration and key paths, and SSH explicitly disables Keychain use even if a copied profile requests it. Solar removes recognized `osxkeychain` helper entries from its private Git configuration while keeping other helpers and original user/project files intact. Configuration and credentials remain separate by default. Selecting a shared configuration folder deliberately shares its contents; read-only access is the default.

**Shared folders** in Projects link selected folders into all current/future Sprites or a selected set. The workspace location can be customized, and existing private files are never overwritten. Read-only is the default; enabling writes means changes affect the original files on your computer. The welcome tour discovers existing `.config`, `.kube`, common global agent folders, and shell/Git configuration by metadata only. Nothing is selected automatically. Check the items to include for all Sprites and choose **Share & continue**. The same choices can be managed later in Projects.

`search_user_home` finds names and relative paths without reading file contents or traversing symlink directories. `pull_from_user_home` requests selected results as live workspace links. Every request waits for an explicit link-list approval, even in Trusted mode. Profiles can include bounded, literal `source` dependencies; no profile code is executed and contents are not exposed by discovery. Links default to read-only, preserve the originals, and are revalidated before approval is applied. Existing private files cause a safe alternate workspace location instead of replacement. Dynamic includes require a separate explicit request.

Sprites manage their own routines through `routine_list`, `routine_create`, `routine_update`, `routine_delete`, and `routine_run`. Only Sprites create routines. Ask a Sprite to schedule a task, react to project folder changes, or follow up after a successful user task. The Routines screen shows a short title, expandable Markdown instructions, and controls to pause, run, remove, or adjust the schedule. A visual cron editor translates choices into a five-field schedule in the computer's local time. Updates can pause or resume a routine; “run now” queues one run even while paused. Routines run while Solar is open and the computer is awake, using the Sprite's current project access and approval preference. Routine runs do not trigger completion routines again. Delegated tasks can inspect their own accessible routines but cannot change or launch them, because a future routine would otherwise escape the delegation's access limits.

Built-in file operations use directory capabilities, require workspace paths, validate registered link targets, and reject nested symlink escapes. Shell processes use a filesystem allowlist on Linux (Bubblewrap) and macOS (Seatbelt), including the private workspace, approved shared targets, and read-only executable/runtime assets. Unshared home files, other Sprites' files, and unassigned projects are denied. Missing sandbox support fails closed. Changing grants cancels active work and invalidates cached browser/MCP sessions.

On macOS, approved symlink targets must also be allowed by their canonical host paths; shell code can address those same approved trees directly. This is a filesystem scope, not a virtual machine or a guarantee that an approved external service is isolated. Native screen/computer interaction is separately authorized host access, and remote HTTP MCP services enforce their own server-side access. Native macOS enforcement and click-away behavior need acceptance testing on a Mac.

Screen sharing begins only through a user action and macOS's content picker. Application/window sharing supports observation. Computer control requires whole-display sharing and Accessibility authorization, and is bounded to that display. Sharing stops at task completion or through Solar or the system stop control.

MCP supports local stdio and remote Streamable HTTP. Add the server in Connections, enable it, and connect it; its tools become available to every Sprite. Local discovery runs in a private service home, and agent calls use separate processes confined to each Sprite’s workspace and approved links. For a filesystem server whose argument is its root folder, use `"args": ["{workspace}"]` (or `${SOLAR_WORKSPACE}`) instead of a fixed host path. Solar substitutes only that literal placeholder and sets the `SOLAR_WORKSPACE` environment variable; it does not evaluate shell expressions. The discovery process receives its private service directory, and each Sprite receives its own workspace. Absolute unshared path arguments are rejected with instructions to use the placeholder or grant access. HTTP MCP services retain their own remote access rules. Server-initiated sampling and elicitation return explicit unsupported-capability errors.

## Local data and task recovery

Solar stores its managed files in `~/.solar`:

```text
~/.solar/
  config.toml                 # Master settings, connections, MCP, projects, routines
  solar.sqlite                # Shared conversations, project memory, task indexes
  credentials/                # Private provider, subscription, and MCP secrets
  skills/                     # Shared skill files
  mcp/<server-id>/             # Each local MCP server's home, cache, data and temp files
  projects/                   # Managed shared-project data
  cache/                      # Application cache
  logs/                       # Application logs
  orbits/<orbit-name>/
    config.toml               # Role, personality, model, permissions, project grants
    orbit.sqlite              # Private chat, memory, model history and run journals
    workspace/                # Agent-created working files
    skills/                   # Sprite-specific skill files
    memory/                   # Additional local memory files
    browser/                  # Dedicated browser profile
    cache/                    # Tool caches (config/data/state/tmp are created as needed)
```

Sprites were previously called Orbits. Existing storage paths, TOML keys, exported profile fields, and internal tool names retain `orbit` for compatibility; no files need to move.

The master TOML contains `[settings]` and `[[providers]]`, `[[projects]]`, `[[mcp]]`, `[[routines]]`, and `[[orbits]]` entries. Each master Sprite entry maps its stable `id` to a directory; the corresponding `config.toml` contains `version = 1` and an `[orbit]` table. Configuration files are authoritative on startup, and settings changes made in Solar update them atomically. Close Solar before editing them directly, then restart to load changes. Invalid configuration is reported rather than overwritten.

Sprite folder names come from their names, normalized to safe lowercase names; collisions receive a suffix. Changing a Sprite's display name keeps its directory stable so files, database handles, and browser sessions stay together. Shared conversations and shared memories remain in the master database; private conversations, memories, and provider transcripts live in each Sprite's database. Memories can be Solar System-wide (every Sprite can read and update them), shared with several selected Sprites, or attached to several selected projects. Project memories require project access; delegated work retains the parent's access limits. Changing a memory's sharing moves its stored copy without duplication. Directories use owner-only permissions on Unix. Connection secrets remain in the private credentials folder described above.

Tauri cache/log paths also point beneath `~/.solar`. Launcher and widget webviews use nonpersistent browser storage; their actual state is saved by Rust. Shell and local MCP process home/cache/config/temp locations stay in the managed directories. The filesystem allowlist described above applies to agent processes; external project and shared-folder files remain in place and are reached by live links. Skill folders provide file storage; this change does not add automatic skill loading.

Scheduled, folder-change, and task-completion routines run while Solar is running and the computer is awake. Missed scheduled occurrences coalesce into one due run. Folder polling checks nested paths, skips symlinks, and supports up to 50,000 entries per watched project.

Tasks have a 30-minute deadline, bounded model turns and retries, and a configurable model concurrency limit. A computer-sharing session has one owner. Cancelled or interrupted tools receive explicit transcript results. Application restart marks interrupted tasks for review and preserves their history without replaying external actions.

Sprite exports use versioned JSON. Imports receive a fresh identity, require new provider/project assignments, and start in Ask mode.

## Validate

```sh
corepack pnpm run build
corepack pnpm run test:intro
cargo fmt --all -- --check
cargo test --workspace --no-default-features
cargo check -p solar
```

On Linux with Homebrew first in PATH, system GTK/WebKit discovery may need `PKG_CONFIG=/usr/bin/pkg-config`. The Rust application tests use loopback fixtures and dummy credentials.

Native acceptance needs an actual Mac. Verify screen-picker cancellation/revocation, Accessibility controls, keyboard focus, transparent hit regions, expand/collapse animation, attention chimes and camera-housing placement, Spaces/fullscreen behavior, and mixed-scale displays before distributing a build. The Swift helper cannot be exercised on Linux.

The main window uses the normal system title bar and keeps its close-to-menu-bar behavior. Widget popup chats are borderless, open near their planet with a planet-to-window transition, and can be moved by dragging the header. Clicking elsewhere folds the window back to its planet; conversations and ongoing work remain saved. Chat follows new text, live tool output, and layout changes until you deliberately scroll up, and reopening a chat starts at the latest message. Native screen selection and profile saving keep their popup open until they finish. The opening animation and setup screens retain their borderless presentation.

## Package for macOS

Official releases use stable Semantic Versioning tags such as **`0.1.0`**, without a `v` prefix. The tag is the app version shown in **About Solar**. All Rust crates and app manifests stay in sync through the release helper:

```sh
corepack pnpm run release:prepare patch
corepack pnpm run version:check
```

Add matching notes to `CHANGELOG.md`, commit and push `main`, then run `corepack pnpm run release:tag VERSION --push` with the prepared version. Use `minor`, `major`, or an exact version in place of `patch` when appropriate. The initial release is `0.1.0`; prerelease tags are not currently supported.

All builds and checks run on GitHub Actions. Stable releases build when Gitea sync publishes a new version tag. The build workflow checks current `main` every night at **3:00 a.m. America/Chicago**, including daylight saving time; it skips a complete beta when the source has not changed. Run **Build Solar** manually on `main` to rebuild beta. Pull requests build without Apple credentials.

[GitHub Releases](https://github.com/rkw6086/solar/releases) retains **one current stable release and one rolling `beta` release**. Each contains a signed, notarized, stapled DMG and its checksum. New downloads are verified before old release records and assets are deleted. Numbered Git tags remain permanent source references; only the `beta` channel moves. Temporary package transfers expire after one day and are deleted after successful publication.

Beta DMGs include the source commit in their filename, and About Solar shows the beta channel and commit. Stable and beta share the same app identifier and `~/.solar` data; install the stable DMG again to return to the stable build.

Gitea remains the primary source: push only to `origin`, and the [sync pipeline](docs/github-sync.md) reconciles branches with public GitHub and copies immutable version tags outward. See [macOS release setup](docs/macos-release.md) for GitHub-hosted builds, Developer ID signing, notarization, caches, and repository secrets.

## Architecture and license

Remote Sprite hosting is planned in [the native host and relay design](docs/remote-orbits.md), including guided SSH setup using existing SSH configuration, password fallback, launchd services, encrypted pairing, activity, and notifications. The [CloudFormation host template](infra/cloudformation/README.md) prepares an AWS machine for later deployment. Remote services are not implemented or deployed by this change.

- `crates/solar-agent-core` contains Galaxy-derived, UI-independent runtime contracts. Its old provider-run format remains readable for interrupted-task recovery; new Sprite tasks use Rig’s agent state machine.
- `crates/solar-providers` contains selected Rig adapters, request/stream translation, credential services, and provider regression fixtures. `RigOrbitRun` uses Rig’s serializable `AgentRun` for the model/tool loop and canonical conversation history, with a small receipt journal for host actions.
- `crates/solar-tools` contains local tool implementations, MCP transports, browser sessions, and the native macOS bridge.
- `src-tauri` owns authorization, persistence, scheduling, Sprite collaboration, desktop windows, and the frontend command boundary.
- `src` contains the React interface and code-drawn pixel-art characters.

Solar drives Rig’s documented step interface so each tool call still passes through current project access, approval preferences, and cancellation. The host supplies the Sprite preamble and tools, and explicitly executes approved work; `AgentRunner` hooks are not implicitly invoked by this step interface. On restart, unfinished work is closed with correlated tool results and is never automatically replayed.

Solar is licensed under AGPL-3.0-only. Galaxy-derived crates retain their provenance and license files. Rig is pinned to the Galaxy-compatible revision recorded in their manifests. Original Solar UI code and assets are distributed under the same project license.
