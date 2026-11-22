# Solar user guide

Solar is a desktop home for a team of AI companions called **Sprites**. Give each Sprite a purpose, personality, and model connection, then work with it in a direct message or bring several Sprites into a group.

Solar runs on your computer and stores its data in `~/.solar`. The model you choose may run locally or on a provider's servers. Messages and tool context needed for a task are sent to that model connection.

- [Install or update Solar](#install-or-update-solar)
- [Set up your first Sprite](#set-up-your-first-sprite)
- [Direct messages and groups](#direct-messages-and-groups)
- [Use the desktop companions](#use-the-desktop-companions)
- [Projects and shared folders](#projects-and-shared-folders)
- [Tools, approvals, and screen sharing](#tools-approvals-and-screen-sharing)
- [Memories and routines](#memories-and-routines)
- [Ideas for your team](#ideas-for-your-team)
- [Settings, backups, and reset](#settings-backups-and-reset)
- [Troubleshooting](#troubleshooting)
- [Get help](#get-help)

## Install or update Solar

### Requirements

The main desktop target is an **Apple Silicon Mac** running **macOS 14.2 or later**. You can check both in Apple menu → **About This Mac**. A chip named Apple M1, M2, M3, or another Apple M-series chip is Apple Silicon.

You do not need Rust, Node.js, or Xcode to use the downloaded app. Those tools are only needed to build Solar from source. Intel Mac installers are not currently provided.

Windows x64 and Linux x64 packages are also available as previews in new multi-platform builds. Older releases may contain only the Mac download. Windows installers are unsigned; workspace file tools and isolated shell, browser, and local MCP processes are not available on Windows yet. Linux process tools require Bubblewrap. Native screen/computer tools and notification audio currently require macOS. See [Windows and Linux builds](https://github.com/rkw6086/solar/blob/main/docs/desktop-builds.md) for the current limitations.

### Install on a Mac

1. Open [Solar Releases](https://github.com/rkw6086/solar/releases).
2. Choose the latest numbered release for the stable version. **Solar Beta** contains newer changes and may be less predictable.
3. Download the `.dmg` file for Apple Silicon (`aarch64`).
4. Open the downloaded disk image and copy **Solar** into **Applications**.
5. Open Solar from Applications and follow the welcome tour.

The official release process signs, notarizes, and staples the Mac installer. If macOS unexpectedly says a download is damaged or cannot be verified, see [App will not open](#app-will-not-open) before changing security settings.

### Windows and Linux previews

On Windows 10 or 11 x64, download the `x64-setup.exe` file and run the installer. It installs for the current user and can install the WebView2 runtime if needed. Windows may warn because this installer is unsigned.

On compatible Debian/Ubuntu x64 systems, download the `amd64.deb` file and install it using `sudo apt install ./Solar_VERSION_amd64.deb`. For the AppImage, install your distribution's `bubblewrap` package, mark the downloaded AppImage executable, and run it. Linux builds use Ubuntu 22.04 as their library baseline; their behavior also depends on the desktop environment and its sandbox policies.

### Optional: verify the download

Download `SHA256SUMS` from the same release and place it beside the DMG. In Terminal, change to that folder and run:

```sh
cd ~/Downloads
grep 'aarch64.dmg$' SHA256SUMS | shasum -a 256 -c
```

The DMG should report **OK**. If it does not, download both files again from the same release.

### Update an existing installation

1. Quit Solar using its menu-bar menu. Closing the main window alone leaves Solar running.
2. Download and open the new DMG.
3. Replace the Solar app in Applications and reopen it.

Replacing the application does not itself reset your `~/.solar` data. Stable and beta versions use the same data folder, so back up that folder before experimenting with a beta. Check the release notes for any version-specific update instructions.

## Set up your first Sprite

The welcome tour introduces Solar and guides you through connections, your first Sprite, an optional project, and desktop preferences. You can return to configuration from the main application later. Solar does **not** create a default team for you.

### 1. Add a model connection

Open **Connections** and choose the kind of connection you use:

| Connection | What to have ready |
| --- | --- |
| OpenAI-compatible or LiteLLM | Your service's API base URL, an API key if required, and a model ID. A compatible local model server can also use this connection type. |
| ChatGPT subscription | Complete the sign-in flow shown by Solar. |
| Claude subscription | Enable the experimental option in Settings and complete its sign-in flow. Read the access and account notices shown by Solar before enabling it. |

Use the URL and model information supplied by your provider or local server. A provider's API access and its chat website subscription can be different products; select the connection type that matches what you actually have.

### 2. Create a Sprite

Open **Your Sprites** and choose **Create Sprite**. Give it:

- **Name:** type one or generate a suggestion.
- **Personality:** describe the tone and working style you want, or generate a suggestion.
- **Tasks:** explain what it should help with, including useful boundaries and the kind of result you expect.
- **Model:** search the model field and select a model from your configured connections. Results show the model ID and provider name. Search supports multiple terms, `*`, and `?`.

For example:

> Help me research technical topics. Prefer original sources, keep notes concise, distinguish facts from assumptions, and ask when the scope is unclear. Be patient and direct.

When you save, the selected model turns those details into the Sprite's working instructions. Wait for the progress overlay to finish. A working model connection is required for that generation step.

You can edit a Sprite from its chat settings or by right-clicking its desktop planet. Changes to its name, role, personality, or tasks regenerate its instructions. Changing only its model, permissions, or project access does not. **Regenerate avatar** gives it a different face and planet appearance without changing its work or instructions.

### 3. Choose its access

Give the Sprite access to the projects it needs and choose when it should ask for approval. Start with a focused task and a small amount of shared data; you can expand access later.

If setup discovers familiar configuration folders such as `.config` or `.kube`, nothing is selected automatically. Choose only the items you want linked into all current and future Sprite workspaces. You can manage those choices later in **Projects → Shared folders**.

## Direct messages and groups

The left sidebar has **Direct messages** above **Groups**, with a divider between them. Its main list scrolls so larger teams and longer group lists fit; **Connections** and **Settings** stay available at the bottom.

- Select a Sprite under Direct messages to open its chat.
- Select a group to reopen that group's conversation.
- Choose **New group** below the group list to create a group. Select at least two enabled Sprites, name the conversation, and choose a lead Sprite. You can also choose a shared project.

The lead coordinates the group and can delegate work to its members. Adding a Sprite to a group does not bypass its project access or approval rules.

Give requests enough context to be useful: the goal, relevant project or files, expected output, and any actions you want it to ask about first. You can watch its tool cards as it works and answer approval requests in the conversation.

Chat follows new messages and tool output automatically. Scroll up to read earlier work; scroll back to the bottom when you want to follow live output again.

## Use the desktop companions

The desktop sun and Sprite planets appear while the main Solar window is hidden. Opening the main window hides the companions and closes their popup chats.

| Action | Result |
| --- | --- |
| Click the sun | Expand your Sprites, or gather them again. |
| Drag the sun | Choose the nearest of four display corners. The collapsed sun snaps flush to the usable screen edge. |
| Expand from a corner | The sun moves inward so the Sprite orbits have room. Gathering returns it to the saved corner. |
| Hover over a Sprite | Pause orbital movement so it is easier to select. |
| Click a Sprite | Open its popup chat near the planet. |
| Drag the popup chat header | Move that chat window. |
| Click away from a popup | Close the popup back into its Sprite; the Sprite stays expanded. |
| Right-click the sun | Open the main Solar window. |
| Right-click a Sprite | Open that Sprite's settings. |

Expanded Sprites stay out until you gather them; idle time and background clicks do not retract them. They orbit while expanded, with movement paused during Sprite selection and open popup chats. Hovering over the sun itself does not stop the orbits. Reduced motion can also keep the scene still.

The sun occasionally smiles, rolls, yawns, or receives a visit from a rocket, comet, or friendly flying saucer. These decorations are silent. The companions have no text labels or separate arrow launcher button.

### Notifications and sound

A Sprite can peek down from the top of the display when it needs permission, finishes a task, or encounters a problem. Solar places its own overlay around the display's camera-housing safe area; it does not modify the system Dynamic Island.

Open the alert to reach the relevant conversation. Dismissing an alert does **not** approve an action. If that conversation is already visible, Solar skips its matching alert and chime.

Each Sprite has a short synthesized notification sound. Use **Settings → Attention sound** for alert sound and volume, and **Settings → Welcome experience** for general sound effects and their volume. Reduced motion applies to desktop and welcome animations too.

## Projects and shared folders

These are two ways to make selected files available inside Sprite workspaces.

### Projects

1. Open **Projects** and add a project using the folder picker.
2. Choose the folder containing your work. If it is inside a Git repository, Solar uses the repository root.
3. Add helpful project context if you want.
4. Open each relevant Sprite's settings and give it access to that project.

The project's name comes from its root folder. Adding a project does not grant every Sprite access automatically. A Sprite can use an assigned project even when its direct chat has no selected project.

Projects are linked into the Sprite workspace rather than copied. Git changes made through an authorized task affect the real project. Tool cards can show file diffs so you can review what changed.

### Shared folders

Use **Projects → Shared folders → Share a folder** for reference material, configuration, or other files that are not best represented as a project.

- Share with selected Sprites or all current and future Sprites.
- Choose the location of the link inside their workspaces.
- **Read-only** is the default.
- Enabling editing means changes affect the original files on your computer.

The links are live: they do not create independent backup copies. Existing private workspace files are not overwritten. If a location is already occupied, use another path, such as a folder under `shared/`.

A shared configuration folder can contain credentials as well as preferences. Choose its audience and write access with that in mind.

### Let a Sprite request a file

You can ask a Sprite to find a file or configuration item in your home folder and request access to it. Home discovery searches names and relative paths without opening file contents. Adding the selected items as workspace links always requires your explicit approval, including in Trusted mode.

Review the source paths, workspace destinations, and read/write access before approving. Shell profiles may reference other files; Solar can include supported literal dependencies in the request. Dynamic includes may need a separate request.

## Tools, approvals, and screen sharing

Sprites can use tools for files, shell commands, web research, a dedicated browser, memory, routines, projects, collaboration, and connected MCP services. Computer interaction also depends on the operating-system permissions you grant.

| Approval setting | Behavior |
| --- | --- |
| **Ask** | Ask before each tool action. |
| **Workspace** | Allow scoped workspace tools and ask before host execution. |
| **Trusted** | Allow host tools automatically within their configured access. Home-file link requests still always require approval. |

These settings do not make an unassigned project available, share the whole home folder, or replace macOS screen-sharing permission.

Tool cards explain what is happening. Shell cards show the command, working folder, live output, and exit status. Web cards summarize searches and sources. MCP cards identify the service and command. File diffs are available for supported changes in accessible Git projects.

### Screen sharing and computer control

Start sharing explicitly and use macOS's content picker to choose what Solar can see. Sharing an application or window supports observation. Computer control requires sharing a complete display and granting Accessibility access in macOS System Settings.

Control is limited to the shared display. You can stop sharing through Solar or the system sharing control; sharing also stops when the task finishes.

### MCP services

MCP connects Solar to additional tool services. Add a server in **Connections**, enable it, and connect it. Solar supports local stdio servers and remote Streamable HTTP services.

For a local filesystem server's root-folder argument, use the literal `{workspace}` placeholder instead of your whole home folder. Solar substitutes the requesting Sprite's workspace. Remote services use their own authentication and access rules; the local filesystem boundary does not contain actions performed by a remote service.

## Memories and routines

### Memories

Use **Memory** for useful preferences, decisions, and project context that should survive a single conversation. Memories can be:

- **Solar System-wide:** available to every Sprite.
- **Shared with selected Sprites:** available to one or more chosen companions.
- **Project-specific:** associated with one or more projects and available subject to project access.

You can create or edit memories in the Memory screen, or ask a Sprite to update them. Be clear about who should be able to read a memory. A useful example is: “Remember for this project that exported reports must include a date and use plain language.”

### Routines

Ask a Sprite to create a routine; the app does not provide a separate manual routine-creation form. For example:

> Every weekday at 9 a.m., check the assigned project for new Markdown notes and summarize what changed. Do not edit the files.

The **Routines** screen shows the title and expandable Markdown details. You can pause, run, remove, or adjust an existing routine's schedule with the visual schedule editor. Ask the Sprite to change what the routine does.

Routines can run on a schedule, after project files change, or after a task completes. They run only while Solar is running and the computer is awake. Schedules use the computer's local time; missed scheduled occurrences coalesce into one due run. Routines use the Sprite's current access and approval setting, so a required approval can still pause the work.

## Ideas for your team

| Sprite | Example request |
| --- | --- |
| Personal assistant | “Turn these notes into a short plan for tomorrow. Ask about missing deadlines and draft any messages for me to review.” |
| Researcher | “Compare these approaches. Prefer original sources, link your references, and separate findings from assumptions.” |
| Writing partner | “Review this shared draft for clarity and consistency. Suggest changes first; preserve my tone.” |
| Project guide | “Read the assigned project's README and explain how its pieces fit together. Make a setup checklist without changing files.” |
| QA companion | “Review this change against the acceptance criteria. Run relevant checks if permitted, then report reproducible issues and any remaining test gaps.” |
| Group review | “Researcher: investigate the options. QA: challenge the assumptions. Lead: give me a concise recommendation and the unresolved questions.” |

For a first task, ask for a review or a plan before authorizing edits. This helps you see whether the Sprite understands the goal and has the right context.

## Settings, backups, and reset

### Where data lives

Solar stores its managed data in `~/.solar`, including master settings, conversations, memories, credentials, and Sprite workspaces. In Finder, choose **Go → Go to Folder…** and enter `~/.solar` to find it.

The master configuration is `~/.solar/config.toml`. Sprite configuration and private data live under `~/.solar/orbits/<sprite-folder>/`. The folder name `orbits` is retained for compatibility with earlier versions; it does not mean your Sprites share a workspace.

Credentials are stored in owner-only files under `~/.solar/credentials`, not encrypted by an operating-system credential vault. Keep backups private. A local model connection can keep model processing on your own machine; a remote provider receives the requests you send to it.

### Back up Solar

Quit Solar fully, then copy `~/.solar` to a private backup location. Back up external projects and shared folders separately: workspace links are not copies of those files. Sprite profile exports are useful for sharing configuration, but exclude saved connection secrets and are not a complete backup.

Prefer Settings for everyday changes. If you edit TOML files directly, quit Solar first and restart it afterward. Keep a backup before changing configuration by hand.

### Reset one Sprite or all of Solar

- **Reset Sprite**, in that Sprite's settings, clears its private conversations, memories, and activity while keeping its profile and workspace files. Shared group conversations remain.
- **Delete Sprite** removes that companion from the team.
- **Settings → Welcome experience → Replay welcome tour** shows setup again without wiping your data.
- **Settings → Start over → Reset Solar** asks for confirmation, then clears Solar-managed data and credentials and restarts setup.

Resetting Solar preserves original files in external projects and shared folders. It does not undo edits already made to those originals.

## Troubleshooting

### App will not open

Check that the Mac is Apple Silicon and runs macOS 14.2 or later. Download the DMG again from [Solar Releases](https://github.com/rkw6086/solar/releases) and, if needed, verify its checksum. A browser-only source preview is not the installed desktop app. If an official download still cannot be verified, report the exact macOS message rather than disabling system security protections.

### I do not see any Sprites

Solar starts with no default Sprites. Finish creating one in setup or **Your Sprites**. If you already created a team, check that desktop companions are enabled and hide the main Solar window; companions are hidden while it is open. Click the sun to expand them.

### My Sprite will not finish saving

Creating or changing its working instructions calls the selected model. Check the model connection, sign-in, and model availability in **Connections**. Wait for an error or completion before retrying. Changing only its model or access does not need instruction regeneration.

### Planets are not moving

Move the pointer away from a Sprite, close any open popup chats, and check the reduced-motion setting. Those states pause movement intentionally. Hovering over the sun should not pause the orbits. Expanded Sprites do not collapse merely because you are inactive.

### The widget is not where I dragged it

The sun snaps to the nearest display corner when you finish dragging. While expanded, it moves inward to keep the orbits visible. Gathering the Sprites returns it to the saved corner. This is intentional; free-form resting positions are not used.

### A Sprite cannot see my project or file

Check **Projects**, then that Sprite's project access. For another folder, add an appropriate shared-folder link or approve the Sprite's home-file request. Adding a project alone does not grant access to every Sprite. Ask the Sprite to use its workspace project path rather than a raw path outside its workspace.

If a shared folder moved, was deleted, or now points somewhere else, fix its entry in **Projects → Shared folders**. Solar rejects changed link targets rather than silently broadening access.

### A command works in my Terminal but fails for a Sprite

Sprites normally have separate configuration and CLI credentials. Open the Sprite's settings, choose **CLI sign-ins**, then **Open Sprite terminal** and sign in to the required tool there. Enter credentials in that terminal, not in chat.

Alternatively, explicitly share the necessary configuration. A read-only configuration cannot save a new login; making it writable means the tool can change the original shared configuration. Startup files may reference additional files that also need to be linked.

### A model or connection returns an error

Confirm the connection type, base URL, model ID, and saved credentials. Retry the provider's sign-in flow if the session expired. Check that your chosen service/model supports the task, including tool use when required. Experimental subscription access may change or stop working independently of Solar.

### Screen sharing or computer control does not work

Start a fresh sharing session through the macOS picker. For control, share a complete display rather than only a window, and check Solar's Accessibility permission in System Settings. A stopped or revoked session needs to be shared again. Keep the target window completely on the shared display when using keyboard control.

### A routine did not run

Check that the routine and its Sprite are enabled, Solar is still running, and the computer is awake. Review its schedule and local time zone, project access, and any pending approval. Closing the main window is fine; quitting Solar stops local routines.

### I pulled source changes but still see old behavior

Stop the development command and quit every running Solar instance, including the menu-bar app, then restart the source build. Frontend hot reload alone does not replace an already running Rust process. The downloaded app in Applications is also separate from a development build.

For source-build prerequisites and commands, see the [repository README](https://github.com/rkw6086/solar#develop). The repository pins Rust and pnpm versions for consistent builds.

## Get help

Check [release notes](https://github.com/rkw6086/solar/releases) first, then open an issue at [Solar Issues](https://github.com/rkw6086/solar/issues) if the problem persists.

Include the Solar version from **About Solar**, your macOS version and chip, whether you use an installed app or a source build, steps to reproduce, and the exact error text. For a model problem, include the connection type and model ID. Remove passwords, API keys, tokens, private file contents, and personal paths from anything you share.

Solar currently runs its Sprites on the local computer. Remote/cloud Sprite hosting and a relay service are planned, not a shipped setup option. Browser preview is for trying the interface; model calls and operating-system tools require the native app.
