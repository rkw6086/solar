//! User-owned filesystem grants, represented by live links in each workspace.
use crate::{
    engine::{Engine, validate_id, validate_text},
    model::{Orbit, Project, SharedFolder},
};
use serde::Serialize;
use solar_tools::WorkspaceMount;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HomeShareCandidate {
    id: String,
    name: String,
    directory: String,
    mount_path: String,
    description: String,
    sensitive: bool,
}

pub(crate) fn relative_mount(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 512
        || value.chars().any(char::is_control)
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || path
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == ".solar")
    {
        return Err(
            "Choose a relative workspace path without '..' or reserved .solar files".into(),
        );
    }
    Ok(path.into())
}

pub(crate) fn validate_shared_folder(folder: &SharedFolder) -> Result<(), String> {
    validate_id(&folder.id)?;
    validate_text(&folder.name, "Shared folder name", 200)?;
    let mount = relative_mount(&folder.mount_path)?;
    if mount.starts_with(".tmp") {
        return Err("The .tmp workspace folder is reserved for private tool files".into());
    }
    if mount.starts_with("projects") {
        return Err("The projects folder is reserved for assigned projects".into());
    }
    for default in [".config", ".cache", ".local/share", ".local/state"] {
        let default = Path::new(default);
        if mount != default && (mount.starts_with(default) || default.starts_with(&mount)) {
            return Err(
                "Choose an unused shared/ path instead of nesting inside a configuration alias"
                    .into(),
            );
        }
    }
    if !Path::new(&folder.directory).is_absolute() || folder.directory.chars().any(char::is_control)
    {
        return Err("Select an absolute local file or folder path".into());
    }
    for id in &folder.orbit_ids {
        validate_id(id)?;
    }
    if !folder.all_sprites && folder.orbit_ids.is_empty() {
        return Err("Choose at least one Sprite, or share with all Sprites".into());
    }
    Ok(())
}

fn selected(folder: &SharedFolder, orbit: &Orbit) -> bool {
    folder.all_sprites || folder.orbit_ids.contains(&orbit.id)
}

pub(crate) fn project_mount(project: &Project) -> String {
    format!("projects/{}", project.id)
}

fn real_directory(path: &Path) -> Result<(), String> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(format!(
                "Workspace directory must be a real folder: {}",
                path.display()
            ));
        }
    } else {
        std::fs::create_dir(path).map_err(|e| format!("Cannot create workspace folder: {e}"))?;
    }
    Ok(())
}

fn ensure_parents(root: &cap_std::fs::Dir, relative: &Path) -> Result<(), String> {
    let mut path = PathBuf::new();
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            path.push(component);
            match root.symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err("Workspace link parents must be real private directories".into());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    root.create_dir(&path).map_err(|e| e.to_string())?
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    Ok(())
}

fn create_link(root: &cap_std::fs::Dir, target: &Path, path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    let result = root.symlink_contents(target, path);
    #[cfg(windows)]
    let result = if target.is_dir() {
        root.symlink_dir(target, path)
    } else {
        root.symlink_file(target, path)
    };
    result.map_err(|e| format!("Cannot create workspace link: {e}"))
}

struct LinkPlan {
    storage: PathBuf,
    workspace: PathBuf,
    desired: BTreeMap<PathBuf, (PathBuf, bool, bool)>,
    history: Vec<WorkspaceMount>,
}

fn read_links(path: &Path) -> Result<Vec<WorkspaceMount>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "Workspace link manifest is invalid".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}

