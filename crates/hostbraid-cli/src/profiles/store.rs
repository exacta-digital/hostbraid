use super::model::ProfileConfig;
use fs2::FileExt;
use hostbraid_core::{AppError, ErrorCode, Result};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroizing;

#[cfg(unix)]
use rustix::fs::{Mode, OFlags};
#[cfg(not(unix))]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const CONFIG_FILE_NAME: &str = "profiles.json";
const LOCK_FILE_NAME: &str = ".profiles.lock";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const CONFIG_ACCESS_HINT: &str = "Check that the HostBraid config directory is accessible and owned by your user. Set `HOSTBRAID_CONFIG_HOME` to a private directory if needed.";
const CONFIG_WRITE_HINT: &str =
    "Check that the HostBraid config directory is owned by your user and writable, then retry.";
const CONFIG_REPAIR_HINT: &str =
    "Back up profiles.json, then repair it or move it aside; recreate profiles with `hb login`.";
const CONFIG_SAFETY_HINT: &str = "Move the unsafe path aside and recreate it as a real directory or file owned by your user; HostBraid will not follow symlinks.";
#[cfg(any(test, target_os = "windows"))]
const BACKUP_EXTENSION: &str = "json.previous";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct ConfigPaths {
    home: PathBuf,
    #[cfg(not(unix))]
    config: PathBuf,
    #[cfg(not(unix))]
    lock: PathBuf,
}

impl ConfigPaths {
    pub(crate) fn discover() -> Result<Self> {
        Self::discover_with(|name| std::env::var_os(name))
    }

