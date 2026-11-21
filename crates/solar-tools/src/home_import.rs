//! Explicit, reviewable live links from the user's home into a Sprite workspace.
//! Search reads names only. Planning never runs profiles or exposes their contents.
use crate::{CancellationToken, ToolDefinition, WorkspaceMount, definition as tool_definition};
use cap_std::fs::{Dir, OpenOptions};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_LINKS: usize = 256;
const MAX_PROFILE: usize = 256 * 1024;
const MAX_SEARCH_VISITS: usize = 4096;

pub(crate) fn definition() -> ToolDefinition {
    tool_definition(
        "pull_from_user_home",
        "Request specific files or folders from the user's home as live symlinks inside this Sprite's workspace. ALWAYS requires a separate user approval, including for trusted Sprites. Use search_user_home to discover names first. The review lists every link and access level. Sources stay on the user's computer; read-only is the default. Existing workspace files are never overwritten. Static shell source dependencies can be included without running profile code; absolute host paths in profiles are not rewritten.",
        json!({"paths":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":20},"includeDependencies":{"type":"boolean","default":true},"writable":{"type":"boolean","default":false}}),
        &["paths"],
    )
}

pub(crate) fn search_definition() -> ToolDefinition {
    tool_definition(
        "search_user_home",
        "Find files or folders by name or relative path in the user's home before requesting a workspace share. Reads metadata only, never file contents, and does not recurse through symlinks. Solar storage and inaccessible projects are excluded. A result is not a grant: use pull_from_user_home to request a live workspace link and wait for the user's required approval. Supports * and ? wildcards.",
        json!({"query":{"type":"string","minLength":1,"maxLength":200},"path":{"type":"string","description":"Optional home-relative folder, for example .config or Documents"},"limit":{"type":"integer","minimum":1,"maximum":100,"default":50}}),
        &["query"],
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    directory: bool,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created: Option<std::time::SystemTime>,
    #[cfg(not(unix))]
    modified: Option<std::time::SystemTime>,
    #[cfg(not(unix))]
    length: u64,
}

fn identity(metadata: &std::fs::Metadata) -> Identity {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Identity {
        directory: metadata.is_dir(),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        #[cfg(not(unix))]
        created: metadata.created().ok(),
        #[cfg(not(unix))]
        modified: if metadata.is_dir() {
            None
        } else {
            metadata.modified().ok()
        },
        #[cfg(not(unix))]
        length: if metadata.is_dir() { 0 } else { metadata.len() },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Destination {
    link: Option<(PathBuf, Identity)>,
    parents: Vec<(PathBuf, Identity)>,
}

struct PlannedLink {
    requested: PathBuf,
    target: PathBuf,
    source_identity: Identity,
    destination: PathBuf,
    previous: Destination,
    dependency: bool,
}

pub struct HomeImportPlan {
    home: PathBuf,
    workspace: PathBuf,
    storage: PathBuf,
    links: Vec<PlannedLink>,
    warnings: BTreeSet<String>,
    denied: Vec<PathBuf>,
    include_dependencies: bool,
    writable: bool,
}

fn normalized(path: &Path) -> Result<PathBuf, String> {
    let mut output = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !output.pop() {
                    return Err("Invalid home path".into());
                }
            }
            Component::CurDir => {}
            other => output.push(other.as_os_str()),
        }
    }
    Ok(output)
}

fn expand_home(home: &Path, input: &str, base: &Path) -> Result<PathBuf, String> {
    let input = input.trim();
    if input.is_empty() || input.chars().any(char::is_control) {
        return Err("Choose a valid home path".into());
    }
    let expanded = if matches!(input, "~" | "$HOME" | "${HOME}") {
        home.to_owned()
    } else if let Some(tail) = input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("$HOME/"))
        .or_else(|| input.strip_prefix("${HOME}/"))
    {
        home.join(tail)
    } else if input.contains(['$', '`']) {
        return Err("Dynamic paths must be requested explicitly after resolving them".into());
    } else if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        base.join(input)
    };
    let path = normalized(&expanded)?;
    if !path.starts_with(home) {
        return Err("Home requests cannot access outside the user's home".into());
    }
    if path
        .strip_prefix(home)
        .unwrap_or(&path)
        .components()
        .any(|part| part.as_os_str() == ".solar")
    {
        return Err("Solar's private storage cannot be shared into a Sprite".into());
    }
    Ok(path)
}

fn denied_path(path: &Path, directory: bool, denied: &[PathBuf]) -> bool {
    denied
        .iter()
        .any(|root| path.starts_with(root) || (directory && root.starts_with(path)))
}

