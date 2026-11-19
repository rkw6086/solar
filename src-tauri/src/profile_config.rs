//! Human-editable configuration for one Solar data root.
//!
//! TOML is authoritative at startup. SQLite stores runtime state and a cache of
//! these documents; settings screens write the TOML before updating that cache.

use crate::model::{McpServer, Orbit, Project, Routine, Settings};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solar_providers::ProviderProfile;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrbitReference {
    id: String,
    directory: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MasterConfig {
    version: u32,
    settings: Settings,
    providers: Vec<ProviderProfile>,
    projects: Vec<Project>,
    mcp: Vec<McpServer>,
    routines: Vec<Routine>,
    orbits: Vec<OrbitReference>,
}

impl Default for MasterConfig {
    fn default() -> Self {
        Self {
            version: 1,
            settings: Settings::default(),
            providers: vec![],
            projects: vec![],
            mcp: vec![],
            routines: vec![],
            orbits: vec![],
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrbitConfig {
    version: u32,
    orbit: Orbit,
}

#[derive(Clone)]
pub(crate) struct ProfileConfig {
    root: PathBuf,
    master: MasterConfig,
    profiles: BTreeMap<String, Orbit>,
    locations: BTreeMap<String, String>,
}

pub(crate) const CONFIG_KINDS: [&str; 6] = [
    "settings",
    "providers",
    "projects",
    "mcp",
    "routines",
    "orbits",
];

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub(crate) fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 180
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        && Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

pub(crate) fn secure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "Solar data directory must be a real directory: {}",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
        }
        Err(error) => return Err(format!("Cannot inspect {}: {error}", path.display())),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn secure_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "Solar data file must be a regular file: {}",
            path.display()
        )),
        Ok(_) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
    }
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    secure_file(path)?;
    let text =
        fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    toml::from_str(&text)
        .map_err(|e| format!("Invalid Solar configuration {}: {e}", path.display()))
}