    pub(crate) fn from_home(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            #[cfg(not(unix))]
            config: home.join(CONFIG_FILE_NAME),
            #[cfg(not(unix))]
            lock: home.join(LOCK_FILE_NAME),
            home,
        }
    }

    #[cfg(test)]
    pub(crate) fn home(&self) -> &Path {
        &self.home
    }

    #[cfg(test)]
    pub(crate) fn config_file(&self) -> PathBuf {
        self.home.join(CONFIG_FILE_NAME)
    }

    #[cfg(all(test, unix))]
    fn lock_file(&self) -> PathBuf {
        self.home.join(LOCK_FILE_NAME)
    }

    fn discover_with(mut read_environment: impl FnMut(&str) -> Option<OsString>) -> Result<Self> {
        if let Some(path) = nonempty_path(read_environment("HOSTBRAID_CONFIG_HOME")) {
            return Ok(Self::from_home(path));
        }

        #[cfg(target_os = "windows")]
        if let Some(path) = nonempty_path(read_environment("APPDATA")) {
            return Ok(Self::from_home(path.join("HostBraid")));
        }

        #[cfg(target_os = "macos")]
        if let Some(path) = nonempty_path(read_environment("HOME")) {
            return Ok(Self::from_home(
                path.join("Library")
                    .join("Application Support")
                    .join("HostBraid"),
            ));
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if let Some(path) = nonempty_path(read_environment("XDG_CONFIG_HOME")) {
                return Ok(Self::from_home(path.join("hostbraid")));
            }
            if let Some(path) = nonempty_path(read_environment("HOME")) {
                return Ok(Self::from_home(path.join(".config").join("hostbraid")));
            }
        }

        Err(AppError::new(
            ErrorCode::Io,
            "could not determine the HostBraid configuration directory",
        )
        .with_hint("Set HOSTBRAID_CONFIG_HOME to a private directory."))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileStore {
    paths: ConfigPaths,
    #[cfg(test)]
    fail_parent_sync: bool,
}

struct SecuredHome {
    #[cfg(unix)]
    directory: File,
    #[cfg(not(unix))]
    path: PathBuf,
}

impl ProfileStore {
    pub(crate) const fn new(paths: ConfigPaths) -> Self {
        Self {
            paths,
            #[cfg(test)]
            fail_parent_sync: false,
        }
    }

    #[cfg(test)]
    pub(crate) const fn with_parent_sync_failure(mut self) -> Self {
        self.fail_parent_sync = true;
        self
    }

    pub(crate) fn load(&self) -> Result<ProfileConfig> {
        let home = self.prepare_home()?;
        let lock = self.open_lock(&home)?;
        #[cfg(target_os = "windows")]
        FileExt::lock_exclusive(&lock).map_err(|error| {
            AppError::io("failed to lock profile configuration", &error)
                .with_hint(CONFIG_ACCESS_HINT)
        })?;
        #[cfg(not(target_os = "windows"))]
        FileExt::lock_shared(&lock).map_err(|error| {
            AppError::io("failed to lock profile configuration", &error)
                .with_hint(CONFIG_ACCESS_HINT)
        })?;
        #[cfg(target_os = "windows")]
        self.recover_interrupted_replacement()?;
        self.load_unlocked(&home)
    }

    pub(crate) fn update<T>(
        &self,
        operation: impl FnOnce(&mut ProfileConfig) -> Result<T>,
    ) -> Result<T> {
        let home = self.prepare_home()?;
        let lock = self.open_lock(&home)?;
        FileExt::lock_exclusive(&lock).map_err(|error| {
            AppError::io("failed to lock profile configuration", &error)
                .with_hint(CONFIG_ACCESS_HINT)
        })?;

        #[cfg(target_os = "windows")]
        self.recover_interrupted_replacement()?;
        let mut configuration = self.load_unlocked(&home)?;
        let value = operation(&mut configuration)?;
        configuration.normalize_and_validate()?;
        self.write_unlocked(&home, &configuration)?;
        Ok(value)
    }

    fn prepare_home(&self) -> Result<SecuredHome> {
        fs::create_dir_all(&self.paths.home).map_err(|error| {
            AppError::io(
                "failed to create the profile configuration directory",
                &error,
            )
            .with_hint(CONFIG_WRITE_HINT)
        })?;
        open_and_secure_home(&self.paths.home)
    }

    #[cfg(unix)]
    fn open_lock(&self, home: &SecuredHome) -> Result<File> {
        let file = open_file_at(
            &home.directory,
            LOCK_FILE_NAME,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
            "failed to open the profile configuration lock",
        )?;
        secure_open_regular_file(&file, "profile configuration lock")?;
        Ok(file)
    }

    #[cfg(not(unix))]
    fn open_lock(&self, _home: &SecuredHome) -> Result<File> {
        reject_existing_symlink(&self.paths.lock, "profile configuration lock")?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        let file = options.open(&self.paths.lock).map_err(|error| {
            AppError::io("failed to open the profile configuration lock", &error)
        })?;
        secure_file_permissions(&self.paths.lock)?;
        Ok(file)
    }

    #[cfg(unix)]
    fn load_unlocked(&self, home: &SecuredHome) -> Result<ProfileConfig> {
        let file = match rustix::fs::openat(
            &home.directory,
            CONFIG_FILE_NAME,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(file) => File::from(file),
            Err(error) if error == rustix::io::Errno::NOENT => {
                return Ok(ProfileConfig::default());
            }
            Err(error) => {
                return Err(AppError::io(
                    "failed to open the profile configuration",
                    &std::io::Error::from(error),
                )
                .with_hint(CONFIG_ACCESS_HINT));
            }
        };
        secure_open_regular_file(&file, "profile configuration file")?;
        read_configuration(file)
    }

    #[cfg(not(unix))]
    fn load_unlocked(&self, _home: &SecuredHome) -> Result<ProfileConfig> {
        reject_existing_symlink(&self.paths.config, "profile configuration file")?;
        let file = match File::open(&self.paths.config) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ProfileConfig::default());
            }
            Err(error) => {
                return Err(
                    AppError::io("failed to open the profile configuration", &error)
                        .with_hint(CONFIG_ACCESS_HINT),
                );
            }
        };
        secure_file_permissions(&self.paths.config)?;
        read_configuration(file)
    }

    fn write_unlocked(&self, home: &SecuredHome, configuration: &ProfileConfig) -> Result<()> {
        let mut contents = serde_json::to_vec_pretty(configuration).map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                "failed to serialize the profile configuration",
            )
            .with_hint(
                "Retry once. If it repeats, update HostBraid and report the failure with `hb --version`.",
            )
        })?;
        contents.push(b'\n');
        if contents.len() > MAX_CONFIG_BYTES as usize {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "the profile change would exceed the supported configuration size",
            )
            .with_hint(
                "The change was not saved. Remove unneeded profiles or reduce stored metadata, then retry.",
            ));
        }
        write_configuration(
            home,
            &self.paths,
            &contents,
            self.parent_sync_failure_injected(),
        )
    }

    #[cfg(target_os = "windows")]
    fn recover_interrupted_replacement(&self) -> Result<()> {
        recover_interrupted_replacement(
            &self.paths.config,
            &replacement_backup_path(&self.paths.config),
        )
    }

    const fn parent_sync_failure_injected(&self) -> bool {
        #[cfg(test)]
        {
            self.fail_parent_sync
        }
        #[cfg(not(test))]
        {
            false
        }
    }
}

