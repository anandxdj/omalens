//! Private runtime paths for browser invitations and shared preview sockets.

use std::io;
use std::os::unix::fs::{
    DirBuilderExt as _, FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _,
};
use std::path::{Component, Path, PathBuf};

pub(super) fn ensure_private_parent(file_path: &Path) -> io::Result<PathBuf> {
    let parent = file_path.parent().unwrap_or_else(|| Path::new("."));
    ensure_private_directory(parent)
}

pub(super) fn ensure_private_directory(path: &Path) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime directory path is empty",
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let normalized = normalize_path(&absolute);
    let mut current = PathBuf::new();
    for component in normalized.components() {
        match component {
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(name) => {
                current.push(name);
                match std::fs::symlink_metadata(&current) {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(unsafe_runtime_path(&current));
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        std::fs::DirBuilder::new().mode(0o700).create(&current)?;
                        validate_private_directory(&current)?;
                    }
                    Err(error) => return Err(error),
                }
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "runtime directory path is not normalized",
                ));
            }
        }
    }
    validate_private_directory(&current)?;
    Ok(current)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(name) => normalized.push(name),
            Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn validate_private_directory(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    let current_uid = current_uid()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != current_uid
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(unsafe_runtime_path(path));
    }
    Ok(())
}

fn unsafe_runtime_path(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "refusing unsafe browser runtime path {} (expected an owned 0700 directory)",
            path.display()
        ),
    )
}

fn current_uid() -> io::Result<u32> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:")?.split_whitespace().nth(1))
        .ok_or_else(|| io::Error::other("could not determine process uid"))?;
    uid.parse()
        .map_err(|_| io::Error::other("could not determine process uid"))
}

pub(super) fn validate_qr_target(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.uid() != current_uid()?
                || metadata.mode() & 0o077 != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "refusing unsafe existing browser QR path {}",
                        path.display()
                    ),
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) fn require_unused_socket_path(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "refusing to remove existing preview socket path {}",
                path.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SocketIdentity {
    device: u64,
    inode: u64,
    owner: u32,
}

pub(super) fn socket_identity(path: &Path) -> io::Result<Option<SocketIdentity>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == current_uid()? => {
            Ok(Some(SocketIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
                owner: metadata.uid(),
            }))
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn remove_socket_if_same(path: &Path, identity: Option<SocketIdentity>) {
    let Some(identity) = identity else {
        return;
    };
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_socket()
        && metadata.dev() == identity.device
        && metadata.ino() == identity.inode
        && metadata.uid() == identity.owner
    {
        let _ = std::fs::remove_file(path);
    }
}

pub(super) fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "omacam-browser-runtime-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn creates_missing_private_runtime_directory() {
        let root = test_dir("create");
        let runtime = root.join("new").join("runtime");
        let ensured = ensure_private_directory(&runtime).unwrap();
        assert_eq!(ensured, runtime);
        let metadata = std::fs::metadata(ensured).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_insecure_existing_runtime_directory() {
        let root = test_dir("insecure");
        let runtime = root.join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            ensure_private_directory(&runtime).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_symlink_runtime_and_qr_paths() {
        let root = test_dir("symlink");
        let private = root.join("private");
        std::fs::create_dir(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        let directory_link = root.join("link");
        std::os::unix::fs::symlink(&private, &directory_link).unwrap();
        assert_eq!(
            ensure_private_directory(&directory_link)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );

        let qr_target = root.join("actual.svg");
        std::fs::write(&qr_target, "do not follow").unwrap();
        let qr_link = root.join("qr.svg");
        std::os::unix::fs::symlink(&qr_target, &qr_link).unwrap();
        assert_eq!(
            validate_qr_target(&qr_link).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(std::fs::read_to_string(qr_target).unwrap(), "do not follow");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn existing_socket_path_is_refused_and_never_removed() {
        let root = test_dir("socket");
        let socket_path = root.join("preview.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        assert_eq!(
            require_unused_socket_path(&socket_path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(
            std::fs::symlink_metadata(&socket_path)
                .unwrap()
                .file_type()
                .is_socket()
        );
        drop(listener);
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn socket_cleanup_keeps_replaced_path() {
        let root = test_dir("replace");
        let socket_path = root.join("preview.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let identity = socket_identity(&socket_path).unwrap();
        drop(listener);
        std::fs::remove_file(&socket_path).unwrap();
        std::fs::write(&socket_path, "unrelated").unwrap();
        remove_socket_if_same(&socket_path, identity);
        assert_eq!(std::fs::read_to_string(&socket_path).unwrap(), "unrelated");
        let _ = std::fs::remove_file(socket_path);
        let _ = std::fs::remove_dir_all(root);
    }
}
