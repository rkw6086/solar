//! Filesystem allowlist for every agent-owned local process. Approved links
//! remain live; their canonical targets are the only non-runtime host trees
//! visible to the child. This is not a network or native desktop sandbox.
use crate::{ToolContext, files};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Scope {
    working_directory: PathBuf,
    reads: Vec<PathBuf>,
    writes: Vec<PathBuf>,
    readonly: Vec<PathBuf>,
    denied: Vec<PathBuf>,
}

pub(crate) fn fingerprint(context: &ToolContext) -> Result<String, String> {
    files::working_directory(".", context)?;
    for mount in &context.mounts {
        files::validate_mount(context, mount)?;
    }
    serde_json::to_string(&(
        &context.workspace,
        &context.data_directory,
        &context.mounts,
        &context.denied_roots,
    ))
    .map_err(|_| "Cannot identify process filesystem scope".into())
}

pub(crate) fn command(
    executable: &Path,
    arguments: &[OsString],
    context: &ToolContext,
    extra_writable: &[PathBuf],
) -> Result<tokio::process::Command, String> {
    let executable = resolve_executable(executable)?;
    if files::denied(
        &executable
            .canonicalize()
            .map_err(|_| "Executable is unavailable")?,
        context,
    ) {
        return Err("The executable is inside an unassigned project".into());
    }
    let scope = scope(context, &executable, extra_writable)?;
    scoped_command(&executable, arguments, &scope, false)
}

/// Validate literal absolute MCP path arguments before a server starts. The
/// OS allowlist remains authoritative for paths assembled by server code.
pub(crate) fn process_argument_path_allowed(
    executable: &Path,
    path: &Path,
    context: &ToolContext,
    extra_writable: &[PathBuf],
) -> Result<bool, String> {
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Ok(false);
    }
    let executable = resolve_executable(executable)?;
    let scope = scope(context, &executable, extra_writable)?;
    let mut existing_parent = path.to_owned();
    let mut missing = Vec::new();
    while !existing_parent.exists() {
        let Some(name) = existing_parent.file_name() else {
            return Ok(false);
        };
        missing.push(name.to_os_string());
        if !existing_parent.pop() {
            return Ok(false);
        }
    }
    let mut target = existing_parent
        .canonicalize()
        .map_err(|_| "Cannot resolve a local MCP path argument")?;
    for name in missing.into_iter().rev() {
        target.push(name);
    }
    Ok(!scope.denied.iter().any(|root| target.starts_with(root))
        && scope.reads.iter().any(|root| target.starts_with(root)))
}

/// Chromium needs display transport and its own nested sandbox. These
/// allowances belong only to the dedicated browser, never to shell/MCP tools.
pub(crate) fn browser_command(
    executable: &Path,
    arguments: &[OsString],
    context: &ToolContext,
    extra_writable: &[PathBuf],
) -> Result<tokio::process::Command, String> {
    let executable = resolve_executable(executable)?;
    if files::denied(
        &executable
            .canonicalize()
            .map_err(|_| "Executable is unavailable")?,
        context,
    ) {
        return Err("The browser executable is inside an unassigned project".into());
    }
    let mut scope = scope(context, &executable, extra_writable)?;
    if let Some(directory) = executable.parent().filter(|directory| {
        directory.join("icudtl.dat").is_file()
            && [
                "resources.pak",
                "headless_lib_data.pak",
                "chrome_100_percent.pak",
            ]
            .iter()
            .any(|file| directory.join(file).is_file())
    }) {
        // Standalone Chromium distributions keep their runtime assets beside
        // the executable. Do not grant an arbitrary executable's parent.
        scope.reads.push(directory.to_owned());
    }
    #[cfg(target_os = "linux")]
    let display = linux_display_runtime();
    #[cfg(target_os = "linux")]
    {
        scope.reads.extend(display.paths);
        for value in ["/etc/fonts", "/var/cache/fontconfig"] {
            if let Some(path) = existing(Path::new(value)) {
                scope.reads.push(path);
            }
        }
    }
    #[cfg(target_os = "macos")]
    for value in ["/Library/Fonts", "/Library/ColorSync/Profiles"] {
        if let Some(path) = existing(Path::new(value)) {
            scope.reads.push(path);
        }
    }
    let mut command = scoped_command(&executable, arguments, &scope, true)?;
    command.env_clear();
    #[cfg(target_os = "linux")]
    for (name, value) in display.environment {
        command.env(name, value);
    }
    Ok(command)
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct DisplayRuntime {
    paths: Vec<PathBuf>,
    environment: Vec<(&'static str, OsString)>,
}

#[cfg(target_os = "linux")]
fn linux_display_runtime() -> DisplayRuntime {
    linux_display_runtime_with(
        std::env::var_os("DISPLAY"),
        std::env::var_os("XAUTHORITY").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var_os("WAYLAND_DISPLAY"),
    )
}

#[cfg(target_os = "linux")]
fn linux_display_runtime_with(
    display: Option<OsString>,
    authority: Option<PathBuf>,
    home: Option<PathBuf>,
    runtime: Option<PathBuf>,
    wayland: Option<OsString>,
) -> DisplayRuntime {
    use std::os::unix::fs::FileTypeExt;
    let mut result = DisplayRuntime::default();
    if let Some(display) = display {
        let text = display.to_string_lossy();
        let local = text
            .strip_prefix(':')
            .or_else(|| text.strip_prefix("unix:"));
        if let Some(number) = local
            .and_then(|value| value.split('.').next())
            .filter(|value| {
                !value.is_empty() && value.len() <= 4 && value.bytes().all(|b| b.is_ascii_digit())
            })
        {
            let socket = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
            if std::fs::metadata(&socket).is_ok_and(|meta| meta.file_type().is_socket()) {
                result.paths.push(socket);
            }
        }
        result.environment.push(("DISPLAY", display));
        let auth = authority.or_else(|| home.map(|home| home.join(".Xauthority")));
        if let Some(auth) = auth.filter(|path| {
            std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() <= 1_048_576)
        }) {
            if let Ok(auth) = auth.canonicalize() {
                result
                    .environment
                    .push(("XAUTHORITY", auth.as_os_str().to_owned()));
                result.paths.push(auth);
            }
        }
    }
    if let (Some(runtime), Some(display)) = (runtime, wayland) {
        let display_path = PathBuf::from(&display);
        if runtime.is_absolute()
            && display_path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            let socket = runtime.join(&display_path);
            if std::fs::metadata(&socket).is_ok_and(|meta| meta.file_type().is_socket()) {
                result.paths.push(socket);
                result
                    .environment
                    .push(("XDG_RUNTIME_DIR", runtime.into_os_string()));
                result.environment.push(("WAYLAND_DISPLAY", display));
            }
        }
    }
    result
}