fn temporary_file_name() -> String {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        ".{CONFIG_FILE_NAME}.{}.{}.tmp",
        std::process::id(),
        sequence
    )
}

/// The destination name changes at the successful rename. Everything after that is
/// best-effort durability work and must not make callers treat the metadata update as rolled back.
fn commit_replacement(
    replace: impl FnOnce() -> Result<()>,
    sync_parent: impl FnOnce() -> Result<()>,
) -> Result<()> {
    replace()?;
    let _ = sync_parent();
    Ok(())
}

#[cfg(unix)]
fn write_configuration(
    home: &SecuredHome,
    _paths: &ConfigPaths,
    contents: &[u8],
    fail_parent_sync: bool,
) -> Result<()> {
    let temp_name = temporary_file_name();
    let mut cleanup = TempFileCleanup::new(&home.directory, &temp_name);
    let mut file = open_file_at(
        &home.directory,
        &temp_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
        "failed to create a temporary profile configuration",
    )?;
    secure_open_regular_file(&file, "temporary profile configuration")?;
    file.write_all(contents)
        .map_err(|error| AppError::io("failed to write the profile configuration", &error))?;
    file.sync_all()
        .map_err(|error| AppError::io("failed to sync the profile configuration", &error))?;
    drop(file);

    commit_replacement(
        || {
            rustix::fs::renameat(
                &home.directory,
                &temp_name,
                &home.directory,
                CONFIG_FILE_NAME,
            )
            .map_err(|error| {
                AppError::io(
                    "failed to replace the profile configuration",
                    &std::io::Error::from(error),
                )
            })?;
            cleanup.disarm();
            Ok(())
        },
        || {
            if fail_parent_sync {
                return Err(AppError::new(ErrorCode::Io, "injected parent sync failure"));
            }
            home.directory.sync_all().map_err(|error| {
                AppError::io("failed to sync the profile configuration directory", &error)
            })
        },
    )
}

#[cfg(not(unix))]
fn write_configuration(
    home: &SecuredHome,
    paths: &ConfigPaths,
    contents: &[u8],
    fail_parent_sync: bool,
) -> Result<()> {
    let temp_path = home.path.join(temporary_file_name());
    let mut cleanup = TempFileCleanup::new(&temp_path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| {
            AppError::io("failed to create a temporary profile configuration", &error)
        })?;
    secure_file_permissions(&temp_path)?;
    file.write_all(contents)
        .map_err(|error| AppError::io("failed to write the profile configuration", &error))?;
    file.sync_all()
        .map_err(|error| AppError::io("failed to sync the profile configuration", &error))?;
    drop(file);

    commit_replacement(
        || {
            replace_file(&temp_path, &paths.config)?;
            cleanup.disarm();
            Ok(())
        },
        || {
            if fail_parent_sync {
                return Err(AppError::new(ErrorCode::Io, "injected parent sync failure"));
            }
            sync_parent_directory(&home.path)
        },
    )
}

#[cfg(unix)]
struct TempFileCleanup<'a> {
    directory: &'a File,
    name: &'a str,
    armed: bool,
}

#[cfg(unix)]
impl<'a> TempFileCleanup<'a> {
    const fn new(directory: &'a File, name: &'a str) -> Self {
        Self {
            directory,
            name,
            armed: true,
        }
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for TempFileCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = rustix::fs::unlinkat(self.directory, self.name, rustix::fs::AtFlags::empty());
        }
    }
}

#[cfg(not(unix))]
struct TempFileCleanup<'a> {
    path: &'a Path,
    armed: bool,
}

#[cfg(not(unix))]
impl TempFileCleanup<'_> {
    const fn new(path: &Path) -> TempFileCleanup<'_> {
        TempFileCleanup { path, armed: true }
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(not(unix))]
impl Drop for TempFileCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(self.path);
        }
    }
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination)
        .map_err(|error| AppError::io("failed to replace the profile configuration", &error))
}