impl HomeImportPlan {
    pub fn prepare(
        home: &Path,
        workspace: &Path,
        storage: &Path,
        denied: &[PathBuf],
        args: &Value,
    ) -> Result<Self, String> {
        let requested = args
            .get("paths")
            .and_then(Value::as_array)
            .filter(|paths| !paths.is_empty() && paths.len() <= 20)
            .ok_or("Choose between 1 and 20 home files or folders")?;
        let mut plan = Self {
            home: home
                .canonicalize()
                .map_err(|_| "The user's home is unavailable")?,
            workspace: workspace
                .canonicalize()
                .map_err(|_| "The Sprite workspace is unavailable")?,
            storage: storage
                .canonicalize()
                .map_err(|_| "The Sprite storage is unavailable")?,
            links: vec![],
            warnings: BTreeSet::new(),
            denied: denied
                .iter()
                .map(|root| root.canonicalize().unwrap_or_else(|_| root.clone()))
                .collect(),
            include_dependencies: args
                .get("includeDependencies")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            writable: args
                .get("writable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        if plan.workspace == plan.storage || !plan.workspace.starts_with(&plan.storage) {
            return Err("Sprite workspace must be inside its private storage".into());
        }
        let mut paths = requested
            .iter()
            .map(|path| {
                plan.expand(
                    path.as_str().ok_or("Every home path must be text")?,
                    &plan.home,
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        // A folder grant already includes descendants. Plan parents first so
        // the review does not create redundant links through planned symlinks.
        paths.sort_by_key(|path| path.components().count());
        for source in paths {
            plan.collect(&source, false, &mut BTreeSet::new(), 0)?;
        }
        if plan.links.is_empty() {
            return Err("No files or folders were found to share".into());
        }
        plan.links.sort_by(|a, b| a.destination.cmp(&b.destination));
        Ok(plan)
    }

    pub fn links(&self) -> Vec<WorkspaceMount> {
        self.links
            .iter()
            .map(|link| WorkspaceMount {
                path: link.destination.clone(),
                target: link.target.clone(),
                writable: self.writable,
            })
            .collect()
    }

    pub fn validate_scope(&self, denied: &[PathBuf]) -> Result<(), String> {
        let denied: Vec<_> = denied
            .iter()
            .map(|root| root.canonicalize().unwrap_or_else(|_| root.clone()))
            .collect();
        if self
            .links
            .iter()
            .any(|link| denied_path(&link.target, link.source_identity.directory, &denied))
        {
            return Err(
                "Project access changed while approval was open. Request the share again.".into(),
            );
        }
        Ok(())
    }

    pub fn review(&self) -> Value {
        json!({"links":self.links.iter().map(|link|json!({
            "source":self.display(&link.requested),"destination":link.destination,
            "directory":link.source_identity.directory,"writable":self.writable,
            "dependency":link.dependency,"existing":link.previous.link.is_some()
        })).collect::<Vec<_>>(),"warnings":self.warnings,
        "notice":"These are live links to the selected originals, not copies. Read-only unless you approve write access. Profiles may contain credentials and executable startup code. Every link requires your approval; file contents are not included in this review."})
    }

    fn expand(&self, input: &str, base: &Path) -> Result<PathBuf, String> {
        let path = expand_home(&self.home, input, base)?;
        if path.starts_with(&self.storage) {
            return Err("Solar's private storage cannot be shared into a Sprite".into());
        }
        Ok(path)
    }

    fn display(&self, path: &Path) -> String {
        format!(
            "~/{}",
            path.strip_prefix(&self.home)
                .unwrap_or(path)
                .to_string_lossy()
        )
    }

    fn choose_destination(
        &self,
        source: &Path,
        target: &Path,
    ) -> Result<(PathBuf, Destination), String> {
        let relative = source
            .strip_prefix(&self.home)
            .map_err(|_| "Invalid home source")?;
        let direct = self.workspace.join(relative);
        for index in 0..100 {
            let candidate = if index == 0 {
                direct.clone()
            } else {
                let prefix = if index == 1 {
                    "shared/home".to_owned()
                } else {
                    format!("shared/home-{index}")
                };
                self.workspace.join(prefix).join(relative)
            };
            if self.links.iter().any(|link| {
                candidate.starts_with(&link.destination) || link.destination.starts_with(&candidate)
            }) {
                continue;
            }
            if let Ok(snapshot) = destination(&self.workspace, &candidate, target) {
                return Ok((candidate, snapshot));
            }
        }
        Err("No unused workspace path is available for this share".into())
    }

    fn collect(
        &mut self,
        source: &Path,
        dependency: bool,
        visiting: &mut BTreeSet<PathBuf>,
        depth: usize,
    ) -> Result<(), String> {
        if self.links.len() >= MAX_LINKS || depth > 24 {
            return Err("Too many linked profile dependencies; request a smaller set".into());
        }
        let target = source
            .canonicalize()
            .map_err(|_| format!("Cannot find {}", self.display(source)))?;
        self.expand(
            target.to_str().ok_or("Home paths must be valid text")?,
            &self.home,
        )?;
        let metadata = std::fs::metadata(&target).map_err(|_| "Cannot inspect a home source")?;
        if !metadata.is_dir() && !metadata.is_file() {
            return Err("Only regular files and folders can be shared".into());
        }
        if target == self.home || (metadata.is_dir() && self.storage.starts_with(&target)) {
            return Err(
                "Select specific files or folders, not the whole home or Solar storage".into(),
            );
        }
        if denied_path(&target, metadata.is_dir(), &self.denied) {
            return Err("A requested source overlaps a project this Sprite cannot access".into());
        }
        if self.links.iter().any(|link| {
            link.requested == source
                || (link.source_identity.directory && target.starts_with(&link.target))
        }) {
            return Ok(());
        }
        if !visiting.insert(target.clone()) {
            self.warnings.insert(format!(
                "Skipped a circular profile dependency at {}",
                self.display(source)
            ));
            return Ok(());
        }
        let (destination, previous) = self.choose_destination(source, &target)?;
        if destination
            != self.workspace.join(
                source
                    .strip_prefix(&self.home)
                    .map_err(|_| "Invalid home source")?,
            )
        {
            self.warnings.insert(format!(
                "{} uses {} because its usual workspace path is occupied",
                self.display(source),
                destination.display()
            ));
        }
        self.links.push(PlannedLink {
            requested: source.to_owned(),
            target: target.clone(),
            source_identity: identity(&metadata),
            destination,
            previous,
            dependency,
        });
        if self.include_dependencies && metadata.is_file() && profile_file(source) {
            self.warnings.insert("Linked profile contents are used as-is. Absolute host paths are not rewritten and may be blocked by workspace isolation.".into());
            if metadata.len() > MAX_PROFILE as u64 {
                self.warnings.insert(format!(
                    "{} is too large for automatic dependency discovery",
                    self.display(source)
                ));
            } else if let Some(text) = read_profile(&self.home, &target)? {
                self.dependencies(&text, source, visiting, depth)?;
            }
        }
        visiting.remove(&target);
        Ok(())
    }

    fn dependencies(
        &mut self,
        text: &str,
        source: &Path,
        visiting: &mut BTreeSet<PathBuf>,
        depth: usize,
    ) -> Result<(), String> {
        let mut variables = BTreeMap::from([
            ("HOME".to_owned(), self.home.to_string_lossy().into_owned()),
            (
                "ZDOTDIR".to_owned(),
                self.home.to_string_lossy().into_owned(),
            ),
        ]);
        for line in text.lines() {
            let words = shell_words(line);
            for word in &words {
                if let Some((name, value)) = word.split_once('=')
                    && !name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && let Some(value) = expand_variables(value, &variables)
                {
                    variables.insert(name.to_owned(), value);
                }
            }
            for pair in words.windows(2) {
                if pair[0] != "source" && pair[0] != "." {
                    continue;
                }
                let Some(raw) = expand_variables(&pair[1], &variables) else {
                    self.warnings.insert(format!(
                        "{} has a dynamic include; request its resolved path separately",
                        self.display(source)
                    ));
                    continue;
                };
                if raw.contains(['*', '?', '`', '(', ')']) {
                    self.warnings.insert(format!(
                        "{} has a dynamic include; request its resolved path separately",
                        self.display(source)
                    ));
                    continue;
                }
                let path = match self.expand(&raw, source.parent().unwrap_or(&self.home)) {
                    Ok(path) => path,
                    Err(_) => {
                        self.warnings.insert(format!(
                            "{} references a source outside the allowed home; it was not linked",
                            self.display(source)
                        ));
                        continue;
                    }
                };
                if !path.exists() {
                    self.warnings.insert(format!(
                        "An included path in {} was not found; check the profile and request it separately",
                        self.display(source)
                    ));
                    continue;
                }
                self.collect(&path, true, visiting, depth + 1)?;
            }
        }
        Ok(())
    }

    pub fn apply(self, cancellation: &CancellationToken) -> Result<Value, String> {
        Ok(self.apply_with_receipt(cancellation)?.commit())
    }

    pub fn apply_with_receipt(
        self,
        cancellation: &CancellationToken,
    ) -> Result<HomeImportReceipt, String> {
        self.validate_scope(&self.denied)?;
        let mounts = self.links();
        let root = Dir::open_ambient_dir(&self.workspace, cap_std::ambient_authority())
            .map_err(|_| "Sprite workspace is unavailable")?;
        // Validate every source and destination before creating any links.
        for link in &self.links {
            self.validate_link(link, cancellation, &BTreeMap::new())?;
        }
        let mut created_links = Vec::new();
        let result: Result<Value, String> = (|| {
            let mut linked = Vec::new();
            let mut created_parents = BTreeMap::new();
            for link in &self.links {
                self.validate_link(link, cancellation, &created_parents)?;
                if link.previous.link.is_none() {
                    let relative = link
                        .destination
                        .strip_prefix(&self.workspace)
                        .map_err(|_| "Invalid workspace destination")?;
                    for parent in ensure_parents(
                        &root,
                        relative.parent().ok_or("Invalid workspace destination")?,
                    )? {
                        let path = self.workspace.join(parent);
                        created_parents.insert(
                            path.clone(),
                            identity(
                                &std::fs::symlink_metadata(path)
                                    .map_err(|_| "Cannot inspect created workspace folder")?,
                            ),
                        );
                    }
                    // Creation never unlinks or overwrites a destination.
                    create_link(
                        &root,
                        &link.target,
                        relative,
                        link.source_identity.directory,
                    )?;
                    let metadata = root
                        .symlink_metadata(relative)
                        .map_err(|_| "Cannot verify the new workspace link")?;
                    if !metadata.file_type().is_symlink() {
                        return Err(
                            "The workspace link was changed while it was being created".into()
                        );
                    }
                    created_links.push(CreatedLink {
                        path: relative.to_owned(),
                        target: link.target.clone(),
                        identity: cap_identity(&metadata),
                    });
                }
                linked.push(json!({"source":self.display(&link.requested),"destination":link.destination,"directory":link.source_identity.directory,"writable":self.writable}));
            }
            if cancellation.is_cancelled() {
                return Err("Share request cancelled".into());
            }
            Ok(
                json!({"linked":linked,"warnings":self.warnings,"message":"Approved live links are available inside this Sprite's workspace. Original files stay in place; changes are visible through the links."}),
            )
        })();
        match result {
            Ok(result) => Ok(HomeImportReceipt {
                root,
                created: Some(created_links),
                mounts,
                result,
            }),
            Err(error) => {
                if rollback_links(&root, &created_links) {
                    Err(error)
                } else {
                    Err(format!(
                        "{error}. Changed workspace entries were left untouched during rollback."
                    ))
                }
            }
        }
    }

    fn validate_link(
        &self,
        link: &PlannedLink,
        cancellation: &CancellationToken,
        created: &BTreeMap<PathBuf, Identity>,
    ) -> Result<(), String> {
        if cancellation.is_cancelled() {
            return Err("Share request cancelled".into());
        }
        let target = link
            .requested
            .canonicalize()
            .map_err(|_| "A shared source changed while approval was open; request it again")?;
        if target != link.target
            || identity(&std::fs::metadata(&target).map_err(|_| "A shared source is unavailable")?)
                != link.source_identity
        {
            return Err("A shared source was replaced or retargeted while approval was open; request it again".into());
        }
        let current = destination(&self.workspace, &link.destination, &link.target)?;
        if current.link != link.previous.link
            || !link
                .previous
                .parents
                .iter()
                .all(|entry| current.parents.contains(entry))
            || current.parents.iter().any(|entry| {
                !link.previous.parents.contains(entry) && created.get(&entry.0) != Some(&entry.1)
            })
        {
            return Err(
                "A workspace destination changed while approval was open; request it again".into(),
            );
        }
        Ok(())
    }
}

/// Holds new links until Solar has persisted the corresponding access grants.
/// Dropping or explicitly rolling back never removes pre-existing links.
pub struct HomeImportReceipt {
    root: Dir,
    created: Option<Vec<CreatedLink>>,
    mounts: Vec<WorkspaceMount>,
    result: Value,
}

impl HomeImportReceipt {
    pub fn mounts(&self) -> &[WorkspaceMount] {
        &self.mounts
    }

    pub fn result(&self) -> &Value {
        &self.result
    }

    pub fn commit(mut self) -> Value {
        let _ = self.created.take();
        std::mem::take(&mut self.result)
    }

    pub fn rollback(mut self) -> Result<(), String> {
        if self
            .created
            .take()
            .is_none_or(|links| rollback_links(&self.root, &links))
        {
            Ok(())
        } else {
            Err("Changed workspace entries were left untouched during share rollback".into())
        }
    }
}

impl Drop for HomeImportReceipt {
    fn drop(&mut self) {
        if let Some(links) = self.created.take() {
            let _ = rollback_links(&self.root, &links);
        }
    }
}

struct CreatedLink {
    path: PathBuf,
    target: PathBuf,
    identity: Identity,
}

fn cap_identity(metadata: &cap_std::fs::Metadata) -> Identity {
    #[cfg(unix)]
    use cap_std::fs::MetadataExt;
    Identity {
        directory: metadata.is_dir(),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        #[cfg(not(unix))]
        created: metadata.created().ok().map(|time| time.into_std()),
        #[cfg(not(unix))]
        modified: if metadata.is_dir() {
            None
        } else {
            metadata.modified().ok().map(|time| time.into_std())
        },
        #[cfg(not(unix))]
        length: if metadata.is_dir() { 0 } else { metadata.len() },
    }
}

fn rollback_links(root: &Dir, links: &[CreatedLink]) -> bool {
    let mut complete = true;
    for link in links.iter().rev() {
        match root.symlink_metadata(&link.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && cap_identity(&metadata) == link.identity
                    && root.read_link_contents(&link.path).ok().as_ref() == Some(&link.target) =>
            {
                if root.remove_file(&link.path).is_err() {
                    complete = false;
                }
            }
            _ => complete = false,
        }
    }
    complete
}

fn profile_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name.starts_with(".zsh")
        || name.starts_with(".bash")
        || matches!(name, ".profile" | "config.fish")
        || [".sh", ".zsh", ".bash", ".fish"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn read_profile(home: &Path, path: &Path) -> Result<Option<String>, String> {
    let root = Dir::open_ambient_dir(home, cap_std::ambient_authority())
        .map_err(|_| "Cannot inspect profile dependencies")?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = root
        .open_with(
            path.strip_prefix(home)
                .map_err(|_| "Invalid profile source")?,
            &options,
        )
        .map_err(|_| "Cannot inspect profile dependencies")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect a profile")?
        .is_file()
    {
        return Err("Profile source is not a regular file".into());
    }
    let mut data = Vec::new();
    file.take((MAX_PROFILE + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| "Cannot inspect profile dependencies")?;
    if data.len() > MAX_PROFILE {
        return Ok(None);
    }
    Ok(String::from_utf8(data).ok())
}

fn destination(workspace: &Path, path: &Path, target: &Path) -> Result<Destination, String> {
    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| "A link destination escaped the workspace")?;
    if relative.as_os_str().is_empty() {
        return Err("Cannot replace the workspace itself".into());
    }
    let mut current = workspace.to_owned();
    let mut parents = vec![(
        workspace.to_owned(),
        identity(&std::fs::symlink_metadata(workspace).map_err(|_| "Workspace unavailable")?),
    )];
    let components: Vec<_> = relative.components().collect();
    for (index, part) in components.iter().enumerate() {
        if !matches!(part, Component::Normal(_)) {
            return Err("Invalid workspace destination".into());
        }
        current.push(part.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if index + 1 == components.len() => {
                if !metadata.file_type().is_symlink() {
                    return Err(
                        "Workspace destination is already a Sprite-owned file or folder".into(),
                    );
                }
                let value =
                    std::fs::read_link(&current).map_err(|_| "Cannot inspect workspace link")?;
                if value != target {
                    return Err("Workspace destination is an unmanaged link".into());
                }
                let actual = current
                    .canonicalize()
                    .map_err(|_| "An existing workspace link is unavailable")?;
                if actual != target {
                    return Err("Workspace destination is already another share".into());
                }
                return Ok(Destination {
                    link: Some((value, identity(&metadata))),
                    parents,
                });
            }
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                parents.push((current.clone(), identity(&metadata)))
            }
            Ok(_) => return Err("Workspace link parents must be real private directories".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Destination {
                    link: None,
                    parents,
                });
            }
            Err(_) => return Err("Cannot inspect workspace destination".into()),
        }
    }
    Ok(Destination {
        link: None,
        parents,
    })
}

fn ensure_parents(root: &Dir, relative: &Path) -> Result<Vec<PathBuf>, String> {
    let mut current = PathBuf::new();
    let mut created = Vec::new();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("Invalid private workspace path".into());
        }
        current.push(component.as_os_str());
        match root.symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err("Workspace link parents must be real private directories".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                root.create_dir(&current)
                    .map_err(|_| "Cannot create a workspace folder")?;
                created.push(current.clone());
                #[cfg(unix)]
                {
                    use cap_std::fs::PermissionsExt;
                    root.set_permissions(&current, cap_std::fs::Permissions::from_mode(0o700))
                        .map_err(|_| "Cannot protect workspace folder")?;
                }
            }
            Err(_) => return Err("Cannot inspect workspace folder".into()),
        }
    }
    Ok(created)
}