fn resolve_executable(value: &Path) -> Result<PathBuf, String> {
    let path = if value.is_absolute() {
        value.to_owned()
    } else if value.components().count() == 1 {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join(value))
            .find(|path| path.is_file())
            .ok_or("The configured executable was not found in PATH")?
    } else {
        return Err("Use an absolute executable path or a command in PATH".into());
    };
    if !path.is_file() {
        return Err("The configured executable is not a regular file".into());
    }
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("The configured executable cannot contain parent traversal".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "The configured executable is unavailable")?;
    // Only a verified Python venv needs its invocation spelling to locate
    // pyvenv.cfg. Generic shell aliases must not become dangling mount targets.
    if python_venv_launcher(&path) {
        Ok(path)
    } else {
        Ok(canonical)
    }
}

fn existing(path: &Path) -> Option<PathBuf> {
    path.canonicalize()
        .ok()
        .filter(|path| path.parent().is_some())
}

fn add_runtime_path(roots: &mut BTreeSet<PathBuf>, path: &Path) {
    if let Some(canonical) = existing(path) {
        roots.insert(path.to_owned());
        roots.insert(canonical);
    }
}

fn package_name(path: &Path) -> Option<String> {
    let path = path.join("package.json");
    if !std::fs::metadata(&path).is_ok_and(|meta| meta.is_file() && meta.len() <= 65_536) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    value
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn python_venv_launcher(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let python = name.strip_prefix("python").is_some_and(|suffix| {
        suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b't'))
    });
    let Some(bin) = path
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "bin"))
    else {
        return false;
    };
    let Some(prefix) = bin.parent() else {
        return false;
    };
    let config = prefix.join("pyvenv.cfg");
    python
        && std::fs::symlink_metadata(config).is_ok_and(|meta| {
            meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 16_384
        })
        && executable_file(&bin.join("python"))
}

fn add_runtime_target(roots: &mut BTreeSet<PathBuf>, path: &Path) {
    if let Some(canonical) = existing(path) {
        roots.insert(canonical);
    }
}