#[cfg(all(not(unix), target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    // Windows does not guarantee that rename replaces an existing destination. Keep the fallback
    // confined to this platform; the exclusive profile lock prevents another HostBraid process
    // from observing or writing the short replacement window.
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error)
            if destination.exists()
                && matches!(
                    error.kind(),
                    std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                ) =>
        {
            let backup = replacement_backup_path(destination);
            let _ = fs::remove_file(&backup);
            fs::rename(destination, &backup).map_err(|rename_error| {
                AppError::io(
                    "failed to prepare the profile configuration replacement",
                    &rename_error,
                )
            })?;
            if let Err(rename_error) = fs::rename(source, destination) {
                let _ = fs::rename(&backup, destination);
                return Err(AppError::io(
                    "failed to replace the profile configuration",
                    &rename_error,
                ));
            }
            let _ = fs::remove_file(backup);
            Ok(())
        }
        Err(error) => Err(AppError::io(
            "failed to replace the profile configuration",
            &error,
        )),
    }
}

#[cfg(any(test, target_os = "windows"))]
fn replacement_backup_path(destination: &Path) -> PathBuf {
    destination.with_extension(BACKUP_EXTENSION)
}

/// Recover the only crash window in the Windows backup-then-rename fallback.
///
/// Callers hold the profile lock exclusively. If the new destination already exists, it is the
/// committed, fsynced temporary file and a remaining backup is stale. If the destination is absent,
/// the previous configuration is restored before any read or update proceeds.
#[cfg(any(test, target_os = "windows"))]
fn recover_interrupted_replacement(destination: &Path, backup: &Path) -> Result<()> {
    let destination_exists = replacement_file_exists(destination, "profile configuration file")?;
    let backup_exists = replacement_file_exists(backup, "profile configuration backup")?;

    if destination_exists {
        if backup_exists {
            // The replacement was committed before interruption. Cleanup is best effort because
            // the authoritative destination is already present and safe to read.
            let _ = fs::remove_file(backup);
        }
        return Ok(());
    }

    if backup_exists {
        fs::rename(backup, destination).map_err(|error| {
            AppError::io(
                "failed to recover an interrupted profile configuration replacement",
                &error,
            )
        })?;
    }
    Ok(())
}

#[cfg(any(test, target_os = "windows"))]
fn replacement_file_exists(path: &Path, description: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(true)
        }
        Ok(_) => Err(AppError::new(
            ErrorCode::Io,
            format!("{description} is not a safe regular file"),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AppError::io(
            "failed to inspect the profile configuration replacement",
            &error,
        )),
    }
}

#[cfg(not(unix))]
fn secure_directory_permissions(path: &Path) -> Result<()> {
    validate_directory(path)
}

#[cfg(not(unix))]
fn secure_file_permissions(path: &Path) -> Result<()> {
    validate_regular_file(path)
}

#[cfg(not(unix))]
fn validate_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AppError::io(
            "failed to inspect the profile configuration directory",
            &error,
        )
    })?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::Io,
        "profile configuration directory is not a safe directory",
    )
    .with_hint(CONFIG_SAFETY_HINT))
}

#[cfg(not(unix))]
fn validate_regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AppError::io("failed to inspect the profile configuration file", &error)
    })?;
    if metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::Io,
        "profile configuration path is not a safe regular file",
    )
    .with_hint(CONFIG_SAFETY_HINT))
}

#[cfg(not(unix))]
fn reject_existing_symlink(path: &Path, description: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::new(
            ErrorCode::Io,
            format!("{description} must not be a symbolic link"),
        )
        .with_hint(CONFIG_SAFETY_HINT)),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::io(
            "failed to inspect the profile configuration path",
            &error,
        )),
    }
}

#[cfg(unix)]
fn open_and_secure_home(path: &Path) -> Result<SecuredHome> {
    let directory = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| {
        AppError::io(
            "failed to safely open the profile configuration directory",
            &std::io::Error::from(error),
        )
    })?;
    secure_open_directory(&directory)?;
    Ok(SecuredHome { directory })
}

#[cfg(not(unix))]
fn open_and_secure_home(path: &Path) -> Result<SecuredHome> {
    secure_directory_permissions(path)?;
    Ok(SecuredHome {
        path: path.to_owned(),
    })
}