fn create_link(root: &Dir, target: &Path, relative: &Path, directory: bool) -> Result<(), String> {
    #[cfg(unix)]
    let result = {
        let _ = directory;
        root.symlink_contents(target, relative)
    };
    #[cfg(windows)]
    let result = {
        if directory {
            root.symlink_dir(target, relative)
        } else {
            root.symlink_file(target, relative)
        }
    };
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, target, relative, directory);
        return Err("Live workspace links are not supported on this platform".into());
    }
    #[cfg(any(unix, windows))]
    result.map_err(|_| {
        "Cannot create the approved workspace link; its destination may have changed".into()
    })
}

/// Bounded metadata-only discovery. No file is opened for content and symlink
/// directories are never traversed, even if they point elsewhere inside home.
pub fn search_user_home(
    home: &Path,
    denied: &[PathBuf],
    args: &Value,
    cancellation: &CancellationToken,
) -> Result<Value, String> {
    if cancellation.is_cancelled() {
        return Err("Home search cancelled".into());
    }
    let home = home
        .canonicalize()
        .map_err(|_| "The user's home is unavailable")?;
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 200)
        .ok_or("Enter a filename or path search of 1 to 200 characters")?
        .to_lowercase();
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 100) as usize;
    let base = expand_home(
        &home,
        args.get("path").and_then(Value::as_str).unwrap_or("."),
        &home,
    )?;
    let actual = base
        .canonicalize()
        .map_err(|_| "The search folder is unavailable")?;
    expand_home(
        &home,
        actual.to_str().ok_or("Home paths must be valid text")?,
        &home,
    )?;
    let denied: Vec<_> = denied
        .iter()
        .map(|root| root.canonicalize().unwrap_or_else(|_| root.clone()))
        .collect();
    if denied.iter().any(|root| actual.starts_with(root)) {
        return Err("The search folder is in an inaccessible project".into());
    }
    let mut current = home.clone();
    for component in base
        .strip_prefix(&home)
        .map_err(|_| "Search escaped the home")?
        .components()
    {
        current.push(component.as_os_str());
        if std::fs::symlink_metadata(&current)
            .map_err(|_| "Cannot inspect search folder")?
            .file_type()
            .is_symlink()
        {
            return Err(
                "Choose a real folder; discovery does not traverse symlink directories".into(),
            );
        }
    }
    let root = Dir::open_ambient_dir(&home, cap_std::ambient_authority())
        .map_err(|_| "Cannot open the user's home")?;
    let mut stack = vec![(base, 0usize)];
    let mut visited = 0usize;
    let mut truncated = false;
    let mut matches = Vec::new();
    while let Some((directory, depth)) = stack.pop() {
        if cancellation.is_cancelled() {
            return Err("Home search cancelled".into());
        }
        let relative = directory
            .strip_prefix(&home)
            .map_err(|_| "Search escaped the home")?;
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        let entries = match root.read_dir(relative) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        let mut entries = entries
            .take(513)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Cannot inspect home names")?;
        if entries.len() > 512 {
            entries.truncate(512);
            truncated = true;
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if cancellation.is_cancelled() {
                return Err("Home search cancelled".into());
            }
            visited += 1;
            if visited > MAX_SEARCH_VISITS {
                truncated = true;
                break;
            }
            let path = directory.join(entry.file_name());
            let relative = path
                .strip_prefix(&home)
                .map_err(|_| "Search escaped the home")?;
            if relative
                .components()
                .any(|part| part.as_os_str() == ".solar")
            {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|_| "Cannot inspect a home name")?;
            let checked = if kind.is_symlink() {
                match path.canonicalize() {
                    Ok(path) => path,
                    Err(_) => continue,
                }
            } else {
                path.clone()
            };
            if !checked.starts_with(&home)
                || checked
                    .strip_prefix(&home)
                    .unwrap_or(&checked)
                    .components()
                    .any(|part| part.as_os_str() == ".solar")
                || denied.iter().any(|root| checked.starts_with(root))
            {
                continue;
            }
            let is_directory = kind.is_dir();
            let candidate_allowed = !denied_path(
                &checked,
                is_directory || (kind.is_symlink() && checked.is_dir()),
                &denied,
            );
            let text = relative.to_string_lossy().to_lowercase();
            if candidate_allowed && name_matches(&query, &text) {
                matches.push(json!({"path":format!("~/{}",relative.to_string_lossy()),"type":if kind.is_symlink(){"symlink"}else if is_directory{"directory"}else if kind.is_file(){"file"}else{"other"}}));
                if matches.len() >= limit {
                    truncated = true;
                    break;
                }
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if is_directory
                && depth < 8
                && !matches!(name.as_ref(), ".git" | "node_modules" | "target" | ".cache")
            {
                stack.push((path, depth + 1));
            }
        }
        if truncated {
            break;
        }
    }
    Ok(
        json!({"matches":matches,"inspected":visited,"truncated":truncated,"notice":"Names and file types only. Discovery does not grant access; request selected paths with pull_from_user_home for explicit approval."}),
    )
}

