use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::plan::Traversal;

#[derive(Debug, Clone)]
pub(crate) struct Repository {
    root: PathBuf,
}

#[derive(Debug)]
pub(crate) enum SourceError {
    NotFound,
    PermissionDenied,
    Io(String),
    Outside,
    SymlinkEscape,
    Binary,
    UnsupportedEncoding,
}

impl Repository {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn resolve(&self, relative: &str) -> Result<PathBuf, SourceError> {
        let lexical = lexical_relative(relative)?;
        let joined = self.root.join(lexical);
        match joined.canonicalize() {
            Ok(resolved) => {
                if !resolved.starts_with(&self.root) {
                    return Err(SourceError::SymlinkEscape);
                }
                Ok(resolved)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(SourceError::NotFound)
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(SourceError::PermissionDenied)
            }
            Err(error) => Err(SourceError::Io(error.to_string())),
        }
    }

    pub(crate) fn read_text(&self, relative: &str) -> Result<String, SourceError> {
        let path = self.resolve(relative)?;
        let bytes = fs::read(path).map_err(map_io)?;
        decode_text(&bytes)
    }

    pub(crate) fn discover(
        &self,
        roots: &[String],
        extensions: &[String],
        excludes: &[String],
        traversal: &Traversal,
    ) -> Result<Vec<String>, SourceError> {
        let root_paths = roots
            .iter()
            .map(|path| lexical_relative(path).map(|p| self.root.join(p)))
            .collect::<Result<Vec<_>, _>>()?;
        for root in &root_paths {
            if !root.exists() {
                return Err(SourceError::NotFound);
            }
            let resolved = root.canonicalize().map_err(map_io)?;
            if !resolved.starts_with(&self.root) {
                return Err(SourceError::SymlinkEscape);
            }
        }
        let matcher = build_excludes(&self.root, excludes)?;
        let extension_set = extensions
            .iter()
            .map(|e| e.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();

        let mut walk = WalkBuilder::new(&self.root);
        walk.standard_filters(false)
            .hidden(false)
            .ignore(false)
            .git_ignore(traversal.respect_gitignore)
            .git_global(false)
            .git_exclude(false)
            .require_git(false)
            .parents(false)
            .follow_links(traversal.follow_symlinks);
        let mut files = BTreeSet::new();
        for entry in walk.build() {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            if !root_paths.iter().any(|root| path.starts_with(root)) {
                continue;
            }
            if !traversal.include_hidden && is_hidden_below_explicit_root(path, &root_paths) {
                continue;
            }
            if entry
                .file_type()
                .is_some_and(|file_type| file_type.is_symlink())
                && !traversal.follow_symlinks
            {
                continue;
            }
            let relative = path
                .strip_prefix(&self.root)
                .map_err(|_| SourceError::Outside)?;
            if matcher
                .matched_path_or_any_parents(relative, false)
                .is_ignore()
            {
                continue;
            }
            if !extension_set.is_empty() {
                let extension = path
                    .extension()
                    .map(|v| format!(".{}", v.to_string_lossy().to_ascii_lowercase()));
                if !extension.is_some_and(|e| extension_set.contains(&e)) {
                    continue;
                }
            }
            let resolved = path.canonicalize().map_err(map_io)?;
            if !resolved.starts_with(&self.root) {
                return Err(SourceError::SymlinkEscape);
            }
            files.insert(to_slash(relative));
        }

        // Explicitly named files are deliberate and may themselves be hidden.
        for root in &root_paths {
            if root.is_file() {
                if fs::symlink_metadata(root)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                    && !traversal.follow_symlinks
                {
                    continue;
                }
                let relative = root
                    .strip_prefix(&self.root)
                    .map_err(|_| SourceError::Outside)?;
                let rel = to_slash(relative);
                if matcher
                    .matched_path_or_any_parents(relative, false)
                    .is_ignore()
                {
                    continue;
                }
                if extension_set.is_empty()
                    || root
                        .extension()
                        .map(|v| format!(".{}", v.to_string_lossy().to_ascii_lowercase()))
                        .is_some_and(|e| extension_set.contains(&e))
                {
                    files.insert(rel);
                }
            }
        }
        Ok(files.into_iter().collect())
    }
}

fn is_hidden_below_explicit_root(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .all(|root| {
            path.strip_prefix(root)
                .is_ok_and(|suffix| suffix_contains_hidden(root, suffix))
        })
}

fn suffix_contains_hidden(root: &Path, suffix: &Path) -> bool {
    let mut current = root.to_path_buf();
    for component in suffix.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        current.push(name);
        if name.to_string_lossy().starts_with('.') || platform_hidden(&current) {
            return true;
        }
    }
    false
}

#[cfg(windows)]
fn platform_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    fs::metadata(path).is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn platform_hidden(_path: &Path) -> bool {
    false
}

fn build_excludes(root: &Path, patterns: &[String]) -> Result<Gitignore, SourceError> {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .map_err(|e| SourceError::Io(e.to_string()))?;
    }
    builder.build().map_err(|e| SourceError::Io(e.to_string()))
}

fn lexical_relative(value: &str) -> Result<PathBuf, SourceError> {
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized.starts_with("//")
        || (normalized.len() >= 2 && normalized.as_bytes()[1] == b':')
    {
        return Err(SourceError::Outside);
    }
    let mut output = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => output.push(part),
            Component::ParentDir => {
                if !output.pop() {
                    return Err(SourceError::Outside);
                }
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => return Err(SourceError::Outside),
        }
    }
    Ok(output)
}

pub(crate) fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn decode_text(bytes: &[u8]) -> Result<String, SourceError> {
    if bytes.contains(&0) {
        return Err(SourceError::Binary);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SourceError::UnsupportedEncoding)?;
    Ok(normalize_newlines(text))
}

fn map_io(error: std::io::Error) -> SourceError {
    match error.kind() {
        std::io::ErrorKind::NotFound => SourceError::NotFound,
        std::io::ErrorKind::PermissionDenied => SourceError::PermissionDenied,
        _ => SourceError::Io(error.to_string()),
    }
}

pub(crate) fn to_slash(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(v) => Some(v.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn logical_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').map(str::to_owned).collect()
}