#[cfg(unix)]
fn open_file_at(
    directory: &File,
    name: &str,
    flags: OFlags,
    mode: Mode,
    error_message: &str,
) -> Result<File> {
    rustix::fs::openat(directory, name, flags, mode)
        .map(File::from)
        .map_err(|error| AppError::io(error_message, &std::io::Error::from(error)))
}

#[cfg(unix)]
fn secure_open_directory(directory: &File) -> Result<()> {
    let metadata = directory.metadata().map_err(|error| {
        AppError::io(
            "failed to inspect the profile configuration directory",
            &error,
        )
    })?;
    if !metadata.is_dir() {
        return Err(AppError::new(
            ErrorCode::Io,
            "profile configuration directory is not a safe directory",
        )
        .with_hint(CONFIG_SAFETY_HINT));
    }
    validate_unix_owner(&metadata, "profile configuration directory")?;
    directory
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| {
            AppError::io(
                "failed to secure the profile configuration directory",
                &error,
            )
        })?;
    let secured = directory.metadata().map_err(|error| {
        AppError::io(
            "failed to verify the profile configuration directory",
            &error,
        )
    })?;
    validate_unix_owner(&secured, "profile configuration directory")?;
    if secured.mode() & 0o7777 != 0o700 {
        return Err(AppError::new(
            ErrorCode::Io,
            "profile configuration directory permissions could not be secured",
        )
        .with_hint(CONFIG_SAFETY_HINT));
    }
    Ok(())
}

#[cfg(unix)]
fn secure_open_regular_file(file: &File, description: &str) -> Result<()> {
    let metadata = file.metadata().map_err(|error| {
        AppError::io("failed to inspect the profile configuration file", &error)
    })?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(AppError::new(
            ErrorCode::Io,
            format!("{description} is not a safe regular file"),
        )
        .with_hint(CONFIG_SAFETY_HINT));
    }
    validate_unix_owner(&metadata, description)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| AppError::io("failed to secure the profile configuration file", &error))?;
    let secured = file
        .metadata()
        .map_err(|error| AppError::io("failed to verify the profile configuration file", &error))?;
    validate_unix_owner(&secured, description)?;
    if !secured.is_file() || secured.nlink() != 1 || secured.mode() & 0o7777 != 0o600 {
        return Err(AppError::new(
            ErrorCode::Io,
            format!("{description} permissions could not be secured"),
        )
        .with_hint(CONFIG_SAFETY_HINT));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_unix_owner(metadata: &fs::Metadata, description: &str) -> Result<()> {
    if metadata.uid() == rustix::process::geteuid().as_raw() {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::Io,
        format!("{description} is not owned by the current user"),
    )
    .with_hint(CONFIG_SAFETY_HINT))
}