/// Recognize installed language runtimes structurally, not by granting every
/// PATH parent. The prefix itself (and .npmrc, credentials, etc.) stays hidden.
fn language_runtime_paths(executable: &Path) -> BTreeSet<PathBuf> {
    let mut roots = BTreeSet::new();
    let canonical = executable.canonicalize().ok();
    for candidate in std::iter::once(executable).chain(canonical.as_deref()) {
        let mut node_prefixes = BTreeSet::new();
        if candidate
            .parent()
            .is_some_and(|parent| parent.file_name().is_some_and(|name| name == "bin"))
        {
            if let Some(prefix) = candidate.parent().and_then(Path::parent) {
                node_prefixes.insert(prefix.to_owned());
            }
        }
        // Canonical npx/npm/pnpm entrypoints are often inside lib/node_modules.
        for modules in candidate
            .ancestors()
            .filter(|path| path.file_name().is_some_and(|name| name == "node_modules"))
        {
            if let Some(lib) = modules
                .parent()
                .filter(|path| path.file_name().is_some_and(|name| name == "lib"))
            {
                if let Some(prefix) = lib.parent() {
                    node_prefixes.insert(prefix.to_owned());
                }
            }
        }
        for prefix in node_prefixes {
            let modules = prefix.join("lib/node_modules");
            let npm = modules.join("npm");
            let corepack = modules.join("corepack");
            if !executable_file(&prefix.join("bin/node"))
                || !(package_name(&npm).as_deref() == Some("npm")
                    || package_name(&corepack).as_deref() == Some("corepack"))
            {
                continue;
            }
            add_runtime_path(&mut roots, &prefix.join("bin"));
            add_runtime_target(&mut roots, &prefix.join("bin/node"));
            for (path, name) in [(&npm, "npm"), (&corepack, "corepack")] {
                if package_name(path).as_deref() == Some(name) {
                    add_runtime_path(&mut roots, path);
                }
            }
            if let Ok(relative) = candidate.strip_prefix(&modules) {
                let mut components = relative.components();
                if let Some(first) = components.next() {
                    let mut package = modules.join(first.as_os_str());
                    if first.as_os_str().to_string_lossy().starts_with('@') {
                        if let Some(second) = components.next() {
                            package.push(second.as_os_str());
                        }
                    }
                    if package_name(&package).is_some() {
                        add_runtime_path(&mut roots, &package);
                    }
                }
            }
        }
        let Some(bin) = candidate
            .parent()
            .filter(|path| path.file_name().is_some_and(|name| name == "bin"))
        else {
            continue;
        };
        let Some(prefix) = bin.parent() else {
            continue;
        };
        let config = prefix.join("pyvenv.cfg");
        let venv = std::fs::metadata(&config)
            .is_ok_and(|meta| meta.is_file() && meta.len() <= 16_384)
            && executable_file(&bin.join("python"));
        let python = candidate
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("python"));
        if !venv && !python {
            continue;
        }
        let libraries = python_library_paths(prefix, venv);
        if venv || !libraries.is_empty() {
            add_runtime_path(&mut roots, bin);
            for library in libraries {
                add_runtime_path(&mut roots, &library);
            }
        }
        if venv {
            add_runtime_path(&mut roots, &config);
            add_runtime_target(&mut roots, &bin.join("python"));
            if let Ok(contents) = std::fs::read_to_string(&config) {
                if let Some(home) = contents
                    .lines()
                    .filter_map(|line| line.split_once('='))
                    .find_map(|(key, value)| {
                        (key.trim() == "home").then(|| PathBuf::from(value.trim()))
                    })
                {
                    if home.is_absolute() && executable_file(&home.join("python3")) {
                        add_runtime_target(&mut roots, &home.join("python3"));
                        if let Some(base) = home.parent() {
                            for library in python_library_paths(base, false) {
                                add_runtime_path(&mut roots, &library);
                            }
                        }
                    }
                }
            }
        }
    }
    roots
}

fn python_library_paths(prefix: &Path, venv: bool) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for lib in [prefix.join("lib"), prefix.join("lib64")] {
        let Ok(entries) = std::fs::read_dir(&lib) else {
            continue;
        };
        for entry in entries.take(64).flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("libpython") && path.is_file() {
                result.push(path);
                continue;
            }
            let version = name.strip_prefix("python").unwrap_or("");
            if version.is_empty()
                || !version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b't'))
                || !path.is_dir()
            {
                continue;
            }
            if venv {
                let packages = path.join("site-packages");
                if packages.is_dir() {
                    result.push(packages);
                }
            } else if path.join("encodings/__init__.py").is_file() && path.join("os.py").is_file() {
                result.push(path);
            }
        }
    }
    result
}

fn scope(
    context: &ToolContext,
    executable: &Path,
    extra_writable: &[PathBuf],
) -> Result<Scope, String> {
    fingerprint(context)?;
    let workspace = context
        .workspace
        .canonicalize()
        .map_err(|_| "Sprite workspace is unavailable")?;
    let data = context
        .data_directory
        .canonicalize()
        .map_err(|_| "Sprite storage is unavailable")?;
    let mut access = BTreeMap::from([(workspace.clone(), true)]);
    for mount in &context.mounts {
        access
            .entry(mount.target.clone())
            .and_modify(|write| *write &= mount.writable)
            .or_insert(mount.writable);
    }
    // These are private tool state, not the profile, conversation database, or
    // other Sprites. Explicit shared aliases take precedence over private defaults.
    for name in ["cache", "config", "data", "state", "tmp"] {
        if let Some(path) = existing(&data.join(name)) {
            access.entry(path).or_insert(true);
        }
    }
    let mut reads = runtime_paths(executable);
    if let Some(path) = existing(&data.join("shell")) {
        reads.insert(path);
    }
    for path in extra_writable {
        let path = path
            .canonicalize()
            .map_err(|_| "Private process storage is unavailable")?;
        if !(path.starts_with(&workspace) || (path.starts_with(&data) && path != data)) {
            return Err(
                "Process storage must stay inside this Sprite's private directories".into(),
            );
        }
        access.entry(path).or_insert(true);
    }
    // Any read-only ancestor wins over writable aliases to the same data.
    let readonly = access
        .iter()
        .filter_map(|(path, writable)| (!writable).then_some(path.clone()))
        .collect::<Vec<_>>();
    let writes = access
        .iter()
        .filter_map(|(path, writable)| {
            (*writable && !readonly.iter().any(|root| path.starts_with(root)))
                .then_some(path.clone())
        })
        .collect::<Vec<_>>();
    reads.extend(access.keys().cloned());
    let denied = context
        .denied_roots
        .iter()
        .map(|path| {
            if !path.is_absolute() || path.parent().is_none() {
                return Err("Invalid excluded project boundary".to_owned());
            }
            Ok(path.canonicalize().unwrap_or_else(|_| path.clone()))
        })
        .collect::<Result<BTreeSet<_>, String>>()?
        .into_iter()
        .collect();
    Ok(Scope {
        working_directory: workspace,
        reads: reads.into_iter().collect(),
        writes,
        readonly,
        denied,
    })
}

