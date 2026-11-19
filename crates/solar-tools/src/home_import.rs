//! Explicit, reviewable copying from the user's home into an Orbit's private home.
//! Planning never runs profile code. Only the exact buffered files shown at approval
//! can be written; files changing at the destination invalidate that approval.
use crate::{CancellationToken, ToolDefinition, definition as tool_definition};
use cap_std::fs::{Dir, OpenOptions};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_FILES: usize = 256;
const MAX_FILE: usize = 5 * 1024 * 1024;
const MAX_TOTAL: usize = 20 * 1024 * 1024;

pub(crate) fn definition() -> ToolDefinition {
    tool_definition(
        "pull_from_user_home",
        "Request files or folders from the user's home, copied into this Orbit's private home/configuration. Always requires a separate user approval showing every file and replacement, even for trusted Orbits. Discovers literal shell source dependencies and home-contained symlinks without running code; rewrites home paths to the private copies. Profile files can contain credentials and executable startup code. Use only files needed for the user's task; never request the whole home. Dynamic includes may need a follow-up request.",
        json!({"paths":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":20},"includeDependencies":{"type":"boolean","default":true}}),
        &["paths"],
    )
}

struct PlannedFile {
    source: String,
    source_path: PathBuf,
    target: PathBuf,
    data: Vec<u8>,
    previous: Option<Vec<u8>>,
    executable: bool,
    dependency: bool,
}

pub struct HomeImportPlan {
    home: PathBuf,
    workspace: PathBuf,
    storage: PathBuf,
    files: Vec<PlannedFile>,
    warnings: BTreeSet<String>,
    total: usize,
    visited: usize,
    denied: Vec<PathBuf>,
    include_dependencies: bool,
}

impl HomeImportPlan {
    pub fn prepare(
        home: &Path,
        workspace: &Path,
        storage: &Path,
        denied: &[PathBuf],
        args: &Value,
    ) -> Result<Self, String> {
        let paths = args
            .get("paths")
            .and_then(Value::as_array)
            .filter(|v| !v.is_empty() && v.len() <= 20)
            .ok_or("Choose between 1 and 20 home files or folders")?;
        let mut plan = Self {
            home: home
                .canonicalize()
                .map_err(|_| "The user's home is unavailable")?,
            workspace: workspace
                .canonicalize()
                .map_err(|_| "The Orbit's private home is unavailable")?,
            storage: storage
                .canonicalize()
                .map_err(|_| "The Orbit's storage is unavailable")?,
            files: Vec::new(),
            warnings: BTreeSet::new(),
            total: 0,
            visited: 0,
            denied: denied
                .iter()
                .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
                .collect(),
            include_dependencies: args
                .get("includeDependencies")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        };
        if !plan.workspace.starts_with(&plan.storage) {
            return Err("Orbit home must be inside its private storage".into());
        }
        for path in paths {
            let requested = path.as_str().ok_or("Every home path must be text")?;
            let source = plan.expand(requested, &plan.home)?;
            if source == plan.home {
                return Err("Select specific files or folders, not the entire home".into());
            }
            plan.collect(&source, false, &mut BTreeSet::new(), 0)?;
        }
        if plan.files.is_empty() {
            return Err("No regular files were found to import".into());
        }
        plan.files.sort_by(|a, b| a.target.cmp(&b.target));
        Ok(plan)
    }

    pub fn validate_scope(&self, denied: &[PathBuf]) -> Result<(), String> {
        for denied in denied {
            let root = denied.canonicalize().unwrap_or_else(|_| denied.clone());
            if self
                .files
                .iter()
                .any(|file| file.source_path.starts_with(&root))
            {
                return Err(
                    "Project access changed while approval was open. Request the import again."
                        .into(),
                );
            }
        }
        Ok(())
    }

    pub fn review(&self) -> Value {
        json!({"files":self.files.iter().map(|file|json!({"source":file.source,"destination":file.target.to_string_lossy(),"bytes":file.data.len(),"replace":file.previous.is_some(),"dependency":file.dependency})).collect::<Vec<_>>(),"warnings":self.warnings,"totalBytes":self.total,"notice":"These files may contain credentials or executable startup instructions. Approve only files you intend this Orbit to use. Copies remain private to this Orbit; source files are not changed."})
    }

    fn expand(&self, input: &str, base: &Path) -> Result<PathBuf, String> {
        let input = input.trim();
        let expanded = if input == "~" || input == "$HOME" || input == "${HOME}" {
            self.home.clone()
        } else if let Some(tail) = input
            .strip_prefix("~/")
            .or_else(|| input.strip_prefix("$HOME/"))
            .or_else(|| input.strip_prefix("${HOME}/"))
        {
            self.home.join(tail)
        } else if Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else if input.contains('$') || input.contains('`') || input.contains('\0') {
            return Err("Dynamic paths must be requested explicitly after resolving them".into());
        } else {
            base.join(input)
        };
        let normalized = normalize(&expanded)?;
        if !normalized.starts_with(&self.home) {
            return Err("Home imports cannot read outside the user's home".into());
        }
        if normalized.starts_with(self.home.join(".solar")) || normalized.starts_with(&self.storage)
        {
            return Err("Solar's private storage cannot be imported into another Orbit".into());
        }
        Ok(normalized)
    }

    fn target(&self, source: &Path) -> Result<PathBuf, String> {
        let relative = source
            .strip_prefix(&self.home)
            .map_err(|_| "Home source is outside the user's home")?;
        let aliases = [
            (".config", "config"),
            (".local/share", "data"),
            (".local/state", "state"),
            (".cache", "cache"),
        ];
        if relative == Path::new(".gitconfig") {
            return Ok(self.storage.join("config/git/config"));
        }
        for (prefix, mapped) in aliases {
            if let Ok(tail) = relative.strip_prefix(prefix) {
                return Ok(self.storage.join(mapped).join(tail));
            }
        }
        Ok(self.workspace.join(relative))
    }

    fn collect(
        &mut self,
        source: &Path,
        dependency: bool,
        visiting: &mut BTreeSet<PathBuf>,
        depth: usize,
    ) -> Result<(), String> {
        self.visited += 1;
        if self.visited > 1024 {
            return Err(
                "Select a smaller folder (at most 1024 files and folders inspected)".into(),
            );
        }
        if depth > 24 {
            return Err("Home import dependencies are too deeply nested".into());
        }
        let actual = source
            .canonicalize()
            .map_err(|_| format!("Cannot read {}", self.display(source)))?;
        self.expand(
            actual.to_str().ok_or("Home paths must be valid text")?,
            &self.home,
        )?;
        if self.denied.iter().any(|root| actual.starts_with(root)) {
            return Err("A requested file is in a project this Orbit cannot access".into());
        }
        if !visiting.insert(actual.clone()) {
            self.warnings.insert(format!(
                "Skipped a circular link at {}",
                self.display(source)
            ));
            return Ok(());
        }
        let result = self.collect_inner(source, &actual, dependency, visiting, depth);
        visiting.remove(&actual);
        result
    }

    fn collect_inner(
        &mut self,
        source: &Path,
        actual: &Path,
        dependency: bool,
        visiting: &mut BTreeSet<PathBuf>,
        depth: usize,
    ) -> Result<(), String> {
        let metadata = std::fs::metadata(actual).map_err(|_| "Cannot inspect a home file")?;
        if metadata.is_dir() {
            let mut entries = std::fs::read_dir(actual)
                .map_err(|_| "Cannot read a home folder")?
                .take(MAX_FILES + 1)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Cannot read a folder entry")?;
            entries.sort_by_key(|entry| entry.file_name());
            if entries.len() > MAX_FILES {
                return Err("Select a smaller folder (at most 256 files per import)".into());
            }
            for entry in entries {
                self.collect(
                    &source.join(entry.file_name()),
                    dependency,
                    visiting,
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        if !metadata.is_file() {
            return Err("Only regular files and folders can be imported".into());
        }
        let target = self.target(source)?;
        if self.files.iter().any(|f| f.target == target) {
            return Ok(());
        }
        if self.files.len() >= MAX_FILES {
            return Err("Select fewer files (at most 256 per import)".into());
        }
        if metadata.len() > MAX_FILE as u64 {
            return Err(format!(
                "{} exceeds the 5 MiB file limit",
                self.display(source)
            ));
        }
        // Capability-relative reads reject a swapped link escaping the home.
        let home_dir = Dir::open_ambient_dir(&self.home, cap_std::ambient_authority())
            .map_err(|_| "Cannot open the user's home")?;
        let mut data = Vec::new();
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        let file = home_dir
            .open_with(
                actual
                    .strip_prefix(&self.home)
                    .map_err(|_| "Invalid home source")?,
                &options,
            )
            .map_err(|_| "Cannot open a home file")?;
        if !file
            .metadata()
            .map_err(|_| "Cannot inspect opened home file")?
            .is_file()
        {
            return Err("Only regular files can be imported".into());
        }
        file.take((MAX_FILE + 1) as u64)
            .read_to_end(&mut data)
            .map_err(|_| "Cannot read a home file")?;
        if data.len() > MAX_FILE {
            return Err("A home file grew beyond the import limit".into());
        }
        let text = std::str::from_utf8(&data).ok().map(str::to_owned);
        if let Some(text) = &text {
            data = self.rewrite(text).into_bytes();
        }
        if data.len() > MAX_FILE {
            return Err("A rewritten profile exceeds the 5 MiB file limit".into());
        }
        self.total += data.len();
        if self.total > MAX_TOTAL {
            return Err("Select fewer files (20 MiB total per import)".into());
        }
        let previous = read_target(&self.storage, &target)?;
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        self.files.push(PlannedFile {
            source: self.display(source),
            source_path: actual.to_owned(),
            target,
            data,
            previous,
            executable,
            dependency,
        });
        if self.include_dependencies
            && let Some(text) = text
        {
            self.dependencies(&text, source, visiting, depth)?;
        }
        Ok(())
    }

    fn display(&self, path: &Path) -> String {
        format!(
            "~/{}",
            path.strip_prefix(&self.home)
                .unwrap_or(path)
                .to_string_lossy()
        )
    }

    fn rewrite(&self, text: &str) -> String {
        let mut prefixes = Vec::new();
        for (prefix, mapped) in [
            (".config", "config"),
            (".local/share", "data"),
            (".local/state", "state"),
            (".cache", "cache"),
            (".gitconfig", "config/git/config"),
        ] {
            let destination = self.storage.join(mapped).to_string_lossy().into_owned();
            for from in [
                format!("{}/{prefix}", self.home.display()),
                format!("$HOME/{prefix}"),
                format!("${{HOME}}/{prefix}"),
                format!("~/{prefix}"),
            ] {
                prefixes.push((from, destination.clone()));
            }
        }
        prefixes.push((
            self.home.to_string_lossy().into_owned(),
            self.workspace.to_string_lossy().into_owned(),
        ));
        rewrite_path_prefixes(text, &prefixes)
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
        // A deliberately small lexer: no evaluation, process substitution, globbing,
        // commands, or network access. Unresolvable source directives are reported.
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
                    self.warnings.insert(format!("{} contains a dynamic source directive; request its resolved file separately",self.display(source)));
                    continue;
                };
                if raw.contains(['*', '?', '`', '(', ')']) {
                    self.warnings.insert(format!("{} contains a dynamic source directive; request its resolved file separately",self.display(source)));
                    continue;
                }
                let path = match self.expand(&raw, source.parent().unwrap_or(&self.home)) {
                    Ok(path) => path,
                    Err(_) => {
                        self.warnings.insert(format!(
                            "{} references a source outside the home; it was not imported",
                            self.display(source)
                        ));
                        continue;
                    }
                };
                if !path.exists() {
                    self.warnings.insert(format!(
                        "Referenced file {} was not found",
                        self.display(&path)
                    ));
                    continue;
                }
                self.collect(&path, true, visiting, depth + 1)?;
            }
        }
        Ok(())
    }

    pub fn apply(self, cancellation: &CancellationToken) -> Result<Value, String> {
        let root = Dir::open_ambient_dir(&self.storage, cap_std::ambient_authority())
            .map_err(|_| "Orbit storage is unavailable")?;
        // Validate every replacement before changing any file.
        for file in &self.files {
            if cancellation.is_cancelled() {
                return Err("Import cancelled".into());
            }
            if read_target(&self.storage, &file.target)? != file.previous {
                return Err(
                    "An Orbit file changed while approval was open. Request the import again."
                        .into(),
                );
            }
        }
        let mut copied = Vec::new();
        for file in self.files {
            if cancellation.is_cancelled() {
                return Err(
                    "Import cancelled; earlier completed copies remain in the Orbit".into(),
                );
            }
            let relative = file
                .target
                .strip_prefix(&self.storage)
                .map_err(|_| "Invalid Orbit destination")?;
            let parent = relative.parent().ok_or("Invalid Orbit destination")?;
            ensure_parents(&root, parent)?;
            let directory = root
                .open_dir(parent)
                .map_err(|_| "Cannot open Orbit destination")?;
            let temporary = format!(".solar-import-{}", uuid::Uuid::new_v4());
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            let result =
                (|| {
                    let mut output = directory
                        .open_with(&temporary, &options)
                        .map_err(|_| "Cannot create a private copy")?;
                    #[cfg(unix)]
                    {
                        use cap_std::fs::PermissionsExt;
                        output
                            .set_permissions(cap_std::fs::Permissions::from_mode(
                                if file.executable { 0o700 } else { 0o600 },
                            ))
                            .map_err(|_| "Cannot protect imported file")?;
                    }
                    output
                        .write_all(&file.data)
                        .map_err(|_| "Cannot write imported file")?;
                    output.sync_all().map_err(|_| "Cannot save imported file")?;
                    if cancellation.is_cancelled() {
                        return Err("Import cancelled".into());
                    }
                    if read_target(&self.storage, &file.target)? != file.previous {
                        return Err(
                            "An Orbit file changed during import. Request the import again.".into(),
                        );
                    }
                    directory
                        .rename(
                            &temporary,
                            &directory,
                            relative.file_name().ok_or("Missing destination filename")?,
                        )
                        .map_err(|_| "Cannot finish imported file")?;
                    Ok::<_, String>(())
                })();
            if result.is_err() {
                let _ = directory.remove_file(&temporary);
            }
            result?;
            copied.push(json!({"source":file.source,"destination":file.target.to_string_lossy()}));
        }
        Ok(
            json!({"copied":copied,"warnings":self.warnings,"message":"Approved files copied into this Orbit. No source files changed. Startup files take effect on the next shell command. Imported startup code is now executable by the Orbit."}),
        )
    }
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
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

fn ensure_parents(root: &Dir, path: &Path) -> Result<(), String> {
    let mut child = PathBuf::new();
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("Invalid private destination".into());
        }
        child.push(component);
        match root.symlink_metadata(&child) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err("A destination folder is a symbolic link or file".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                root.create_dir(&child)
                    .map_err(|_| "Cannot create private destination")?;
                #[cfg(unix)]
                {
                    use cap_std::fs::PermissionsExt;
                    root.set_permissions(&child, cap_std::fs::Permissions::from_mode(0o700))
                        .map_err(|_| "Cannot protect private folder")?;
                }
            }
            Err(_) => return Err("Cannot inspect private destination".into()),
        }
    }
    Ok(())
}

