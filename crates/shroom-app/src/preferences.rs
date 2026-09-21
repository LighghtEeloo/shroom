use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use shroom_core::{GuestUser, WorkspaceName};
use shroom_integrations::GuestDirectory;
use tempfile::NamedTempFile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Working directory preferences require a regular file and real private directories: {0}"
    )]
    UnsafePath(PathBuf),
    #[error("Working directory preference is too large; edit it or use the default")]
    TooLarge,
    #[error("Working directory preference version {0} is unsupported; edit it or use the default")]
    UnsupportedVersion(u32),
    #[error(
        "Could not read working directory preferences: {0}. Edit the setting or use the default."
    )]
    InvalidRecord(#[from] serde_json::Error),
    #[error("Invalid saved working directory: {0}. Edit the setting or use the default.")]
    InvalidDirectory(#[from] shroom_integrations::Error),
    #[error("Could not access working directory preferences: {0}")]
    Io(#[from] io::Error),
    #[error(
        "The preference changed, but its durability could not be confirmed: {0}. Refresh to read the saved value."
    )]
    Durability(io::Error),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum WorkingDirectoryPreference {
    #[default]
    Default,
    Custom(GuestDirectory),
}

impl WorkingDirectoryPreference {
    pub fn resolve(&self, user: &GuestUser) -> GuestDirectory {
        match self {
            Self::Default => user
                .default_working_directory()
                .parse()
                .expect("validated guest profile"),
            Self::Custom(directory) => directory.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    working_directory: String,
}

/// Access only while the owning Session holds Core's state-root lock.
pub struct PreferenceStore {
    pub state_dir: PathBuf,
}

impl PreferenceStore {
    const LIMIT: u64 = 16 * 1024;

    fn directory(path: &Path) -> Result<bool, Error> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() && metadata.permissions().mode() & 0o777 == 0o700 => {
                Ok(true)
            }
            Ok(_) => Err(Error::UnsafePath(path.into())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub fn workspace_directory(&self, name: &WorkspaceName) -> Result<PathBuf, Error> {
        let access = self.state_dir.join("access");
        let workspace = access.join(name.as_str());
        for path in [&access, &workspace] {
            if !Self::directory(path)? {
                return Err(Error::UnsafePath(path.clone()));
            }
        }
        Ok(workspace)
    }

    fn regular_file(path: &Path) -> Result<Option<fs::Metadata>, Error> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => Ok(Some(metadata)),
            Ok(_) => Err(Error::UnsafePath(path.into())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn load(&self, name: &WorkspaceName) -> Result<WorkingDirectoryPreference, Error> {
        let app = self.workspace_directory(name)?.join("app");
        if !Self::directory(&app)? {
            return Ok(WorkingDirectoryPreference::Default);
        }
        let path = app.join("launch.json");
        let Some(metadata) = Self::regular_file(&path)? else {
            return Ok(WorkingDirectoryPreference::Default);
        };
        if metadata.len() > Self::LIMIT {
            return Err(Error::TooLarge);
        }
        let mut bytes = Vec::new();
        File::open(&path)?
            .take(Self::LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > Self::LIMIT {
            return Err(Error::TooLarge);
        }
        let record: Record = serde_json::from_slice(&bytes)?;
        if record.version != 1 {
            return Err(Error::UnsupportedVersion(record.version));
        }
        Ok(WorkingDirectoryPreference::Custom(
            record.working_directory.parse()?,
        ))
    }

    pub fn save(
        &self,
        name: &WorkspaceName,
        preference: &WorkingDirectoryPreference,
    ) -> Result<(), Error> {
        let workspace = self.workspace_directory(name)?;
        let app = workspace.join("app");
        let exists = Self::directory(&app)?;
        let path = app.join("launch.json");
        if exists {
            Self::regular_file(&path)?;
        }
        let WorkingDirectoryPreference::Custom(directory) = preference else {
            if exists && Self::regular_file(&path)?.is_some() {
                fs::remove_file(path)?;
                File::open(&app)
                    .and_then(|file| file.sync_all())
                    .map_err(Error::Durability)?;
            }
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&Record {
            version: 1,
            working_directory: directory.to_string(),
        })?;
        if bytes.len() as u64 + 1 > Self::LIMIT {
            return Err(Error::TooLarge);
        }
        if !exists {
            fs::DirBuilder::new().mode(0o700).create(&app)?;
            File::open(&workspace)?.sync_all()?;
        }
        let mut staged = NamedTempFile::new_in(&app)?;
        staged.write_all(&bytes)?;
        staged.write_all(b"\n")?;
        staged.as_file().sync_all()?;
        staged
            .persist(&path)
            .map_err(|error| Error::Io(error.error))?;
        File::open(&app)
            .and_then(|file| file.sync_all())
            .map_err(Error::Durability)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