fn read_configuration(file: File) -> Result<ProfileConfig> {
    let length = file
        .metadata()
        .map_err(|error| AppError::io("failed to inspect the profile configuration", &error))?
        .len();
    if length > MAX_CONFIG_BYTES {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "profile configuration is larger than the supported limit",
        )
        .with_hint(CONFIG_REPAIR_HINT));
    }

    let mut contents = Zeroizing::new(Vec::with_capacity(usize::try_from(length).unwrap_or(0)));
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|error| AppError::io("failed to read the profile configuration", &error))?;
    if contents.len() > MAX_CONFIG_BYTES as usize {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "profile configuration is larger than the supported limit",
        )
        .with_hint(CONFIG_REPAIR_HINT));
    }
    let mut configuration: ProfileConfig = serde_json::from_slice(&contents).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidInput,
            "profile configuration is not valid HostBraid JSON",
        )
        .with_hint(CONFIG_REPAIR_HINT)
    })?;
    configuration.normalize_and_validate()?;
    Ok(configuration)
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn nonempty_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{
        ConfigPaths, MAX_CONFIG_BYTES, ProfileConfig, ProfileStore, commit_replacement,
        recover_interrupted_replacement, replacement_backup_path,
    };
    use crate::profiles::{CredentialSource, ProfileRecord};
    use hostbraid_core::{AppError, ErrorCode, OpaqueId, ProfileName, ProviderId};
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "hostbraid-profile-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn profile(name: &str) -> ProfileRecord {
        ProfileRecord {
            provider: ProviderId::new("kinsta").expect("provider"),
            name: ProfileName::new(name).expect("profile"),
            company_id: OpaqueId::new(format!("company-{name}")).expect("company"),
            credential_source: CredentialSource::Keyring,
            credential_expires_at: None,
        }
    }

    #[test]
    fn explicit_config_home_override_wins() {
        let paths = ConfigPaths::discover_with(|name| match name {
            "HOSTBRAID_CONFIG_HOME" => Some(OsString::from("/private/hostbraid-test")),
            "XDG_CONFIG_HOME" => Some(OsString::from("/ignored")),
            _ => None,
        })
        .expect("config paths");
        assert_eq!(paths.home(), Path::new("/private/hostbraid-test"));
    }

    #[test]
    fn writes_are_versioned_deterministic_and_secret_free() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path());
        let store = ProfileStore::new(paths.clone());
        store
            .update(|configuration| {
                configuration.profiles.push(profile("zeta"));
                configuration.profiles.push(ProfileRecord {
                    credential_source: CredentialSource::environment("KINSTA_TOKEN")?,
                    ..profile("alpha")
                });
                Ok(())
            })
            .expect("write profiles");

        let contents = fs::read_to_string(paths.config_file()).expect("read configuration");
        assert!(contents.contains("\"schema_version\": 1"));
        assert!(contents.contains("KINSTA_TOKEN"));
        assert!(!contents.contains("api-token-value"));
        assert!(contents.find("alpha").expect("alpha") < contents.find("zeta").expect("zeta"));
        assert_eq!(store.load().expect("load").profiles.len(), 2);
    }

    #[test]
    fn oversized_serialized_configuration_is_rejected_before_a_temp_write() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path());
        let store = ProfileStore::new(paths.clone());
        let configuration = ProfileConfig {
            profiles: (0..10_000)
                .map(|index| profile(&format!("profile-{index}")))
                .collect(),
            ..ProfileConfig::default()
        };
        let serialized_bytes = serde_json::to_vec_pretty(&configuration)
            .expect("configuration serializes")
            .len()
            + 1;
        assert!(serialized_bytes > MAX_CONFIG_BYTES as usize);

        let home = store.prepare_home().expect("prepare configuration home");
        let error = store
            .write_unlocked(&home, &configuration)
            .expect_err("oversized configuration is rejected");

        assert_eq!(error.code(), ErrorCode::InvalidInput);
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("The change was not saved"))
        );
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("Remove unneeded profiles"))
        );
        assert!(!paths.config_file().exists());
        assert!(
            fs::read_dir(paths.home())
                .expect("read configuration directory")
                .all(|entry| !entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn interrupted_replacement_restores_the_previous_configuration() {
        let directory = TestDirectory::new();
        let destination = directory.path().join("profiles.json");
        let backup = replacement_backup_path(&destination);
        fs::write(&backup, "previous configuration").expect("write replacement backup");

        recover_interrupted_replacement(&destination, &backup)
            .expect("recover interrupted replacement");

        assert_eq!(
            fs::read_to_string(&destination).expect("read recovered configuration"),
            "previous configuration"
        );
        assert!(!backup.exists());
    }

    #[test]
    fn committed_replacement_discards_a_stale_backup() {
        let directory = TestDirectory::new();
        let destination = directory.path().join("profiles.json");
        let backup = replacement_backup_path(&destination);
        fs::write(&destination, "new configuration").expect("write committed configuration");
        fs::write(&backup, "previous configuration").expect("write stale backup");

        recover_interrupted_replacement(&destination, &backup)
            .expect("clean committed replacement");

        assert_eq!(
            fs::read_to_string(&destination).expect("read committed configuration"),
            "new configuration"
        );
        assert!(!backup.exists());
    }

    #[test]
    fn invalid_configuration_errors_do_not_echo_file_contents() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path());
        fs::write(
            paths.config_file(),
            r#"{"schema_version":1,"api_token":"secret-canary","profiles":[]}"#,
        )
        .expect("write invalid configuration");
        let store = ProfileStore::new(paths);

        let error = store.load().expect_err("unknown secret field is rejected");
        assert!(
            error
                .hint()
                .is_some_and(|hint| hint.contains("Back up profiles.json"))
        );
        assert!(!format!("{error:?}").contains("secret-canary"));
        assert!(!error.to_string().contains("secret-canary"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_permissions_are_restrictive() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path().join("config"));
        let store = ProfileStore::new(paths.clone());
        store.update(|_| Ok(())).expect("write configuration");

        let directory_mode = fs::metadata(paths.home())
            .expect("directory metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(paths.config_file())
            .expect("file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn configuration_symlinks_are_rejected_without_touching_the_target() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path().join("config"));
        fs::create_dir_all(paths.home()).expect("create configuration directory");
        let target = directory.path().join("unrelated-file");
        fs::write(&target, "do-not-touch").expect("write symlink target");
        symlink(&target, paths.config_file()).expect("create configuration symlink");
        let store = ProfileStore::new(paths);

        let error = store.load().expect_err("configuration symlink is rejected");

        assert_eq!(error.code(), hostbraid_core::ErrorCode::Io);
        assert_eq!(
            fs::read_to_string(target).expect("read untouched target"),
            "do-not-touch"
        );
    }

    #[cfg(unix)]
    #[test]
    fn lock_symlinks_are_rejected_without_changing_the_target() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path().join("config"));
        fs::create_dir_all(paths.home()).expect("create configuration directory");
        let target = directory.path().join("unrelated-lock-target");
        fs::write(&target, "do-not-touch").expect("write symlink target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
            .expect("set target permissions");
        symlink(&target, paths.lock_file()).expect("create lock symlink");
        let store = ProfileStore::new(paths);

        let error = store.load().expect_err("lock symlink is rejected");

        assert_eq!(error.code(), ErrorCode::Io);
        assert_eq!(
            fs::read_to_string(&target).expect("read untouched target"),
            "do-not-touch"
        );
        assert_eq!(
            fs::metadata(target)
                .expect("target metadata")
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_configuration_is_rejected_before_permissions_change() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::from_home(directory.path().join("config"));
        fs::create_dir_all(paths.home()).expect("create configuration directory");
        let target = directory.path().join("unrelated-hard-link-target");
        fs::write(&target, "not-profile-json").expect("write hard-link target");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
            .expect("set target permissions");
        fs::hard_link(&target, paths.config_file()).expect("create configuration hard link");
        let store = ProfileStore::new(paths);

        let error = store
            .load()
            .expect_err("hard-linked configuration is rejected");

        assert_eq!(error.code(), ErrorCode::Io);
        assert_eq!(
            fs::metadata(target)
                .expect("target metadata")
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[cfg(unix)]
    #[test]
    fn configuration_home_symlink_is_rejected_without_chmodding_target() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let target = directory.path().join("actual-directory");
        fs::create_dir(&target).expect("create target directory");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
            .expect("set target permissions");
        let linked_home = directory.path().join("linked-config");
        symlink(&target, &linked_home).expect("create home symlink");
        let store = ProfileStore::new(ConfigPaths::from_home(linked_home));

        let error = store.load().expect_err("home symlink is rejected");

        assert_eq!(error.code(), ErrorCode::Io);
        assert_eq!(
            fs::metadata(target)
                .expect("target metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[test]
    fn post_commit_sync_failure_does_not_report_the_replacement_as_rolled_back() {
        let replaced = Cell::new(false);
        let sync_attempted = Cell::new(false);

        let result = commit_replacement(
            || {
                replaced.set(true);
                Ok(())
            },
            || {
                sync_attempted.set(true);
                Err(AppError::new(ErrorCode::Io, "injected sync failure"))
            },
        );

        assert!(result.is_ok());
        assert!(replaced.get());
        assert!(sync_attempted.get());
    }

    #[test]
    fn concurrent_updates_are_serialized_without_lost_profiles() {
        let directory = TestDirectory::new();
        let store = Arc::new(ProfileStore::new(ConfigPaths::from_home(directory.path())));
        let barrier = Arc::new(Barrier::new(2));

        let first_store = Arc::clone(&store);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_store.update(|configuration| {
                configuration.profiles.push(profile("first"));
                first_barrier.wait();
                thread::sleep(Duration::from_millis(50));
                Ok(())
            })
        });

        barrier.wait();
        let second_store = Arc::clone(&store);
        let second = thread::spawn(move || {
            second_store.update(|configuration| {
                configuration.profiles.push(profile("second"));
                Ok(())
            })
        });

        first.join().expect("first thread").expect("first update");
        second
            .join()
            .expect("second thread")
            .expect("second update");
        let configuration = store.load().expect("load");
        let names: Vec<&str> = configuration
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert_eq!(names, ["first", "second"]);
    }
}