fn runtime_paths(executable: &Path) -> BTreeSet<PathBuf> {
    let mut roots = BTreeSet::new();
    // Installed code and public OS data only. No blanket /etc, /var, /home,
    // /Users, /Applications, or root-filesystem grant.
    for value in [
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib",
        "/usr/lib64",
        "/usr/libexec",
        "/usr/share",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/usr/local/lib",
        "/usr/local/libexec",
        "/usr/local/share",
        "/usr/local/Cellar",
        "/usr/local/opt",
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/opt/homebrew/lib",
        "/opt/homebrew/share",
        "/opt/homebrew/Cellar",
        "/opt/homebrew/opt",
        "/opt/local/bin",
        "/opt/local/sbin",
        "/opt/local/lib",
        "/opt/local/libexec",
        "/opt/local/share",
        "/nix/store",
        "/home/linuxbrew/.linuxbrew/bin",
        "/home/linuxbrew/.linuxbrew/lib",
        "/home/linuxbrew/.linuxbrew/share",
        "/home/linuxbrew/.linuxbrew/Cellar",
        "/home/linuxbrew/.linuxbrew/opt",
        "/etc/ld.so.cache",
        "/etc/ld.so.conf",
        "/etc/ld.so.conf.d",
        "/etc/resolv.conf",
        "/etc/hosts",
        "/etc/nsswitch.conf",
        "/etc/passwd",
        "/etc/group",
        "/etc/localtime",
        "/etc/ssl/certs",
        "/etc/pki",
        "/etc/ca-certificates",
        "/etc/protocols",
        "/etc/services",
        "/etc/inputrc",
        "/etc/shells",
        "/etc/profile",
        "/etc/profile.d",
        "/etc/bash.bashrc",
        "/etc/zsh",
        "/etc/zshrc",
        "/etc/zprofile",
        "/etc/zshenv",
        "/etc/zlogin",
        "/System/Library",
        "/System/Cryptexes/App",
        "/System/Cryptexes/OS",
        "/System/Volumes/Preboot/Cryptexes",
        "/Library/Apple",
        "/Library/Frameworks",
        "/private/var/db/dyld",
        "/private/var/db/timezone",
    ] {
        let path = Path::new(value);
        if let Some(canonical) = existing(path) {
            roots.insert(canonical);
            roots.insert(path.to_owned());
        }
    }
    add_runtime_target(&mut roots, executable);
    roots.extend(language_runtime_paths(executable));
    // Chrome ships code, locales and resources together; grant only its chosen
    // installation bundle, never every application or a user profile.
    if let Some(bundle) = executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
    {
        roots.insert(bundle.to_owned());
    } else if executable.file_name().is_some_and(|name| {
        ["chrome", "chromium", "chromium-browser"]
            .iter()
            .any(|value| name == *value)
    }) {
        if let Some(parent) = executable
            .parent()
            .filter(|path| !matches!(path.to_str(), Some("/bin" | "/usr/bin" | "/usr/local/bin")))
        {
            roots.insert(parent.to_owned());
        }
    }
    roots
}