fn name_matches(query: &str, path: &str) -> bool {
    if !query.contains(['*', '?']) {
        return path.contains(query);
    }
    let query: Vec<_> = query.chars().collect();
    let path: Vec<_> = path.chars().collect();
    let (mut pattern, mut text, mut star, mut retry) = (0, 0, None, 0);
    while text < path.len() {
        if pattern < query.len() && (query[pattern] == '?' || query[pattern] == path[text]) {
            pattern += 1;
            text += 1;
        } else if pattern < query.len() && query[pattern] == '*' {
            star = Some(pattern);
            pattern += 1;
            retry = text;
        } else if let Some(position) = star {
            retry += 1;
            text = retry;
            pattern = position + 1;
        } else {
            return false;
        }
    }
    while pattern < query.len() && query[pattern] == '*' {
        pattern += 1;
    }
    pattern == query.len()
}

fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            word.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None
            } else {
                word.push(c)
            };
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        if c == '#' && word.is_empty() {
            break;
        }
        if c.is_whitespace() || matches!(c, ';' | '&' | '|') {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}
// Expand complete variable names only. This is substitution, never shell evaluation.
fn expand_variables(value: &str, variables: &BTreeMap<String, String>) -> Option<String> {
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '`' {
            return None;
        }
        if character != '$' {
            result.push(character);
            continue;
        }
        let mut name = String::new();
        if chars.peek() == Some(&'{') {
            chars.next();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(c) if c.is_ascii_alphanumeric() || c == '_' => name.push(c),
                    _ => return None,
                }
            }
        } else {
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
            {
                name.push(chars.next()?);
            }
        }
        if !name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            return None;
        }
        result.push_str(variables.get(&name)?);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().join("user");
        let storage = home.join(".solar/orbits/test");
        let workspace = storage.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        (temporary, home, storage, workspace)
    }

    #[test]
    fn variable_expansion_is_static_and_uses_complete_names() {
        let variables = BTreeMap::from([
            ("HOME".into(), "/home/user".into()),
            ("HOME_DIR".into(), "/home/user/profile".into()),
        ]);
        assert_eq!(
            expand_variables("$HOME_DIR/secrets", &variables).as_deref(),
            Some("/home/user/profile/secrets")
        );
        for value in [
            "$(touch bad)",
            "`pwd`",
            "${HOME:-/tmp}",
            "$HOME_SUFFIX/file",
        ] {
            assert!(expand_variables(value, &variables).is_none());
        }
    }

    #[test]
    fn folder_links_are_live_and_default_to_read_only() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::create_dir_all(home.join("notes/subfolder")).unwrap();
        std::fs::write(home.join("notes/subfolder/file"), "first").unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/notes"]}),
        )
        .unwrap();
        assert_eq!(
            plan.links(),
            vec![WorkspaceMount {
                path: workspace.join("notes"),
                target: home.join("notes"),
                writable: false
            }]
        );
        let review = plan.review();
        assert_eq!(review["links"][0]["directory"], true);
        assert_eq!(review["links"][0]["writable"], false);
        assert!(!review.to_string().contains("first"));
        let result = plan.apply(&CancellationToken::new()).unwrap();
        assert_eq!(result["linked"].as_array().unwrap().len(), 1);
        assert!(
            std::fs::symlink_metadata(workspace.join("notes"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(home.join("notes/subfolder/file"), "updated").unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("notes/subfolder/file")).unwrap(),
            "updated"
        );
    }

    #[test]
    fn profile_dependencies_are_reviewed_without_content_or_execution() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::create_dir_all(home.join("profiles")).unwrap();
        std::fs::write(
            home.join(".zshrc"),
            "HOME_DIR=\"$HOME/profiles\"\nsource \"$HOME_DIR/secrets\"\nsource $(touch unwanted)\n",
        )
        .unwrap();
        std::fs::write(home.join("profiles/secrets"), "TOKEN=super-private").unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/.zshrc"],"writable":true}),
        )
        .unwrap();
        assert_eq!(plan.links().len(), 2);
        assert!(plan.links().iter().all(|link| link.writable));
        assert!(!plan.review().to_string().contains("super-private"));
        assert!(!plan.review()["warnings"].as_array().unwrap().is_empty());
        plan.apply(&CancellationToken::new()).unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("profiles/secrets")).unwrap(),
            "TOKEN=super-private"
        );
        assert!(!home.join("unwanted").exists());
    }

    #[test]
    fn existing_private_files_and_configuration_aliases_are_not_overwritten() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::write(home.join(".zshrc"), "source ~/.zshrc.secrets").unwrap();
        std::fs::write(home.join(".zshrc.secrets"), "secret").unwrap();
        std::fs::write(workspace.join(".zshrc"), "private stub").unwrap();
        std::fs::create_dir_all(home.join(".config/app")).unwrap();
        std::fs::create_dir(storage.join("config")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(storage.join("config"), workspace.join(".config")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(storage.join("config"), workspace.join(".config"))
            .unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/.zshrc","~/.config/app"]}),
        )
        .unwrap();
        assert!(
            plan.links()
                .iter()
                .any(|link| link.path == workspace.join("shared/home/.zshrc"))
        );
        assert!(
            plan.links()
                .iter()
                .any(|link| link.path == workspace.join("shared/home/.config/app"))
        );
        plan.apply(&CancellationToken::new()).unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join(".zshrc")).unwrap(),
            "private stub"
        );
        assert_eq!(
            workspace.join(".config").canonicalize().unwrap(),
            storage.join("config")
        );
    }

    #[test]
    fn parent_and_child_requests_do_not_create_redundant_or_nested_links() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::create_dir_all(home.join("notes/subfolder")).unwrap();
        std::fs::write(home.join("notes/subfolder/file"), "text").unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/notes/subfolder/file","~/notes"]}),
        )
        .unwrap();
        assert_eq!(plan.links().len(), 1);
        plan.apply(&CancellationToken::new()).unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("notes/subfolder/file")).unwrap(),
            "text"
        );
    }

    #[test]
    fn whole_home_storage_and_denied_project_ancestors_are_rejected() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::create_dir_all(home.join("projects/secret")).unwrap();
        std::fs::write(home.join("projects/secret/file"), "secret").unwrap();
        for path in [
            "~",
            "~/..",
            "~/.solar",
            "~/projects",
            "~/projects/secret/file",
        ] {
            assert!(
                HomeImportPlan::prepare(
                    &home,
                    &workspace,
                    &storage,
                    &[home.join("projects/secret")],
                    &json!({"paths":[path]})
                )
                .is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn source_and_destination_changes_invalidate_pending_approval() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::write(home.join("notes"), "one").unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/notes"]}),
        )
        .unwrap();
        std::fs::rename(home.join("notes"), home.join("old-notes")).unwrap();
        std::fs::write(home.join("notes"), "two").unwrap();
        assert!(plan.apply(&CancellationToken::new()).is_err());
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/notes"]}),
        )
        .unwrap();
        std::fs::write(workspace.join("notes"), "unrelated").unwrap();
        assert!(plan.apply(&CancellationToken::new()).is_err());
        assert_eq!(
            std::fs::read_to_string(workspace.join("notes")).unwrap(),
            "unrelated"
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_alias_and_destination_parent_retargets_are_rejected() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::write(home.join("one"), "one").unwrap();
        std::fs::write(home.join("two"), "two").unwrap();
        std::os::unix::fs::symlink(home.join("one"), home.join("alias")).unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/alias"]}),
        )
        .unwrap();
        std::fs::remove_file(home.join("alias")).unwrap();
        std::os::unix::fs::symlink(home.join("two"), home.join("alias")).unwrap();
        assert!(plan.apply(&CancellationToken::new()).is_err());
        std::fs::create_dir_all(home.join("notes/subfolder")).unwrap();
        std::fs::write(home.join("notes/subfolder/file"), "one").unwrap();
        std::fs::create_dir(workspace.join("notes")).unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/notes/subfolder/file"]}),
        )
        .unwrap();
        std::fs::remove_dir(workspace.join("notes")).unwrap();
        std::os::unix::fs::symlink(home.join("notes"), workspace.join("notes")).unwrap();
        assert!(plan.apply(&CancellationToken::new()).is_err());
        assert!(!home.join("notes/subfolder/file").is_symlink());
    }

    #[test]
    fn revocation_and_cancellation_create_no_links() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::create_dir(home.join("notes")).unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &storage,
            &[],
            &json!({"paths":["~/notes"]}),
        )
        .unwrap();
        assert!(plan.validate_scope(&[home.join("notes")]).is_err());
        let token = CancellationToken::new();
        token.cancel();
        assert!(plan.apply(&token).is_err());
        assert!(!workspace.join("notes").exists());
    }

    #[test]
    fn rollback_removes_only_new_links_that_still_match_and_preserves_user_changes() {
        let (_temporary, home, _, workspace) = fixture();
        std::fs::write(home.join("source"), "original").unwrap();
        let root = Dir::open_ambient_dir(&workspace, cap_std::ambient_authority()).unwrap();
        let mut created = Vec::new();
        for name in ["owned", "changed"] {
            create_link(&root, &home.join("source"), Path::new(name), false).unwrap();
            created.push(CreatedLink {
                path: name.into(),
                target: home.join("source"),
                identity: cap_identity(&root.symlink_metadata(name).unwrap()),
            });
        }
        std::fs::remove_file(workspace.join("changed")).unwrap();
        std::fs::write(workspace.join("changed"), "user-created").unwrap();
        assert!(!rollback_links(&root, &created));
        assert!(!workspace.join("owned").exists());
        assert_eq!(
            std::fs::read_to_string(workspace.join("changed")).unwrap(),
            "user-created"
        );
        assert_eq!(
            std::fs::read_to_string(home.join("source")).unwrap(),
            "original"
        );
    }

    #[test]
    fn uncommitted_receipts_roll_back_new_links_and_preserve_existing_ones() {
        let (_temporary, home, storage, workspace) = fixture();
        std::fs::write(home.join("new"), "new").unwrap();
        std::fs::write(home.join("existing"), "existing").unwrap();
        let root = Dir::open_ambient_dir(&workspace, cap_std::ambient_authority()).unwrap();
        create_link(&root, &home.join("existing"), Path::new("existing"), false).unwrap();
        let plan = || {
            HomeImportPlan::prepare(
                &home,
                &workspace,
                &storage,
                &[],
                &json!({"paths":["~/new","~/existing"]}),
            )
            .unwrap()
        };
        let receipt = plan()
            .apply_with_receipt(&CancellationToken::new())
            .unwrap();
        assert_eq!(receipt.mounts().len(), 2);
        assert_eq!(receipt.result()["linked"].as_array().unwrap().len(), 2);
        assert!(workspace.join("new").exists());
        drop(receipt);
        assert!(!workspace.join("new").exists());
        assert!(workspace.join("existing").exists());
        let receipt = plan()
            .apply_with_receipt(&CancellationToken::new())
            .unwrap();
        receipt.rollback().unwrap();
        assert!(!workspace.join("new").exists());
        let result = plan()
            .apply_with_receipt(&CancellationToken::new())
            .unwrap()
            .commit();
        assert_eq!(result["linked"].as_array().unwrap().len(), 2);
        assert!(workspace.join("new").exists());
        assert!(workspace.join("existing").exists());
    }

    #[cfg(unix)]
    #[test]
    fn metadata_search_skips_storage_denied_projects_and_symlink_traversal() {
        let (_temporary, home, storage, _) = fixture();
        std::fs::create_dir_all(home.join("allowed/child")).unwrap();
        std::fs::create_dir(home.join("secret")).unwrap();
        std::fs::create_dir(home.join(".ssh")).unwrap();
        std::fs::write(
            home.join("allowed/child/profile.txt"),
            "SECRET_CONTENT_NEVER_RETURNED",
        )
        .unwrap();
        std::fs::write(home.join("secret/private.txt"), "denied").unwrap();
        std::fs::write(storage.join("profile-private.txt"), "storage").unwrap();
        std::os::unix::fs::symlink(home.join("allowed"), home.join("alias")).unwrap();
        // A special file matching the query proves discovery does not read contents.
        let fifo =
            std::ffi::CString::new(home.join(".ssh/profile-key").to_string_lossy().as_bytes())
                .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let result = search_user_home(
            &home,
            &[home.join("secret")],
            &json!({"query":"*profile*"}),
            &CancellationToken::new(),
        )
        .unwrap();
        let text = result.to_string();
        assert!(text.contains("allowed/child/profile.txt"));
        assert!(text.contains(".ssh/profile-key"));
        assert!(!text.contains("SECRET_CONTENT"));
        assert!(!text.contains("profile-private"));
        assert!(!text.contains("alias/child"));
        assert!(!text.contains("private.txt"));
        assert!(
            search_user_home(
                &home,
                &[],
                &json!({"query":"*","path":"alias/child"}),
                &CancellationToken::new()
            )
            .is_err()
        );
        let limited = search_user_home(
            &home,
            &[],
            &json!({"query":"*","limit":1}),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(limited["matches"].as_array().unwrap().len(), 1);
        assert_eq!(limited["truncated"], true);
    }

    #[test]
    fn cancelled_name_search_does_not_traverse() {
        let (_temporary, home, _, _) = fixture();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(search_user_home(&home, &[], &json!({"query":"*"}), &cancellation).is_err());
    }
}