fn write_toml(path: &Path, value: &impl Serialize) -> Result<(), String> {
    secure_file(path)?;
    if path.try_exists().map_err(|e| e.to_string())? {
        let _: toml::Value = read_toml(path)?;
    }
    let text = toml::to_string_pretty(value)
        .map_err(|e| format!("Cannot encode {}: {e}", path.display()))?;
    let temporary = path.with_file_name(format!(".config-{}.tmp", crate::model::id()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| format!("Cannot save {}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            fs::File::open(path.parent().ok_or("Configuration has no parent")?)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn validate_master(master: &MasterConfig) -> Result<(), String> {
    if master.version != 1 {
        return Err("Unsupported Solar config.toml version".into());
    }
    let settings = &master.settings;
    if !(1..=8).contains(&settings.max_concurrent)
        || !settings.attention_volume.is_finite()
        || !(0.0..=1.0).contains(&settings.attention_volume)
        || !settings.sound_effects_volume.is_finite()
        || !(0.0..=0.5).contains(&settings.sound_effects_volume)
        || ![
            "birth",
            "welcome",
            "team",
            "desktop",
            "connection",
            "orbit",
            "workspace",
            "ready",
        ]
        .contains(&settings.onboarding_step.as_str())
    {
        return Err(
            "Invalid settings in Solar config.toml: check concurrency, volume, and onboarding step"
                .into(),
        );
    }
    let mut mcp_ids = std::collections::HashSet::new();
    for server in &master.mcp {
        if !valid_id(&server.id)
            || server.id.len() > 48
            || !mcp_ids.insert(server.id.to_ascii_lowercase().replace('-', "_"))
        {
            return Err(
                "MCP ids must be unique, case-insensitive tool namespaces of at most 48 characters"
                    .into(),
            );
        }
    }
    Ok(())
}

fn validate_orbit(orbit: &Orbit) -> Result<(), String> {
    if !valid_id(&orbit.id)
        || orbit.name.trim().is_empty()
        || orbit.name.len() > 100
        || orbit.role.trim().is_empty()
        || orbit.role.len() > 300
        || orbit.tasks.len() > 32_000
        || orbit.personality.len() > 8_000
        || orbit.instructions.len() > 32_000
        || !["ask", "workspace", "trusted"].contains(&orbit.autonomy.as_str())
        || orbit.x.is_some_and(|v| !v.is_finite())
        || orbit.y.is_some_and(|v| !v.is_finite())
    {
        return Err(
            "Invalid Sprite configuration: check its name, role, autonomy, and instruction lengths"
                .into(),
        );
    }
    Ok(())
}

impl ProfileConfig {
    pub fn open(root: &Path, mut locations: BTreeMap<String, String>) -> Result<Self, String> {
        secure_directory(root)?;
        secure_directory(&root.join("orbits"))?;
        let path = root.join("config.toml");
        let exists = path.try_exists().map_err(|e| e.to_string())?;
        let master: MasterConfig = if exists {
            read_toml(&path)?
        } else {
            MasterConfig::default()
        };
        validate_master(&master)?;
        let mut profiles = BTreeMap::new();
        let mut seen_directories = BTreeMap::new();
        for reference in &master.orbits {
            if !valid_id(&reference.id) || !safe_component(&reference.directory) {
                return Err("Invalid Sprite id or directory in config.toml".into());
            }
            let folded = reference.directory.to_ascii_lowercase();
            if seen_directories
                .insert(folded, reference.id.clone())
                .is_some()
            {
                return Err("Sprite directories must be unique, including case".into());
            }
            let directory = root.join("orbits").join(&reference.directory);
            secure_directory(&directory)?;
            let config: OrbitConfig = read_toml(&directory.join("config.toml"))?;
            if config.version != 1
                || config.orbit.id != reference.id
                || config.orbit.name.trim().is_empty()
            {
                return Err(format!(
                    "Invalid Sprite configuration: {}",
                    directory.display()
                ));
            }
            validate_orbit(&config.orbit)?;
            if profiles
                .insert(reference.id.clone(), config.orbit)
                .is_some()
            {
                return Err("Duplicate Sprite id in config.toml".into());
            }
            locations.insert(reference.id.clone(), reference.directory.clone());
            Self::ensure_layout(&directory)?;
        }
        let mut all_directories = std::collections::HashSet::new();
        for (id, directory) in &locations {
            if !valid_id(id)
                || !safe_component(directory)
                || !all_directories.insert(directory.to_ascii_lowercase())
            {
                return Err("Invalid or colliding stored Sprite location".into());
            }
        }
        for (kind, values) in [
            (
                "providers",
                master
                    .providers
                    .iter()
                    .map(|v| v.id.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                "projects",
                master.projects.iter().map(|v| v.id.as_str()).collect(),
            ),
            ("mcp", master.mcp.iter().map(|v| v.id.as_str()).collect()),
            (
                "routines",
                master.routines.iter().map(|v| v.id.as_str()).collect(),
            ),
        ] {
            let mut ids = std::collections::HashSet::new();
            if values.iter().any(|id| !valid_id(id) || !ids.insert(*id)) {
                return Err(format!("Invalid or duplicate {kind} id in config.toml"));
            }
        }
        if !exists {
            write_toml(&path, &master)?;
        }
        Ok(Self {
            root: root.into(),
            master,
            profiles,
            locations,
        })
    }

    fn ensure_layout(directory: &Path) -> Result<(), String> {
        for child in ["workspace", "skills", "memory", "browser", "cache"] {
            secure_directory(&directory.join(child))?;
        }
        Ok(())
    }

    pub fn locations(&self) -> &BTreeMap<String, String> {
        &self.locations
    }

    pub fn orbit_dir(&self, id: &str) -> Result<PathBuf, String> {
        let directory = self
            .locations
            .get(id)
            .ok_or("Sprite data directory is unavailable")?;
        let path = self.root.join("orbits").join(directory);
        secure_directory(&path)?;
        Ok(path)
    }

    pub fn documents(&self) -> Result<Vec<(&'static str, String, Value)>, String> {
        let mut docs = vec![(
            "settings",
            "app".into(),
            serde_json::to_value(&self.master.settings).map_err(|e| e.to_string())?,
        )];
        macro_rules! collect {
            ($kind:literal, $values:expr) => {
                for value in $values {
                    docs.push((
                        $kind,
                        value.id.clone(),
                        serde_json::to_value(value).map_err(|e| e.to_string())?,
                    ));
                }
            };
        }
        collect!("providers", &self.master.providers);
        collect!("projects", &self.master.projects);
        collect!("mcp", &self.master.mcp);
        collect!("routines", &self.master.routines);
        collect!("orbits", self.profiles.values());
        Ok(docs)
    }

    fn directory_for_new(&self, orbit: &Orbit) -> Result<String, String> {
        if let Some(existing) = self.locations.get(&orbit.id) {
            return Ok(existing.clone());
        }
        let mut base = String::new();
        for c in orbit.name.chars() {
            if c.is_ascii_alphanumeric() {
                base.push(c.to_ascii_lowercase());
            } else if !base.ends_with('-') {
                base.push('-');
            }
            if base.len() >= 60 {
                break;
            }
        }
        let base = base.trim_matches('-');
        let base = if base.is_empty() { "orbit" } else { base };
        let occupied = fs::read_dir(self.root.join("orbits"))
            .map_err(|e| e.to_string())?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if !occupied.iter().any(|name| name == base) {
            return Ok(base.into());
        }
        if let Ok(config) =
            read_toml::<OrbitConfig>(&self.root.join("orbits").join(base).join("config.toml"))
            && config.orbit.id == orbit.id
        {
            return Ok(base.into());
        }
        let suffix = orbit
            .id
            .chars()
            .take(8)
            .collect::<String>()
            .to_ascii_lowercase();
        for counter in 0..1000 {
            let candidate = if counter == 0 {
                format!("{base}-{suffix}")
            } else {
                format!("{base}-{suffix}-{counter}")
            };
            if !occupied.iter().any(|name| name == &candidate) {
                return Ok(candidate);
            }
            // Recover a previous config write whose root-config update failed.
            let path = self
                .root
                .join("orbits")
                .join(&candidate)
                .join("config.toml");
            if let Ok(config) = read_toml::<OrbitConfig>(&path)
                && config.orbit.id == orbit.id
            {
                return Ok(candidate);
            }
        }
        Err("Cannot allocate a Sprite data directory".into())
    }

    fn check_external_edits(&self, orbit_id: Option<&str>) -> Result<(), String> {
        let disk: MasterConfig = read_toml(&self.root.join("config.toml"))?;
        validate_master(&disk)?;
        if serde_json::to_value(&disk).map_err(|e| e.to_string())?
            != serde_json::to_value(&self.master).map_err(|e| e.to_string())?
        {
            return Err("config.toml changed outside Solar. Restart Solar to reload it before saving settings.".into());
        }
        if let Some(id) = orbit_id
            && let Some(previous) = self.profiles.get(id)
        {
            let disk: OrbitConfig = read_toml(&self.orbit_dir(id)?.join("config.toml"))?;
            validate_orbit(&disk.orbit)?;
            if disk.version != 1
                || serde_json::to_value(&disk.orbit).map_err(|e| e.to_string())?
                    != serde_json::to_value(previous).map_err(|e| e.to_string())?
            {
                return Err("This Sprite config.toml changed outside Solar. Restart Solar to reload it before saving.".into());
            }
        }
        Ok(())
    }

    /// Publish one configuration edit atomically. Folder names stay stable on
    /// display-name edits so active SQLite/browser/workspace handles remain valid.
    pub fn put(&mut self, kind: &str, id: &str, value: Value) -> Result<(), String> {
        self.check_external_edits(if kind == "orbits" { Some(id) } else { None })?;
        let mut updated = self.clone();
        macro_rules! replace {
            ($field:ident, $ty:ty) => {{
                let item: $ty = serde_json::from_value(value).map_err(|e| e.to_string())?;
                if item.id != id || !valid_id(id) {
                    return Err("Configuration id mismatch".into());
                }
                updated.master.$field.retain(|item| item.id != id);
                updated.master.$field.push(item);
            }};
        }
        match kind {
            "settings" => {
                if id != "app" {
                    return Err("Invalid master settings id".into());
                }
                updated.master.settings =
                    serde_json::from_value(value).map_err(|e| e.to_string())?;
            }
            "providers" => replace!(providers, ProviderProfile),
            "projects" => replace!(projects, Project),
            "mcp" => replace!(mcp, McpServer),
            "routines" => replace!(routines, Routine),
            "orbits" => {
                let orbit: Orbit = serde_json::from_value(value).map_err(|e| e.to_string())?;
                validate_orbit(&orbit)?;
                if orbit.id != id {
                    return Err("Sprite configuration id mismatch".into());
                }
                let directory = updated.directory_for_new(&orbit)?;
                let path = updated.root.join("orbits").join(&directory);
                secure_directory(&path)?;
                Self::ensure_layout(&path)?;
                write_toml(
                    &path.join("config.toml"),
                    &OrbitConfig {
                        version: 1,
                        orbit: orbit.clone(),
                    },
                )?;
                updated.locations.insert(id.into(), directory.clone());
                updated.master.orbits.retain(|entry| entry.id != id);
                updated.master.orbits.push(OrbitReference {
                    id: id.into(),
                    directory,
                });
                updated.profiles.insert(id.into(), orbit);
            }
            _ => return Err("Not a Solar configuration document".into()),
        }
        validate_master(&updated.master)?;
        write_toml(&updated.root.join("config.toml"), &updated.master)?;
        *self = updated;
        Ok(())
    }

    pub fn remove(&mut self, kind: &str, id: &str) -> Result<(), String> {
        self.check_external_edits(None)?;
        let mut updated = self.clone();
        match kind {
            "providers" => updated.master.providers.retain(|item| item.id != id),
            "projects" => updated.master.projects.retain(|item| item.id != id),
            "mcp" => updated.master.mcp.retain(|item| item.id != id),
            "routines" => updated.master.routines.retain(|item| item.id != id),
            "orbits" => {
                updated.master.orbits.retain(|item| item.id != id);
                updated.profiles.remove(id);
            }
            _ => return Err("Cannot remove this configuration document".into()),
        }
        validate_master(&updated.master)?;
        write_toml(&updated.root.join("config.toml"), &updated.master)?;
        *self = updated;
        Ok(())
    }
}