#[cfg(target_os = "linux")]
fn scoped_command(
    executable: &Path,
    arguments: &[OsString],
    scope: &Scope,
    browser: bool,
) -> Result<tokio::process::Command, String> {
    let wrapper = ["/usr/bin/bwrap", "/bin/bwrap"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .ok_or(
            "Workspace isolation requires Bubblewrap (bwrap). No unsandboxed command was started.",
        )?;
    let mut command = tokio::process::Command::new(wrapper);
    command.args([
        "--unshare-all",
        "--unshare-user",
        "--share-net",
        "--cap-drop",
        "ALL",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/run",
        "--tmpfs",
        "/dev/shm",
        "--new-session",
        "--die-with-parent",
    ]);
    // Chromium retains its own sandbox. Nested user namespaces inherit locked
    // mounts and cannot uncover our denied trees or the host root. See
    // https://man7.org/linux/man-pages/man7/mount_namespaces.7.html
    if !browser {
        command.arg("--disable-userns");
    }
    let mut roots = scope.reads.iter().cloned().collect::<BTreeSet<_>>();
    roots.extend(scope.writes.iter().cloned());
    // Parent mounts must precede narrower mounts. Read-only exceptions and
    // denied project masks are then applied last and cannot be overridden.
    let mut roots = roots.into_iter().collect::<Vec<_>>();
    roots.sort_by_key(|path| path.components().count());
    for path in roots {
        let writable = scope.writes.iter().any(|root| path.starts_with(root))
            && !scope.readonly.iter().any(|root| path.starts_with(root));
        command
            .arg(if writable { "--bind" } else { "--ro-bind" })
            .arg(&path)
            .arg(&path);
    }
    for path in &scope.readonly {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    for path in &scope.denied {
        if path.is_dir() {
            command
                .arg("--tmpfs")
                .arg(path)
                .arg("--remount-ro")
                .arg(path);
        } else if path.exists() {
            command.args(["--ro-bind", "/dev/null"]).arg(path);
        }
    }
    command
        .arg("--chdir")
        .arg(&scope.working_directory)
        .arg("--")
        .arg(executable)
        .args(arguments);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn scoped_command(
    executable: &Path,
    arguments: &[OsString],
    scope: &Scope,
    browser: bool,
) -> Result<tokio::process::Command, String> {
    let wrapper = Path::new("/usr/bin/sandbox-exec");
    if !wrapper.is_file() {
        return Err(
            "Workspace isolation is unavailable on this Mac. No command was started.".into(),
        );
    }
    let mut command = tokio::process::Command::new(wrapper);
    command
        .arg("-p")
        .arg(macos_profile_for(scope, browser)?)
        .arg(executable)
        .args(arguments);
    Ok(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn scoped_command(
    _: &Path,
    _: &[OsString],
    _: &Scope,
    _: bool,
) -> Result<tokio::process::Command, String> {
    Err("Workspace process isolation is not supported on this operating system. No command was started.".into())
}

#[cfg(test)]
fn macos_profile(scope: &Scope) -> Result<String, String> {
    macos_profile_for(scope, false)
}

#[cfg(any(target_os = "macos", test))]
fn macos_profile_for(scope: &Scope, browser: bool) -> Result<String, String> {
    // Supported operations and same-sandbox inheritance follow Chromium's
    // production Seatbelt policy and Anthropic's sandbox-runtime implementation:
    // https://chromium.googlesource.com/chromium/src/+/refs/heads/main/sandbox/policy/mac/common.sb
    // https://github.com/anthropics/sandbox-runtime/blob/main/src/sandbox/macos-sandbox-utils.ts
    let mut profile = String::from(
        "(version 1)\n(deny default)\n(allow process-fork)\n(allow process-info* (target same-sandbox))\n(allow signal (target same-sandbox))\n(allow sysctl-read (sysctl-name-prefix \"hw.\") (sysctl-name \"kern.osrelease\") (sysctl-name \"kern.ostype\") (sysctl-name \"kern.osversion\") (sysctl-name \"kern.osproductversion\") (sysctl-name \"kern.hostname\") (sysctl-name \"kern.maxfilesperproc\"))\n(allow system-socket (socket-domain AF_INET) (socket-domain AF_INET6) (socket-domain AF_UNIX))\n(allow network-outbound (remote ip \"*:*\"))\n(allow network-inbound (local ip \"*:*\"))\n(allow network-bind (local ip \"*:*\"))\n(allow mach-lookup (global-name \"com.apple.logd\") (global-name \"com.apple.system.logger\") (global-name \"com.apple.system.opendirectoryd.libinfo\") (global-name \"com.apple.system.opendirectoryd.membership\") (global-name \"com.apple.bsd.dirhelper\") (global-name \"com.apple.trustd.agent\"))\n",
    );
    let mut ancestors = BTreeSet::from([PathBuf::from("/")]);
    for path in &scope.reads {
        profile.push_str(&format!(
            "(allow file-read* file-map-executable process-exec ({} {}))\n",
            if path.is_dir() { "subpath" } else { "literal" },
            profile_path(path)?
        ));
        ancestors.extend(path.ancestors().skip(1).map(Path::to_owned));
    }
    for path in &scope.writes {
        let path = profile_path(path)?;
        profile.push_str(&format!("(allow file-read* file-write* file-map-executable process-exec (subpath {path}))\n(allow network-bind (local unix-socket (subpath {path})))\n(allow network-outbound (remote unix-socket (subpath {path})))\n"));
    }
    // CLI temporary files use private TMPDIR. /tmp itself is metadata-only.
    for path in ["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"] {
        profile.push_str(&format!(
            "(allow file-read* file-write-data file-ioctl (literal \"{path}\"))\n"
        ));
    }
    for path in ancestors {
        profile.push_str(&format!(
            "(allow file-read-metadata (literal {}))\n",
            profile_path(&path)?
        ));
    }
    let mut pinned = scope.writes.iter().cloned().collect::<BTreeSet<_>>();
    for path in &scope.readonly {
        profile.push_str(&format!(
            "(deny file-write* (subpath {}))\n",
            profile_path(path)?
        ));
        pinned.extend(
            path.ancestors()
                .take_while(|path| path.parent().is_some())
                .map(Path::to_owned),
        );
    }
    for path in &scope.denied {
        profile.push_str(&format!(
            "(deny file-read* file-write* file-map-executable process-exec (subpath {}))\n",
            profile_path(path)?
        ));
        pinned.extend(
            path.ancestors()
                .take_while(|path| path.parent().is_some())
                .map(Path::to_owned),
        );
    }
    for path in pinned {
        profile.push_str(&format!(
            "(deny file-write-unlink (literal {}))\n",
            profile_path(&path)?
        ));
    }
    // Never let a shell ask a host service to launch an unsandboxed process.
    profile.push_str("(deny appleevent-send)\n(deny lsopen)\n(deny mach-lookup (global-name \"com.apple.cfprefsd.daemon\") (global-name \"com.apple.coreservices.appleevents\") (global-name \"com.apple.coreservices.launchservicesd\") (global-name \"com.apple.lsd.open\") (global-name \"com.apple.lsd.openurl\"))\n(deny mach-lookup (global-name \"com.apple.securityd.xpc\") (global-name \"com.apple.SecurityServer\"))\n(deny process-exec (regex #\"(^|/)git-credential-osxkeychain$\") (literal \"/usr/bin/security\") (literal \"/usr/bin/open\") (literal \"/usr/bin/osascript\") (literal \"/bin/launchctl\") (literal \"/usr/bin/automator\"))\n");
    if browser {
        // Selected display/GPU services from Chromium's own macOS GPU policy:
        // https://chromium.googlesource.com/chromium/src/+/main/sandbox/policy/mac/gpu.sb
        // LaunchServices, AppleEvents, preferences and Keychain remain denied.
        profile.push_str("(allow mach-lookup (global-name \"com.apple.windowserver.active\") (global-name \"com.apple.CARenderServer\") (global-name \"com.apple.tsm.uiserver\") (global-name \"com.apple.cvmsServ\") (global-name \"com.apple.gpumemd.source\") (xpc-service-name \"com.apple.MTLCompilerService\"))\n(allow ipc-posix-shm)\n(allow iokit-open (iokit-user-client-class \"IOSurfaceRootUserClient\") (iokit-user-client-class \"IOFramebufferSharedUserClient\") (iokit-user-client-class \"IOAccelerationUserClient\"))\n");
        profile.push_str("(allow mach-lookup (global-name \"com.apple.FontObjectsServer\") (global-name \"com.apple.fonts\"))\n");
        // Chromium's browser-to-child rendezvous service authenticates clients
        // against its registered child PIDs. Limit bootstrap access to those
        // service names, not arbitrary Chrome/host application IPC.
        // https://chromium.googlesource.com/chromium/src/+/main/sandbox/policy/mac/common.sb
        let rendezvous = r##"#"^(com\.google\.Chrome|org\.chromium\.Chromium|com\.brave\.Browser)(\.(beta|dev|canary))?\.MachPortRendezvousServer\.[0-9]+$""##;
        profile.push_str(&format!("(allow mach-lookup (global-name-regex {rendezvous}))\n(allow mach-register (global-name-regex {rendezvous}))\n"));
    }
    Ok(profile)
}

#[cfg(any(target_os = "macos", test))]
fn profile_path(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or("Filesystem isolation requires UTF-8 paths on macOS")?;
    if text.chars().any(char::is_control) {
        return Err("Filesystem isolation cannot use control characters in paths".into());
    }
    serde_json::to_string(text).map_err(|_| "Cannot encode a filesystem boundary".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn node_fixture(root: &Path, node: &Path) -> PathBuf {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let prefix = root.join("versions/node/v24");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(prefix.join("lib/node_modules/npm/bin")).unwrap();
        std::fs::create_dir_all(prefix.join("lib/node_modules/npm/lib")).unwrap();
        std::fs::write(
            prefix.join("lib/node_modules/npm/package.json"),
            r#"{"name":"npm"}"#,
        )
        .unwrap();
        let cli = prefix.join("lib/node_modules/npm/bin/npx-cli.js");
        std::fs::write(
            &cli,
            "#!/usr/bin/env node\nconsole.log(require('../lib/message.js'));\n",
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            prefix.join("lib/node_modules/npm/lib/message.js"),
            "module.exports = 'scoped-node-ok';\n",
        )
        .unwrap();
        symlink(node, prefix.join("bin/node")).unwrap();
        symlink(
            "../lib/node_modules/npm/bin/npx-cli.js",
            prefix.join("bin/npx"),
        )
        .unwrap();
        prefix
    }

    #[cfg(unix)]
    #[test]
    fn recognized_node_prefix_grants_runtime_code_without_home_or_config() {
        let root = tempfile::tempdir().unwrap();
        let prefix = node_fixture(root.path(), Path::new("/bin/sh"));
        let command = prefix.join("bin/npx");
        assert_eq!(
            resolve_executable(&command).unwrap(),
            command.canonicalize().unwrap()
        );
        let paths = language_runtime_paths(&command);
        assert!(paths.contains(&prefix.join("bin")));
        assert!(paths.contains(&prefix.join("lib/node_modules/npm")));
        assert!(!paths.contains(&prefix));
        assert!(
            !paths
                .iter()
                .any(|path| prefix.join(".npmrc").starts_with(path))
        );
        assert!(
            !paths
                .iter()
                .any(|path| root.path().join("credentials").starts_with(path))
        );
        std::fs::write(
            prefix.join("lib/node_modules/npm/package.json"),
            r#"{"name":"not-npm"}"#,
        )
        .unwrap();
        assert!(language_runtime_paths(&command).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn recognized_python_venv_preserves_its_launcher_and_limits_library_roots() {
        let root = tempfile::tempdir().unwrap();
        let venv = root.path().join("venv");
        std::fs::create_dir_all(venv.join("bin")).unwrap();
        std::fs::create_dir_all(venv.join("lib/python3.13/site-packages")).unwrap();
        std::fs::write(
            venv.join("pyvenv.cfg"),
            "home = /usr/bin\ninclude-system-site-packages = false\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("/bin/sh", venv.join("bin/python")).unwrap();
        assert_eq!(
            resolve_executable(&venv.join("bin/python")).unwrap(),
            venv.join("bin/python")
        );
        let paths = language_runtime_paths(&venv.join("bin/python"));
        assert!(paths.contains(&venv.join("pyvenv.cfg")));
        assert!(paths.contains(&venv.join("bin")));
        assert!(paths.contains(&venv.join("lib/python3.13/site-packages")));
        assert!(!paths.contains(&venv));
        assert!(!paths.iter().any(|path| venv.join(".env").starts_with(path)));
        assert!(
            !paths
                .iter()
                .any(|path| root.path().join("other-venv/lib").starts_with(path))
        );
    }

    #[cfg(unix)]
    #[test]
    fn generic_shell_aliases_resolve_to_their_installed_executable() {
        let root = tempfile::tempdir().unwrap();
        let alias = root.path().join("zsh");
        std::os::unix::fs::symlink("/bin/sh", &alias).unwrap();
        assert_eq!(
            resolve_executable(&alias).unwrap(),
            Path::new("/bin/sh").canonicalize().unwrap()
        );
        assert!(!runtime_paths(&resolve_executable(&alias).unwrap()).contains(&alias));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn scoped_generic_shell_alias_starts_without_a_dangling_mount() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let alias = root.path().join("zsh");
        std::os::unix::fs::symlink("/bin/sh", &alias).unwrap();
        let context = ToolContext {
            orbit_id: "shell-alias-test".into(),
            data_directory: workspace.clone(),
            workspace,
            allowed_roots: vec![],
            mounts: vec![],
            denied_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let output = command(
            &alias,
            &["-c".into(), "printf alias-ok".into()],
            &context,
            &[],
        )
        .unwrap()
        .output()
        .await
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"alias-ok");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn scoped_npx_launcher_can_find_node_and_its_own_package_code() {
        let node = [Path::new("/usr/bin/node"), Path::new("/usr/local/bin/node")]
            .into_iter()
            .find(|path| path.is_file());
        let Some(node) = node else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let prefix = node_fixture(root.path(), node);
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let context = ToolContext {
            orbit_id: "runtime-test".into(),
            data_directory: workspace.clone(),
            workspace: workspace.clone(),
            allowed_roots: vec![],
            mounts: vec![],
            denied_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            command(&prefix.join("bin/npx"), &[], &context, &[])
                .unwrap()
                .env_clear()
                .env("HOME", &workspace)
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", prefix.join("bin").display()),
                )
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "scoped-node-ok"
        );
    }

    #[test]
    fn literal_process_paths_require_an_approved_tree() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let context = ToolContext {
            orbit_id: "paths".into(),
            data_directory: workspace.clone(),
            workspace: workspace.clone(),
            allowed_roots: vec![],
            mounts: vec![],
            denied_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        assert!(
            process_argument_path_allowed(
                Path::new("/bin/sh"),
                &workspace.join("new/file"),
                &context,
                &[]
            )
            .unwrap()
        );
        assert!(
            !process_argument_path_allowed(
                Path::new("/bin/sh"),
                &root.path().join("private"),
                &context,
                &[]
            )
            .unwrap()
        );
        assert!(
            !process_argument_path_allowed(
                Path::new("/bin/sh"),
                &workspace.join("new/../../private"),
                &context,
                &[]
            )
            .unwrap()
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn browser_display_grants_only_exact_auth_and_wayland_socket_paths() {
        use std::os::unix::net::UnixListener;
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("wayland-test");
        let _listener = UnixListener::bind(&socket).unwrap();
        let auth = root.path().join("Xauthority");
        std::fs::write(&auth, "display cookie").unwrap();
        let display = linux_display_runtime_with(
            Some("localhost:99".into()),
            Some(auth.clone()),
            None,
            Some(root.path().into()),
            Some("wayland-test".into()),
        );
        assert!(display.paths.contains(&auth));
        assert!(display.paths.contains(&socket));
        assert!(!display.paths.contains(&root.path().to_owned()));
        assert!(
            display
                .environment
                .iter()
                .any(|(name, value)| *name == "XAUTHORITY" && value == auth.as_os_str())
        );
        let escaped = linux_display_runtime_with(
            None,
            None,
            None,
            Some(root.path().into()),
            Some("../wayland-test".into()),
        );
        assert!(escaped.paths.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn browser_nested_namespace_cannot_uncover_denied_host_files() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let shared = root.path().join("shared");
        let denied = shared.join("private-project");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&denied).unwrap();
        std::fs::write(denied.join("secret"), "private data").unwrap();
        std::fs::write(shared.join("keep.txt"), "read-only data").unwrap();
        let link = workspace.join("reference");
        std::os::unix::fs::symlink(&shared, &link).unwrap();
        let context = ToolContext {
            orbit_id: "browser-boundary".into(),
            data_directory: workspace.clone(),
            workspace: workspace.clone(),
            allowed_roots: vec![],
            mounts: vec![crate::WorkspaceMount {
                path: link,
                target: shared.clone(),
                writable: false,
            }],
            denied_roots: vec![denied.clone()],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let script = r#"test ! -r "$1/secret" || exit 1
if printf overwritten > "$3/keep.txt" 2>/dev/null; then exit 4; fi
unshare -Ur -m /bin/sh -c 'umount "$1" 2>/dev/null; if test -r "$1/secret"; then printf leaked > "$2"; fi; mount -o remount,rw "$3" 2>/dev/null; if printf overwritten > "$3/keep.txt" 2>/dev/null; then printf leaked > "$2"; fi' sh "$1" "$2" "$3" 2>/dev/null || true
test ! -e "$2" || exit 2
test ! -r /proc/1/root/etc/shadow || exit 3"#;
        let args = vec![
            "-c".into(),
            script.into(),
            "sh".into(),
            denied.into_os_string(),
            workspace.join("leak").into_os_string(),
            workspace.join("reference").into_os_string(),
        ];
        let output = browser_command(Path::new("/bin/sh"), &args, &context, &[])
            .unwrap()
            .output()
            .await
            .unwrap();
        if !output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains("No permissions to create")
        {
            return;
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!workspace.join("leak").exists());
        assert_eq!(
            std::fs::read_to_string(shared.join("keep.txt")).unwrap(),
            "read-only data"
        );
    }

    #[test]
    fn macos_is_default_deny_with_scoped_writes_and_no_automation_escape() {
        let scope = Scope {
            working_directory: PathBuf::from("/Users/me/sprite"),
            reads: vec![PathBuf::from("/usr/bin"), PathBuf::from("/Users/me/sprite")],
            writes: vec![PathBuf::from("/Users/me/sprite")],
            readonly: vec![PathBuf::from("/Users/me/sprite/reference")],
            denied: vec![PathBuf::from("/Users/me/sprite/private\"project")],
        };
        let profile = macos_profile(&scope).unwrap();
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains(
            "(allow file-read* file-map-executable process-exec (subpath \"/usr/bin\"))"
        ));
        assert!(!profile.contains("(allow default)"));
        assert!(profile.contains("(deny appleevent-send)"));
        assert!(profile.contains("(deny lsopen)"));
        assert!(!profile.contains("(allow mach-lookup)"));
        assert!(profile.contains("(deny file-write* (subpath \"/Users/me/sprite/reference\"))"));
        assert!(profile.contains("(deny file-write-unlink (literal \"/Users/me/sprite\"))"));
        assert!(profile.contains("private\\\"project"));
        assert!(profile_path(Path::new("/tmp/new\nline")).is_err());
        assert!(!profile.contains("com.apple.windowserver.active"));
        let browser = macos_profile_for(&scope, true).unwrap();
        assert!(browser.contains("com.apple.windowserver.active"));
        assert!(browser.contains("(deny appleevent-send)"));
        assert!(browser.contains("(deny lsopen)"));
        assert!(browser.contains("(deny mach-lookup (global-name \"com.apple.cfprefsd.daemon\")"));
        assert!(!browser.contains("(allow mach-lookup)"));
    }
}
