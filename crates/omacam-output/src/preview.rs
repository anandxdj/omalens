use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const PREVIEW_CAPS: &str = "video/x-raw,format=RGBx,width=640,height=360,framerate=15/1";

pub(crate) fn add_preview_branch(command: &mut Command, socket: Option<&Path>) {
    let Some(socket) = socket else {
        return;
    };
    command.args([
        "decoded.",
        "!",
        "queue",
        "max-size-buffers=1",
        "max-size-bytes=0",
        "max-size-time=0",
        "leaky=downstream",
        "!",
        "videoconvert",
        "!",
        "videoscale",
        "!",
        "videorate",
        "drop-only=true",
        "!",
        PREVIEW_CAPS,
        "!",
        "unixfdsink",
        &format!("socket-path={}", socket.display()),
        "wait-for-connection=false",
        "sync=false",
    ]);
}

pub(crate) fn elements() -> &'static [&'static str] {
    &[
        "tee",
        "queue",
        "videoconvert",
        "videoscale",
        "videorate",
        "unixfdsink",
    ]
}
pub(crate) fn validate_preview_socket(socket: &Path) -> Result<PathBuf, String> {
    if !socket.is_absolute() {
        return Err("path must be absolute".to_owned());
    }
    if socket.exists() {
        return Err("path already exists; refusing to replace it".to_owned());
    }
    let parent = socket
        .parent()
        .ok_or_else(|| "path has no parent directory".to_owned())?;
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("{}: {error}", parent.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("parent must be a non-symlink directory".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "parent directory must not be accessible by group or other users".to_owned(),
            );
        }
    }
    Ok(socket.to_path_buf())
}