fn read_target(storage: &Path, target: &Path) -> Result<Option<Vec<u8>>, String> {
    let root = Dir::open_ambient_dir(storage, cap_std::ambient_authority())
        .map_err(|_| "Cannot open Orbit storage")?;
    let relative = target
        .strip_prefix(storage)
        .map_err(|_| "Destination is outside Orbit storage")?;
    let mut current = PathBuf::new();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("Invalid destination".into());
        }
        current.push(component);
        match root.symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Import destinations cannot contain symbolic links".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Cannot inspect Orbit destination".into()),
        }
    }
    let mut data = Vec::new();
    let file = root
        .open(relative)
        .map_err(|_| "Cannot read existing Orbit file")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect existing Orbit file")?
        .is_file()
    {
        return Err("Import would replace a folder".into());
    }
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| "Cannot read existing Orbit file")?;
    if data.len() > MAX_FILE {
        return Err("Existing Orbit file is too large to replace safely".into());
    }
    Ok(Some(data))
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

// Match path components instead of substrings; scan the original only, so a
// replacement below the real home is never rewritten again. Preserve quoted
// literals and escape replacement paths for the surrounding shell quote style.
fn rewrite_path_prefixes(text: &str, prefixes: &[(String, String)]) -> String {
    let mut prefixes = prefixes.iter().collect::<Vec<_>>();
    prefixes.sort_by_key(|(source, _)| std::cmp::Reverse(source.len()));
    let mut result = String::with_capacity(text.len());
    let mut quote = None;
    let mut offset = 0;
    while offset < text.len() {
        let tail = &text[offset..];
        let previous = text[..offset].chars().next_back();
        let starts_word = previous.is_none_or(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    '\'' | '"' | '=' | '(' | ')' | '[' | ']' | ';' | ':' | ','
                )
        });
        let replacement = starts_word
            .then(|| {
                prefixes.iter().find(|(source, _)| {
                    if !tail.starts_with(source.as_str()) {
                        return false;
                    }
                    if source.starts_with('$') && quote == Some('\'') {
                        return false;
                    }
                    if source.starts_with('~') && quote.is_some() {
                        return false;
                    }
                    tail[source.len()..].chars().next().is_none_or(|c| {
                        c == '/'
                            || c.is_whitespace()
                            || matches!(c, '\'' | '"' | ';' | ')' | ']' | ',')
                    })
                })
            })
            .flatten();
        if let Some((source, destination)) = replacement {
            result.push_str(&quote_replacement(destination, quote));
            offset += source.len();
            continue;
        }
        let character = tail.chars().next().expect("nonempty tail");
        result.push(character);
        offset += character.len_utf8();
        if character == '\\' && quote != Some('\'') {
            if let Some(next) = text[offset..].chars().next() {
                result.push(next);
                offset += next.len_utf8();
            }
            continue;
        }
        if character == '\'' || character == '"' {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        }
    }
    result
}

