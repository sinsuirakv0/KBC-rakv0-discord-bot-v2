//! Local contentを起動時に検証し、不変Snapshotとして保持する。

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub(crate) const MAX_CONTENT_FILES_PER_DIRECTORY: usize = 128;
pub(crate) const MAX_CONTENT_FILE_BYTES: u64 = 64 * 1024;

pub(crate) struct ContentCatalog {
    responses: BTreeMap<String, String>,
    help: BTreeMap<String, String>,
}

impl ContentCatalog {
    pub(crate) fn load(root_directory: &Path) -> Result<Self, ContentError> {
        Ok(Self {
            responses: load_directory(&root_directory.join("responses"))?,
            help: load_directory(&root_directory.join("help"))?,
        })
    }

    pub(crate) fn help(&self, command_name: &str) -> Option<&str> {
        self.help.get(command_name).map(String::as_str)
    }

    pub(crate) fn responses(&self) -> impl Iterator<Item = (&str, &str)> {
        self.responses
            .iter()
            .map(|(name, response)| (name.as_str(), response.as_str()))
    }
}

fn load_directory(directory: &Path) -> Result<BTreeMap<String, String>, ContentError> {
    let entries = fs::read_dir(directory).map_err(|source| ContentError::ReadDirectory {
        path: directory.to_owned(),
        source,
    })?;
    let mut files = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| ContentError::ReadDirectoryEntry {
            path: directory.to_owned(),
            source,
        })?;
        let file_type = entry
            .file_type()
            .map_err(|source| ContentError::ReadFileType {
                path: entry.path(),
                source,
            })?;
        if !file_type.is_file() {
            continue;
        }

        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| ContentError::InvalidFileName(entry.path()))?;
        let Some(command_name) = file_name.strip_suffix(".txt") else {
            continue;
        };
        if !is_valid_command_name(command_name) {
            return Err(ContentError::InvalidFileName(entry.path()));
        }
        if files.len() == MAX_CONTENT_FILES_PER_DIRECTORY {
            return Err(ContentError::TooManyFiles {
                path: directory.to_owned(),
                maximum: MAX_CONTENT_FILES_PER_DIRECTORY,
            });
        }
        files.push((command_name.to_owned(), entry.path()));
    }

    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut content_by_name = BTreeMap::new();
    for (command_name, path) in files {
        let content = read_content(&path)?;
        content_by_name.insert(command_name, content);
    }

    Ok(content_by_name)
}

fn is_valid_command_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte))
}

fn read_content(path: &Path) -> Result<String, ContentError> {
    let file = File::open(path).map_err(|source| ContentError::ReadFile {
        path: path.to_owned(),
        source,
    })?;
    let file_size = file
        .metadata()
        .map_err(|source| ContentError::ReadMetadata {
            path: path.to_owned(),
            source,
        })?
        .len();
    if file_size > MAX_CONTENT_FILE_BYTES {
        return Err(ContentError::FileTooLarge {
            path: path.to_owned(),
            maximum_bytes: MAX_CONTENT_FILE_BYTES,
        });
    }

    let mut bytes = Vec::with_capacity(file_size as usize);
    file.take(MAX_CONTENT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ContentError::ReadFile {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_CONTENT_FILE_BYTES {
        return Err(ContentError::FileTooLarge {
            path: path.to_owned(),
            maximum_bytes: MAX_CONTENT_FILE_BYTES,
        });
    }

    let content = String::from_utf8(bytes).map_err(|source| ContentError::InvalidUtf8 {
        path: path.to_owned(),
        source,
    })?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let mut normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.ends_with('\n') {
        normalized.pop();
    }
    if normalized.is_empty() {
        return Err(ContentError::EmptyContent(path.to_owned()));
    }

    Ok(normalized)
}

#[derive(Debug)]
pub(crate) enum ContentError {
    ReadDirectory {
        path: PathBuf,
        source: io::Error,
    },
    ReadDirectoryEntry {
        path: PathBuf,
        source: io::Error,
    },
    ReadFileType {
        path: PathBuf,
        source: io::Error,
    },
    InvalidFileName(PathBuf),
    TooManyFiles {
        path: PathBuf,
        maximum: usize,
    },
    ReadFile {
        path: PathBuf,
        source: io::Error,
    },
    ReadMetadata {
        path: PathBuf,
        source: io::Error,
    },
    FileTooLarge {
        path: PathBuf,
        maximum_bytes: u64,
    },
    InvalidUtf8 {
        path: PathBuf,
        source: std::string::FromUtf8Error,
    },
    EmptyContent(PathBuf),
}

impl Display for ContentError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadDirectory { path, .. } => {
                write!(
                    formatter,
                    "failed to read content directory: {}",
                    path.display()
                )
            }
            Self::ReadDirectoryEntry { path, .. } => write!(
                formatter,
                "failed to read an entry in content directory: {}",
                path.display()
            ),
            Self::ReadFileType { path, .. } => {
                write!(
                    formatter,
                    "failed to inspect content file: {}",
                    path.display()
                )
            }
            Self::InvalidFileName(path) => {
                write!(formatter, "invalid content filename: {}", path.display())
            }
            Self::TooManyFiles { path, maximum } => write!(
                formatter,
                "content directory exceeds {maximum} text files: {}",
                path.display()
            ),
            Self::ReadFile { path, .. } => {
                write!(formatter, "failed to read content file: {}", path.display())
            }
            Self::ReadMetadata { path, .. } => {
                write!(
                    formatter,
                    "failed to inspect content size: {}",
                    path.display()
                )
            }
            Self::FileTooLarge {
                path,
                maximum_bytes,
            } => write!(
                formatter,
                "content file exceeds {maximum_bytes} bytes: {}",
                path.display()
            ),
            Self::InvalidUtf8 { path, .. } => {
                write!(formatter, "content file is not UTF-8: {}", path.display())
            }
            Self::EmptyContent(path) => {
                write!(formatter, "content file is empty: {}", path.display())
            }
        }
    }
}

impl Error for ContentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ReadDirectory { source, .. }
            | Self::ReadDirectoryEntry { source, .. }
            | Self::ReadFileType { source, .. }
            | Self::ReadFile { source, .. }
            | Self::ReadMetadata { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::InvalidFileName(_)
            | Self::TooManyFiles { .. }
            | Self::FileTooLarge { .. }
            | Self::EmptyContent(_) => None,
        }
    }
}
