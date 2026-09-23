//! UVC output lifecycle and JPEG preview from the existing `UnixFD` shared decoder.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use crossbeam_channel::Receiver;
use omacam_core::media::{MediaBinding, MediaRecord};

use crate::capture::OutputWorker;
use crate::default_runtime_path;

use super::runtime::{
    ensure_private_parent, remove_socket_if_same, require_unused_socket_path, socket_identity,
};
use super::session::{CameraSession, PreviewFrame, VideoFormat};

pub(super) fn output_loop(
    device: &Path,
    binding: MediaBinding,
    format: VideoFormat,
    receiver: Receiver<MediaRecord>,
    session: &Arc<Mutex<CameraSession>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let configured_socket = default_runtime_path(&format!(
        "browser-{}.sock",
        session.lock().unwrap().session_id
    ));
    let parent = ensure_private_parent(&configured_socket).map_err(|error| error.to_string())?;
    let preview_socket = parent.join(
        configured_socket
            .file_name()
            .ok_or("preview socket path must name a file")?,
    );
    require_unused_socket_path(&preview_socket).map_err(|error| error.to_string())?;
    let mut output = OutputWorker::spawn_service_with_format(
        device,
        Some(&preview_socket),
        format.width,
        format.height,
        format.fps,
    )
    .map_err(|error| error.to_string())?;
    let preview_socket_identity = socket_identity(&preview_socket).ok().flatten();
    output.bind(binding).map_err(|error| error.to_string())?;
    let mut preview_child = spawn_preview_consumer(&preview_socket);
    let preview_reader = preview_child.as_mut().and_then(|child| child.stdout.take());
    let preview_thread = preview_reader.map(|stdout| {
        let preview_session = Arc::clone(session);
        thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                let Ok(bytes) = STANDARD.decode(line.trim()) else {
                    continue;
                };
                if bytes.len() > 256 * 1024
                    || !bytes.starts_with(&[0xff, 0xd8])
                    || !bytes.ends_with(&[0xff, 0xd9])
                {
                    continue;
                }
                let mut state = preview_session.lock().unwrap();
                if state.cancel.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                state.preview = Some(PreviewFrame {
                    captured_at: std::time::Instant::now(),
                    jpeg: bytes,
                });
                state.observe_preview_frame();
            }
        })
    });

    let write_result = (|| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for (sequence, mut record) in (0_u64..).zip(receiver) {
            if session
                .lock()
                .unwrap()
                .cancel
                .load(std::sync::atomic::Ordering::Acquire)
            {
                break;
            }
            record.sequence = sequence;
            output
                .write(binding, &record)
                .map_err(|error| error.to_string())?;
            session.lock().unwrap().observe_written_frame();
        }
        Ok(())
    })();

    // Reset reaches the existing output worker as its terminal stop, even when
    // a write failed or an HTTP Stop cancelled the bounded media queue.
    let reset_result = output.reset(binding).map_err(|error| error.to_string());
    if let Some(mut child) = preview_child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(reader) = preview_thread {
        let _ = reader.join();
    }
    remove_socket_if_same(&preview_socket, preview_socket_identity);
    write_result?;
    reset_result?;
    Ok(())
}

fn spawn_preview_consumer(socket: &PathBuf) -> Option<std::process::Child> {
    let executable = std::env::current_exe()
        .ok()?
        .with_file_name("omacam-preview");
    std::process::Command::new(executable)
        .arg("--socket")
        .arg(socket)
        .arg("--wait-for-socket")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
}