fn quote_replacement(value: &str, quote: Option<char>) -> String {
    if quote == Some('\'') {
        return value.replace('\'', "'\\''");
    }
    let escaped = value
        .chars()
        .flat_map(|c| {
            if matches!(c, '\\' | '"' | '$' | '`') {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect::<String>();
    if quote == Some('"') {
        escaped
    } else if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
    {
        value.to_owned()
    } else {
        format!("\"{escaped}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variable_expansion_respects_complete_names_and_rejects_execution() {
        let variables = BTreeMap::from([
            ("HOME".into(), "/home/user".into()),
            ("HOME_DIR".into(), "/home/user/profiles".into()),
            ("ZSH".into(), "/home/user/zsh".into()),
            ("ZSH_CUSTOM".into(), "/home/user/custom".into()),
        ]);
        assert_eq!(
            expand_variables("$HOME_DIR/secrets", &variables).as_deref(),
            Some("/home/user/profiles/secrets")
        );
        assert_eq!(
            expand_variables("${ZSH_CUSTOM}/private.zsh", &variables).as_deref(),
            Some("/home/user/custom/private.zsh")
        );
        for dynamic in [
            "$HOME_SUFFIX/file",
            "$(touch unwanted)",
            "`pwd`/file",
            "${HOME:-/tmp}/file",
            "${HOME",
            "$$/file",
        ] {
            assert!(expand_variables(dynamic, &variables).is_none(), "{dynamic}");
        }
    }

    #[test]
    fn rewriting_matches_path_boundaries_and_preserves_literal_variables() {
        let prefixes = vec![
            ("$HOME/.config".into(), "/private/config".into()),
            ("/home/user".into(), "/private/home".into()),
        ];
        let text = "source $HOME/.config/profile\nsource $HOME/.config-backup/profile\nsource /home/user/profile\nsource /home/user-two/profile\nsource '$HOME/.config/profile'\nsource \\$HOME/.config/profile\n";
        assert_eq!(
            rewrite_path_prefixes(text, &prefixes),
            "source /private/config/profile\nsource $HOME/.config-backup/profile\nsource /private/home/profile\nsource /home/user-two/profile\nsource '$HOME/.config/profile'\nsource \\$HOME/.config/profile\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rewritten_paths_keep_spaces_quotes_and_dollars_literal() {
        let target = "/tmp/Orbit's home/$cache";
        let prefixes = vec![("/home/user".into(), target.into())];
        for argument in [
            "/home/user/file",
            "\"/home/user/file\"",
            "'/home/user/file'",
        ] {
            let command = rewrite_path_prefixes(&format!("printf '%s' {argument}"), &prefixes);
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &command])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("{target}/file"),
                "{command}"
            );
        }
    }

    #[test]
    fn longer_profile_variables_find_their_own_dependencies() {
        let (_temporary, home, store, workspace) = fixture();
        std::fs::create_dir_all(home.join("profiles")).unwrap();
        std::fs::create_dir_all(home.join("custom")).unwrap();
        std::fs::write(home.join("profiles/secrets"), "export FIRST=1").unwrap();
        std::fs::write(home.join("custom/private.zsh"), "export SECOND=1").unwrap();
        std::fs::write(home.join(".zshrc"), "HOME_DIR=\"$HOME/profiles\"\nZSH=\"$HOME/zsh\"\nZSH_CUSTOM=\"$HOME/custom\"\nsource \"$HOME_DIR/secrets\"\nsource \"${ZSH_CUSTOM}/private.zsh\"\n").unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &store,
            &[],
            &json!({"paths":["~/.zshrc"]}),
        )
        .unwrap();
        assert_eq!(plan.review()["files"].as_array().unwrap().len(), 3);
        assert!(plan.review()["warnings"].as_array().unwrap().is_empty());
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("user");
        let store = home.join(".solar/orbits/one");
        let workspace = store.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        (tmp, home, store, workspace)
    }
    #[test]
    fn dependencies_private_config_and_replacements_are_reviewed_without_contents() {
        let (_tmp, home, store, workspace) = fixture();
        std::fs::create_dir_all(home.join(".config/demo")).unwrap();
        std::fs::write(
            home.join(".zshrc"),
            format!(
                "source \"$HOME/.zshrc.secrets\"\nsource \"{}/.config/demo/profile\"\n",
                home.display()
            ),
        )
        .unwrap();
        std::fs::write(home.join(".zshrc.secrets"), "export TOKEN=very-secret").unwrap();
        std::fs::write(home.join(".config/demo/profile"), "export DEMO=1").unwrap();
        std::fs::write(workspace.join(".zshrc"), "stub").unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &store,
            &[],
            &json!({"paths":["~/.zshrc"]}),
        )
        .unwrap();
        let review = plan.review();
        assert_eq!(review["files"].as_array().unwrap().len(), 3);
        assert!(!review.to_string().contains("very-secret"));
        assert!(review.to_string().contains("\"replace\":true"));
        plan.apply(&CancellationToken::new()).unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join(".zshrc.secrets")).unwrap(),
            "export TOKEN=very-secret"
        );
        assert!(
            std::fs::read_to_string(workspace.join(".zshrc"))
                .unwrap()
                .contains(
                    &store
                        .join("config/demo/profile")
                        .to_string_lossy()
                        .to_string()
                )
        );
        assert!(store.join("config/demo/profile").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(workspace.join(".zshrc.secrets"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(store.join("config/demo"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
    #[test]
    fn rejects_home_root_other_orbits_denied_projects_and_destination_changes() {
        let (_tmp, home, store, workspace) = fixture();
        std::fs::write(home.join("profile"), "source ~/.solar/orbits/two/token").unwrap();
        for path in ["~", "~/../outside", "~/.solar/orbits/one/workspace"] {
            assert!(
                HomeImportPlan::prepare(&home, &workspace, &store, &[], &json!({"paths":[path]}))
                    .is_err()
            );
        }
        std::fs::create_dir_all(home.join("project")).unwrap();
        std::fs::write(home.join("project/key"), "no").unwrap();
        assert!(
            HomeImportPlan::prepare(
                &home,
                &workspace,
                &store,
                &[home.join("project")],
                &json!({"paths":["~/project/key"]})
            )
            .is_err()
        );
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &store,
            &[],
            &json!({"paths":["~/profile"]}),
        )
        .unwrap();
        std::fs::write(workspace.join("profile"), "changed").unwrap();
        assert!(plan.apply(&CancellationToken::new()).is_err());
        assert_eq!(
            std::fs::read_to_string(workspace.join("profile")).unwrap(),
            "changed"
        );
    }
    #[test]
    fn approval_cannot_survive_project_access_revocation() {
        let (_tmp, home, store, workspace) = fixture();
        let project = home.join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("profile"), "private project value").unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &store,
            &[],
            &json!({"paths":["~/project/profile"]}),
        )
        .unwrap();
        assert!(plan.validate_scope(&[]).is_ok());
        assert!(plan.validate_scope(&[project]).is_err());
        assert!(!workspace.join("project/profile").exists());
    }

    #[test]
    fn dynamic_sources_are_not_executed_and_cancellation_writes_nothing() {
        let (_tmp, home, store, workspace) = fixture();
        std::fs::write(
            home.join(".zshrc"),
            "source $(touch evil)\nsource $UNKNOWN/file\n",
        )
        .unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &store,
            &[],
            &json!({"paths":["~/.zshrc"]}),
        )
        .unwrap();
        assert!(!plan.review()["warnings"].as_array().unwrap().is_empty());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(plan.apply(&cancel).is_err());
        assert!(!workspace.join(".zshrc").exists());
        assert!(!home.join("evil").exists());
    }
    #[cfg(unix)]
    #[test]
    fn home_symlinks_are_copied_as_files_external_links_and_target_links_rejected() {
        let (_tmp, home, store, workspace) = fixture();
        std::fs::write(home.join("actual"), "copy").unwrap();
        std::os::unix::fs::symlink(home.join("actual"), home.join("alias")).unwrap();
        let plan = HomeImportPlan::prepare(
            &home,
            &workspace,
            &store,
            &[],
            &json!({"paths":["~/alias"]}),
        )
        .unwrap();
        plan.apply(&CancellationToken::new()).unwrap();
        assert!(
            !std::fs::symlink_metadata(workspace.join("alias"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::os::unix::fs::symlink("/etc/passwd", home.join("bad")).unwrap();
        assert!(
            HomeImportPlan::prepare(&home, &workspace, &store, &[], &json!({"paths":["~/bad"]}))
                .is_err()
        );
        std::os::unix::fs::symlink(home.join("actual"), workspace.join("target")).unwrap();
        std::fs::write(home.join("target"), "new").unwrap();
        assert!(
            HomeImportPlan::prepare(
                &home,
                &workspace,
                &store,
                &[],
                &json!({"paths":["~/target"]})
            )
            .is_err()
        );
    }
}