fn write_links(path: &Path, links: &[WorkspaceMount]) -> Result<(), String> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(links).map_err(|e| e.to_string())?;
    let temporary = path.with_file_name(format!(".workspace-links-{}.tmp", crate::model::id()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

impl Engine {
    pub(crate) fn validate_share_source(&self, source: &str) -> Result<PathBuf, String> {
        let source = std::fs::canonicalize(source)
            .map_err(|_| "The selected file or folder is unavailable")?;
        if ["/proc", "/sys", "/dev", "/run", "/private/var/run"]
            .iter()
            .any(|root| source.starts_with(root))
        {
            return Err("System process, device, and service directories cannot be shared as workspace data".into());
        }
        let root = self
            .data_dir
            .canonicalize()
            .map_err(|_| "Solar storage is unavailable")?;
        if source.parent().is_none() || source.starts_with(&root) || root.starts_with(&source) {
            return Err("Share a specific folder outside Solar's private storage, not the whole home or filesystem".into());
        }
        if std::env::var_os("HOME")
            .and_then(|home| PathBuf::from(home).canonicalize().ok())
            .is_some_and(|home| home == source)
        {
            return Err("Share specific home files or folders, not the entire home".into());
        }
        let meta = std::fs::metadata(&source).map_err(|e| e.to_string())?;
        if !meta.is_file() && !meta.is_dir() {
            return Err("Share a regular file or folder".into());
        }
        Ok(source)
    }

    pub fn shared_folders(&self) -> Result<Vec<SharedFolder>, String> {
        self.store.list("shared_folders")
    }

    pub fn save_shared_folder(&self, mut folder: SharedFolder) -> Result<SharedFolder, String> {
        let _guard = self
            .sharing_write
            .lock()
            .map_err(|_| "Sharing settings are unavailable")?;
        self.ensure_running()?;
        if folder.id.is_empty() {
            folder.id = crate::model::id();
        }
        if folder.mount_path.is_empty() {
            folder.mount_path = format!("shared/{}", folder.id);
        }
        folder.name = folder.name.trim().to_owned();
        validate_shared_folder(&folder)?;
        folder.directory = self
            .validate_share_source(&folder.directory)?
            .to_str()
            .ok_or("Shared folders must have a UTF-8 path")?
            .to_owned();
        folder.orbit_ids.sort();
        folder.orbit_ids.dedup();
        for orbit_id in &folder.orbit_ids {
            self.orbit(orbit_id)?;
        }
        let previous = self
            .store
            .get::<SharedFolder>("shared_folders", &folder.id)?;
        for existing in self.shared_folders()?.iter().filter(|f| f.id != folder.id) {
            let overlapping_scope = folder.all_sprites
                || existing.all_sprites
                || folder
                    .orbit_ids
                    .iter()
                    .any(|id| existing.orbit_ids.contains(id));
            let a = relative_mount(&folder.mount_path)?;
            let b = relative_mount(&existing.mount_path)?;
            if overlapping_scope && (a.starts_with(&b) || b.starts_with(&a)) {
                return Err("Workspace share paths cannot overlap for the same Sprites".into());
            }
        }

        // Never allow a share to expand access to a registered project that the
        // selected Sprite was not given. This includes a shared ancestor folder.
        for orbit in self
            .store
            .list::<Orbit>("orbits")?
            .iter()
            .filter(|o| selected(&folder, o))
        {
            for project in self.store.list::<Project>("projects")? {
                let project_path = Path::new(&project.directory);
                let shared_path = Path::new(&folder.directory);
                if !orbit.project_ids.contains(&project.id)
                    && (project_path.starts_with(shared_path)
                        || shared_path.starts_with(project_path))
                {
                    return Err(format!(
                        "Grant {} access to project {} before sharing this folder",
                        orbit.name, project.name
                    ));
                }
            }
            for existing in self
                .shared_folders()?
                .iter()
                .filter(|f| f.id != folder.id && selected(f, orbit))
            {
                let a = relative_mount(&folder.mount_path)?;
                let b = relative_mount(&existing.mount_path)?;
                if a.starts_with(&b) || b.starts_with(&a) {
                    return Err("Workspace share paths cannot overlap".into());
                }
            }
            if Path::new(&folder.mount_path).starts_with("projects") {
                return Err("The projects folder is reserved for assigned projects".into());
            }
        }
        if previous.is_some() {
            self.cancel_workspace_tasks()?;
        }
        self.store.put("shared_folders", &folder.id, &folder)?;
        if let Err(error) = self.sync_all_workspaces() {
            if let Some(previous) = previous {
                self.store.put("shared_folders", &folder.id, &previous)?;
            } else {
                self.store.remove("shared_folders", &folder.id)?;
            }
            let _ = self.sync_all_workspaces();
            return Err(error);
        }
        self.changed();
        Ok(folder)
    }

    pub(crate) fn register_home_links(
        &self,
        orbit_id: &str,
        task_id: &str,
        links: &[WorkspaceMount],
    ) -> Result<(), String> {
        let _guard = self
            .sharing_write
            .lock()
            .map_err(|_| "Sharing settings are unavailable")?;
        self.ensure_running()?;
        let workspace = self.workspace(orbit_id)?;
        let orbit = self.orbit(orbit_id)?;
        let previous = self.shared_folders()?;
        let mut folders = previous.clone();
        for link in links {
            let mount_path = link
                .path
                .strip_prefix(&workspace)
                .map_err(|_| "Approved link escaped its workspace")?
                .to_str()
                .ok_or("Link paths must be UTF-8")?
                .to_owned();
            let directory = link
                .target
                .to_str()
                .ok_or("Link targets must be UTF-8")?
                .to_owned();
            if self.approved_source(&directory)? != link.target {
                return Err("The approved source changed".into());
            }
            for project in self.store.list::<Project>("projects")? {
                let path = Path::new(&project.directory);
                if !orbit.project_ids.contains(&project.id)
                    && (link.target.starts_with(path) || path.starts_with(&link.target))
                {
                    return Err("A project grant changed while approving the home request".into());
                }
            }
            if let Some(existing) = folders.iter().find(|folder| {
                selected(folder, &orbit)
                    && folder.mount_path == mount_path
                    && folder.directory == directory
                    && (folder.all_sprites || folder.orbit_ids.len() != 1)
            }) {
                if existing.writable != link.writable {
                    return Err("This item is already shared with multiple Sprites; change its access in Projects > Shared folders".into());
                }
                continue;
            }
            let existing = folders.iter_mut().find(|folder| {
                !folder.all_sprites
                    && folder.orbit_ids.len() == 1
                    && folder.orbit_ids[0] == orbit_id
                    && folder.mount_path == mount_path
                    && folder.directory == directory
            });
            if let Some(folder) = existing {
                folder.writable = link.writable;
            } else {
                folders.push(SharedFolder {
                    id: crate::model::id(),
                    name: link
                        .target
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    directory,
                    mount_path,
                    all_sprites: false,
                    orbit_ids: vec![orbit_id.into()],
                    writable: link.writable,
                });
            }
        }
        for (index, folder) in folders.iter().enumerate() {
            validate_shared_folder(folder)?;
            for other in folders.iter().skip(index + 1) {
                let overlap = folder.all_sprites
                    || other.all_sprites
                    || folder
                        .orbit_ids
                        .iter()
                        .any(|id| other.orbit_ids.contains(id));
                let a = relative_mount(&folder.mount_path)?;
                let b = relative_mount(&other.mount_path)?;
                if overlap && (a.starts_with(&b) || b.starts_with(&a)) {
                    return Err("An existing share overlaps this requested workspace link".into());
                }
            }
        }
        self.invalidate_workspace_sessions();
        for (id, token) in self
            .active
            .lock()
            .map_err(|_| "Task state is unavailable")?
            .iter()
        {
            if id != task_id {
                token.cancel();
            }
        }
        self.store.replace_shared_folders(folders)?;
        if let Err(error) = self.sync_all_workspaces() {
            self.store.replace_shared_folders(previous)?;
            let _ = self.sync_all_workspaces();
            return Err(error);
        }
        self.changed();
        Ok(())
    }

    pub fn delete_shared_folder(&self, id: &str) -> Result<(), String> {
        let _guard = self
            .sharing_write
            .lock()
            .map_err(|_| "Sharing settings are unavailable")?;
        self.ensure_running()?;
        self.cancel_workspace_tasks()?;
        let previous = self.store.get::<SharedFolder>("shared_folders", id)?;
        self.store.remove("shared_folders", id)?;
        if let Err(error) = self.sync_all_workspaces() {
            if let Some(previous) = previous {
                self.store.put("shared_folders", id, &previous)?;
            }
            let _ = self.sync_all_workspaces();
            return Err(error);
        }
        self.changed();
        Ok(())
    }

    fn cancel_workspace_tasks(&self) -> Result<(), String> {
        self.invalidate_workspace_sessions();
        for token in self
            .active
            .lock()
            .map_err(|_| "Task state is unavailable")?
            .values()
        {
            token.cancel();
        }
        Ok(())
    }

    pub(crate) fn sync_all_workspaces(&self) -> Result<(), String> {
        let orbits = self.store.list::<Orbit>("orbits")?;
        // Validate every destination before changing the first Sprite. A bad
        // private-file collision must not leave an earlier workspace changed.
        for orbit in &orbits {
            self.workspace_plan(&orbit.id)?;
        }
        for orbit in &orbits {
            self.sync_workspace(&orbit.id)?;
        }
        Ok(())
    }

    pub(crate) fn approved_source(&self, source: &str) -> Result<PathBuf, String> {
        let actual = self.validate_share_source(source)?;
        if actual != Path::new(source) {
            return Err(
                "A shared source was redirected. Select it again to approve its new location"
                    .into(),
            );
        }
        Ok(actual)
    }

    fn workspace_plan(&self, orbit_id: &str) -> Result<LinkPlan, String> {
        let orbit = self.orbit(orbit_id)?;
        let storage = self
            .store
            .orbit_dir(orbit_id)?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let workspace = storage.join("workspace");
        real_directory(&workspace)?;
        let root = cap_std::fs::Dir::open_ambient_dir(&workspace, cap_std::ambient_authority())
            .map_err(|e| e.to_string())?;
        let mut desired = BTreeMap::<PathBuf, (PathBuf, bool, bool)>::new();
        for (relative, child) in [
            (".config", "config"),
            (".cache", "cache"),
            (".local/share", "data"),
            (".local/state", "state"),
            (".tmp", "tmp"),
        ] {
            real_directory(&storage.join(child))?;
            desired.insert(relative.into(), (storage.join(child), true, true));
        }
        for project in self.store.list::<Project>("projects")? {
            if orbit.project_ids.contains(&project.id) {
                desired.insert(
                    project_mount(&project).into(),
                    (self.approved_source(&project.directory)?, true, false),
                );
            }
        }
        for folder in self
            .shared_folders()?
            .iter()
            .filter(|f| selected(f, &orbit))
        {
            validate_shared_folder(folder)?;
            let relative = relative_mount(&folder.mount_path)?;
            if desired
                .get(&relative)
                .is_some_and(|(_, _, private)| !private)
            {
                return Err("Two grants use the same workspace path".into());
            }
            desired.insert(
                relative,
                (
                    self.approved_source(&folder.directory)?,
                    folder.writable,
                    false,
                ),
            );
        }
        let keys: Vec<_> = desired.keys().collect();
        if keys.iter().enumerate().any(|(i, a)| {
            keys.iter()
                .skip(i + 1)
                .any(|b| a.starts_with(b) || b.starts_with(a))
        }) {
            return Err("Workspace links cannot overlap".into());
        }
        let mut history = read_links(&storage.join("workspace-links.json"))?;
        history.extend(read_links(&storage.join("workspace-links.pending.json"))?);
        for old in &history {
            let relative = old
                .path
                .strip_prefix(&workspace)
                .map_err(|_| "Workspace link manifest escapes its workspace")?;
            relative_mount(relative.to_str().ok_or("Invalid workspace link path")?)?;
        }
        // These checks perform no link mutations and cap-std keeps any later
        // parent substitution from traversing outside the workspace.
        for (relative, (target, _, private)) in &desired {
            let mut parent = PathBuf::new();
            if let Some(path) = relative.parent() {
                for component in path.components() {
                    parent.push(component);
                    match root.symlink_metadata(&parent) {
                        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
                        Ok(_) => {
                            return Err(
                                "Workspace link parents must be real private directories".into()
                            );
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                        Err(e) => return Err(e.to_string()),
                    }
                }
            }
            match root.symlink_metadata(relative) {
                Ok(m) if m.file_type().is_symlink() => {
                    let actual = root
                        .read_link_contents(relative)
                        .map_err(|e| e.to_string())?;
                    if actual != *target
                        && !history
                            .iter()
                            .any(|old| old.path == workspace.join(relative) && old.target == actual)
                    {
                        return Err(format!(
                            "{} is an unrecognized link; choose an unused workspace path",
                            relative.display()
                        ));
                    }
                }
                Ok(_) if *private => {}
                Ok(_) => {
                    return Err(format!(
                        "{} already exists in the workspace; choose an unused share path",
                        relative.display()
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        for old in &history {
            let relative = old
                .path
                .strip_prefix(&workspace)
                .map_err(|e| e.to_string())?;
            if !desired.contains_key(relative)
                && let Ok(actual) = root.read_link_contents(relative)
                && !history
                    .iter()
                    .any(|known| known.path == old.path && known.target == actual)
            {
                return Err("A managed workspace link was changed outside Solar".into());
            }
        }
        Ok(LinkPlan {
            storage,
            workspace,
            desired,
            history,
        })
    }

    pub(crate) fn sync_workspace(&self, orbit_id: &str) -> Result<Vec<WorkspaceMount>, String> {
        let _guard = self
            .workspace_write
            .lock()
            .map_err(|_| "Workspace links are unavailable")?;
        let plan = self.workspace_plan(orbit_id)?;
        let root =
            cap_std::fs::Dir::open_ambient_dir(&plan.workspace, cap_std::ambient_authority())
                .map_err(|e| e.to_string())?;
        let pending: Vec<_> = plan
            .desired
            .iter()
            .map(|(relative, (target, writable, _))| WorkspaceMount {
                path: plan.workspace.join(relative),
                target: target.clone(),
                writable: *writable,
            })
            .chain(plan.history.iter().cloned())
            .collect();
        // Retain both old/new known targets before touching links. Interrupted
        // updates can be safely completed or rolled back without adopting an
        // unrecognized external link.
        write_links(&plan.storage.join("workspace-links.pending.json"), &pending)?;
        for old in &plan.history {
            let relative = old
                .path
                .strip_prefix(&plan.workspace)
                .map_err(|e| e.to_string())?;
            if !plan.desired.contains_key(relative)
                && let Ok(actual) = root.read_link_contents(relative)
            {
                if !plan
                    .history
                    .iter()
                    .any(|known| known.path == old.path && known.target == actual)
                {
                    return Err("A managed workspace link was changed outside Solar".into());
                }
                root.remove_file(relative).map_err(|e| e.to_string())?;
            }
        }
        let mut mounts = Vec::new();
        for (relative, (target, writable, private)) in plan.desired {
            ensure_parents(&root, &relative)?;
            let path = plan.workspace.join(&relative);
            match root.symlink_metadata(&relative) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let actual = root
                        .read_link_contents(&relative)
                        .map_err(|e| e.to_string())?;
                    if actual != target {
                        if !plan
                            .history
                            .iter()
                            .any(|old| old.path == path && old.target == actual)
                        {
                            return Err("A workspace link changed while saving".into());
                        }
                        root.remove_file(&relative).map_err(|e| e.to_string())?;
                        create_link(&root, &target, &relative)?;
                    }
                }
                Ok(_) if private => continue,
                Ok(_) => {
                    return Err(format!(
                        "{} already exists in the workspace",
                        relative.display()
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    create_link(&root, &target, &relative)?
                }
                Err(e) => return Err(e.to_string()),
            }
            mounts.push(WorkspaceMount {
                path,
                target,
                writable,
            });
        }
        write_links(&plan.storage.join("workspace-links.json"), &mounts)?;
        std::fs::remove_file(plan.storage.join("workspace-links.pending.json"))
            .map_err(|e| e.to_string())?;
        Ok(mounts)
    }

    pub(crate) fn discover_home_shares(&self) -> Result<Vec<HomeShareCandidate>, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("Home folder is unavailable")?;
        discover_in_home(&home)
    }
}

fn discover_in_home(home: &Path) -> Result<Vec<HomeShareCandidate>, String> {
    let home = home
        .canonicalize()
        .map_err(|_| "Home folder is unavailable")?;
    let mut result = Vec::new();
    for (path, name, description, sensitive) in [
        (
            ".config",
            "Application configuration",
            "Shared command-line and application settings; may include login tokens.",
            true,
        ),
        (
            ".kube",
            "Kubernetes",
            "Cluster configuration and credentials.",
            true,
        ),
        (
            ".aws",
            "AWS configuration",
            "Cloud profiles and credentials. Share only with Sprites that need these accounts.",
            true,
        ),
        (
            ".azure",
            "Azure configuration",
            "Cloud profiles and credentials.",
            true,
        ),
        (
            ".docker",
            "Docker configuration",
            "Registry logins and CLI configuration; this does not grant Docker socket access.",
            true,
        ),
        (
            ".minikube",
            "Local Kubernetes configuration",
            "Local cluster certificates and configuration.",
            true,
        ),
        (
            ".claude",
            "Claude configuration",
            "Global agent instructions, skills, and configuration; may include credentials.",
            true,
        ),
        (
            ".codex",
            "Codex configuration",
            "Global agent instructions, skills, and configuration; may include credentials.",
            true,
        ),
        (
            ".nvm",
            "Node toolchains",
            "Installed Node versions and global tools stored in your home. Read-only access can make these tools available to Sprites.",
            false,
        ),
        (
            ".volta",
            "Volta toolchains",
            "Installed JavaScript tools stored in your home.",
            false,
        ),
        (
            ".cargo",
            "Rust tools and configuration",
            "Rust command-line tools and registry configuration; may include registry credentials.",
            true,
        ),
        (
            ".rustup",
            "Rust toolchains",
            "Installed Rust toolchains stored in your home.",
            false,
        ),
        (
            ".bun",
            "Bun tools",
            "Bun runtime and global tools stored in your home.",
            false,
        ),
        (
            ".cursor",
            "Cursor configuration",
            "Global editor agent configuration and instructions.",
            true,
        ),
        (
            ".continue",
            "Continue configuration",
            "Global agent configuration and instructions.",
            true,
        ),
        (
            ".ssh",
            "SSH configuration",
            "SSH configuration and keys. Share only if all selected Sprites should use these credentials.",
            true,
        ),
        (
            ".agents",
            "Agent skills",
            "Global agent skills and instructions.",
            false,
        ),
        (
            ".gemini",
            "Gemini configuration",
            "Global agent configuration and instructions; may include credentials.",
            true,
        ),
        (
            "AGENTS.md",
            "Global agent instructions",
            "Home-level AGENTS.md instructions.",
            false,
        ),
        (
            "CLAUDE.md",
            "Global Claude instructions",
            "Home-level CLAUDE.md instructions.",
            false,
        ),
        (
            ".gitconfig",
            "Git configuration",
            "Global Git settings and includes.",
            true,
        ),
        (
            ".zshrc",
            "Zsh profile",
            "Shell startup code and environment settings.",
            true,
        ),
        (
            ".bashrc",
            "Bash profile",
            "Shell startup code and environment settings.",
            true,
        ),
        (
            ".profile",
            "Login profile",
            "Shell login settings and environment.",
            true,
        ),
    ] {
        let source = home.join(path);
        let Ok(actual) = source.canonicalize() else {
            continue;
        };
        if !actual.starts_with(&home) || (!actual.is_dir() && !actual.is_file()) {
            continue;
        }
        result.push(HomeShareCandidate {
            id: path.replace('.', "_").replace('/', "-"),
            name: name.into(),
            directory: actual.to_string_lossy().into_owned(),
            mount_path: path.into(),
            description: description.into(),
            sensitive,
        });
    }
    Ok(result)
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn discover_home_shares(
    window: tauri::WebviewWindow,
    engine: tauri::State<'_, std::sync::Arc<Engine>>,
) -> Result<Vec<HomeShareCandidate>, String> {
    if window.label() != "main" {
        return Err("Manage shared folders in the main Solar window".into());
    }
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.discover_home_shares())
        .await
        .map_err(|_| "Home discovery was interrupted".to_owned())?
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn save_shared_folder(
    window: tauri::WebviewWindow,
    engine: tauri::State<'_, std::sync::Arc<Engine>>,
    folder: SharedFolder,
) -> Result<SharedFolder, String> {
    if window.label() != "main" {
        return Err("Manage shared folders in the main Solar window".into());
    }
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.save_shared_folder(folder))
        .await
        .map_err(|_| "Sharing update was interrupted".to_owned())?
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn delete_shared_folder(
    window: tauri::WebviewWindow,
    engine: tauri::State<'_, std::sync::Arc<Engine>>,
    id: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Manage shared folders in the main Solar window".into());
    }
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.delete_shared_folder(&id))
        .await
        .map_err(|_| "Sharing update was interrupted".to_owned())?
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn choose_shared_directory(
    window: tauri::WebviewWindow,
    directory: Option<String>,
) -> Result<Option<serde_json::Value>, String> {
    use tauri_plugin_dialog::DialogExt;
    if window.label() != "main" {
        return Err("Manage shared folders in the main Solar window".into());
    }
    let mut picker = window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Choose a shared folder");
    if let Some(directory) = directory.filter(|p| Path::new(p).is_dir()) {
        picker = picker.set_directory(directory);
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    picker.pick_folder(move |folder| {
        let _ = sender.send(folder);
    });
    let Some(folder) = receiver
        .await
        .map_err(|_| "Folder picker closed unexpectedly")?
    else {
        return Ok(None);
    };
    let path = folder
        .into_path()
        .map_err(|_| "Select a local folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    Ok(Some(
        serde_json::json!({"directory":path.to_string_lossy(),"name":path.file_name().unwrap_or_default().to_string_lossy()}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_relative_and_discovery_is_opt_in_metadata_only() {
        for invalid in ["", "/tmp", "../outside", "a/../b", ".solar/state", "a\n"] {
            assert!(relative_mount(invalid).is_err(), "{invalid}");
        }
        assert!(relative_mount(".config").is_ok());
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".kube")).unwrap();
        std::fs::write(home.path().join("AGENTS.md"), "never expose content").unwrap();
        let values = discover_in_home(home.path()).unwrap();
        assert_eq!(values.len(), 2);
        let json = serde_json::to_string(&values).unwrap();
        assert!(!json.contains("never expose content"));
        assert!(!json.contains(".solar"));
    }

    fn share(path: &Path, mount: &str) -> SharedFolder {
        SharedFolder {
            id: "shared-test".into(),
            name: "Shared files".into(),
            directory: path.to_string_lossy().into_owned(),
            mount_path: mount.into(),
            all_sprites: true,
            orbit_ids: vec![],
            writable: false,
        }
    }

    #[test]
    fn global_links_are_live_private_by_default_and_apply_to_future_sprites() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        engine
            .save_shared_folder(share(external.path(), "shared/docs"))
            .unwrap();
        assert!(engine.bootstrap().unwrap().orbits.is_empty());
        crate::test_support::add_assistant(&engine);
        let path = engine.workspace("assistant").unwrap().join("shared/docs");
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(external.path().join("live.txt"), "first").unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("live.txt")).unwrap(),
            "first"
        );
        std::fs::write(external.path().join("live.txt"), "second").unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("live.txt")).unwrap(),
            "second"
        );
        assert!(
            !engine
                .sync_workspace("assistant")
                .unwrap()
                .iter()
                .find(|m| m.path == path)
                .unwrap()
                .writable
        );
        let mut other = engine.orbit("assistant").unwrap();
        other.id = "other".into();
        other.name = "Other".into();
        engine.save_orbit(other).unwrap();
        assert!(
            engine
                .workspace("other")
                .unwrap()
                .join("shared/docs/live.txt")
                .exists()
        );
        drop(engine);
        let engine = Engine::open(root.path().into()).unwrap();
        assert_eq!(engine.shared_folders().unwrap().len(), 1);
        assert!(engine.shared_folders().unwrap()[0].all_sprites);
        assert!(
            engine
                .sync_workspace("other")
                .unwrap()
                .iter()
                .any(|m| m.path.ends_with("shared/docs"))
        );
        engine.delete_shared_folder("shared-test").unwrap();
        assert!(!path.exists());
        assert!(external.path().join("live.txt").exists());
        drop(engine);
        let restored = Engine::open(root.path().into()).unwrap();
        assert!(restored.shared_folders().unwrap().is_empty());
    }

    #[test]
    fn sharing_collisions_are_rejected_before_any_sprite_exists() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        engine
            .save_shared_folder(share(outside.path(), "shared/docs"))
            .unwrap();
        let mut duplicate = share(outside.path(), "shared/docs/nested");
        duplicate.id = "another".into();
        assert!(engine.save_shared_folder(duplicate).is_err());
        assert_eq!(engine.shared_folders().unwrap().len(), 1);
    }

    #[test]
    fn a_collision_in_second_sprite_does_not_change_the_first() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let mut other = engine.orbit("assistant").unwrap();
        other.id = "other".into();
        other.name = "Other".into();
        engine.save_orbit(other).unwrap();
        let collision = engine.workspace("other").unwrap().join("shared/docs");
        std::fs::create_dir_all(collision.parent().unwrap()).unwrap();
        std::fs::write(&collision, "keep me").unwrap();
        assert!(
            engine
                .save_shared_folder(share(outside.path(), "shared/docs"))
                .is_err()
        );
        assert!(
            !engine
                .workspace("assistant")
                .unwrap()
                .join("shared/docs")
                .exists()
        );
        assert_eq!(std::fs::read_to_string(collision).unwrap(), "keep me");
        assert!(engine.shared_folders().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn shared_source_and_managed_link_retargeting_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = outside.path().join("source");
        let secret = outside.path().join("secret");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&secret).unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        engine
            .save_shared_folder(share(&source, "shared/docs"))
            .unwrap();
        let path = engine.workspace("assistant").unwrap().join("shared/docs");
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&secret, &path).unwrap();
        assert!(engine.sync_workspace("assistant").is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&source, &path).unwrap();
        std::fs::remove_dir(&source).unwrap();
        std::os::unix::fs::symlink(&secret, &source).unwrap();
        assert!(engine.sync_workspace("assistant").is_err());
    }

    #[test]
    fn projects_are_linked_only_for_assigned_sprites_and_revocation_removes_link() {
        let root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        engine
            .save_project(Project {
                id: "project".into(),
                name: String::new(),
                directory: project.path().to_string_lossy().into_owned(),
                description: String::new(),
            })
            .unwrap();
        let link = engine
            .workspace("assistant")
            .unwrap()
            .join("projects/project");
        assert!(!link.exists());
        let mut orbit = engine.orbit("assistant").unwrap();
        orbit.project_ids.push("project".into());
        engine.save_orbit(orbit.clone()).unwrap();
        assert!(link.exists());
        orbit.project_ids.clear();
        engine.save_orbit(orbit).unwrap();
        assert!(!link.exists());
        assert!(project.path().exists());
    }

    #[test]
    fn explicit_config_share_restores_private_config_after_revocation() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let alias = engine.workspace("assistant").unwrap().join(".config");
        let original = alias.canonicalize().unwrap();
        std::fs::write(original.join("keep.txt"), "private").unwrap();
        engine
            .save_shared_folder(share(config.path(), ".config"))
            .unwrap();
        assert_eq!(
            alias.canonicalize().unwrap(),
            config.path().canonicalize().unwrap()
        );
        engine.delete_shared_folder("shared-test").unwrap();
        assert_eq!(alias.canonicalize().unwrap(), original);
        assert_eq!(
            std::fs::read_to_string(alias.join("keep.txt")).unwrap(),
            "private"
        );
    }

    #[test]
    fn approved_link_batches_update_modes_and_fail_without_partial_grants() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        let workspace = engine.workspace("assistant").unwrap();
        let mut link = WorkspaceMount {
            path: workspace.join("shared/approved"),
            target: outside.path().canonicalize().unwrap(),
            writable: false,
        };
        engine
            .register_home_links("assistant", "request", &[link.clone()])
            .unwrap();
        assert!(!engine.shared_folders().unwrap()[0].writable);
        link.writable = true;
        engine
            .register_home_links("assistant", "request", &[link.clone()])
            .unwrap();
        assert!(engine.shared_folders().unwrap()[0].writable);
        link.writable = false;
        engine
            .register_home_links("assistant", "request", &[link])
            .unwrap();
        assert!(!engine.shared_folders().unwrap()[0].writable);

        std::fs::write(workspace.join("occupied"), "keep").unwrap();
        let first = WorkspaceMount {
            path: workspace.join("shared/new"),
            target: outside.path().canonicalize().unwrap(),
            writable: false,
        };
        let second = WorkspaceMount {
            path: workspace.join("occupied"),
            target: outside.path().canonicalize().unwrap(),
            writable: false,
        };
        assert!(
            engine
                .register_home_links("assistant", "request", &[first, second])
                .is_err()
        );
        assert_eq!(engine.shared_folders().unwrap().len(), 1);
        assert!(!workspace.join("shared/new").exists());
        assert_eq!(
            std::fs::read_to_string(workspace.join("occupied")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn relocated_solar_storage_still_rejects_the_whole_user_home() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        if let Some(home) = std::env::var_os("HOME") {
            assert!(
                engine
                    .validate_share_source(Path::new(&home).to_str().unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn failed_project_assignment_preserves_the_previous_profile() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path().into()).unwrap();
        crate::test_support::add_assistant(&engine);
        engine
            .save_project(Project {
                id: "colliding".into(),
                name: String::new(),
                directory: outside.path().to_string_lossy().into_owned(),
                description: String::new(),
            })
            .unwrap();
        let location = engine.workspace("assistant").unwrap().join("projects");
        std::fs::create_dir_all(&location).unwrap();
        std::fs::write(location.join("colliding"), "private file").unwrap();
        let mut profile = engine.orbit("assistant").unwrap();
        profile.project_ids.push("colliding".into());
        assert!(engine.save_orbit(profile).is_err());
        assert!(engine.orbit("assistant").unwrap().project_ids.is_empty());
        assert_eq!(
            std::fs::read_to_string(location.join("colliding")).unwrap(),
            "private file"
        );
    }
}
