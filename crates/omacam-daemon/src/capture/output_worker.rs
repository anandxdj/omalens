//! Service-owned adapter that binds validated H.264 access units to the output process.
use std::env;
use std::io;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use omacam_core::media::{
    MediaBinding, MediaRecord, MediaRecordKind, OutputControlCommand, write_output_control,
    write_record,
};

pub(crate) struct OutputWorker {
    child: Child,
    input: Option<ChildStdin>,
    current_binding: Option<MediaBinding>,
    failure: Option<String>,
}

impl OutputWorker {
    pub(crate) fn spawn_service(
        device: &Path,
        preview_socket: Option<&Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::spawn_service_with_format(device, preview_socket, 1280, 720, 30)
    }

    pub(crate) fn spawn_service_with_format(
        device: &Path,
        preview_socket: Option<&Path>,
        width: u16,
        height: u16,
        fps: u8,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let executable = env::current_exe()?
            .parent()
            .map(|parent| parent.join("omacam-output"))
            .filter(|path| path.exists())
            .unwrap_or_else(|| "omacam-output".into());
        // Keep the worker a child of the daemon and arrange for it to receive
        // TERM if the daemon disappears. Its GStreamer child does the same.
        let mut command = Command::new("setpriv");
        command.args(["--pdeathsig", "TERM"]).arg(executable);
        command.arg("--device").arg(device).arg("--service-stdin");
        command
            .arg("--width")
            .arg(width.to_string())
            .arg("--height")
            .arg(height.to_string())
            .arg("--fps")
            .arg(fps.to_string());
        if let Some(socket) = preview_socket {
            reject_existing_preview_path(socket)?;
            command.arg("--preview-socket").arg(socket);
        }
        let mut child = command.stdin(Stdio::piped()).spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or("output worker did not expose media input")?;
        let mut worker = Self {
            child,
            input: Some(input),
            current_binding: None,
            failure: None,
        };
        worker
            .check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Ok(worker)
    }

    pub(crate) fn check_health(&mut self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => {
                let error = format!("output worker exited unexpectedly with {status}");
                self.failure = Some(error.clone());
                self.input.take();
                Err(error)
            }
            Err(error) => {
                let message = format!("could not inspect output worker: {error}");
                self.failure = Some(message.clone());
                self.input.take();
                Err(message)
            }
        }
    }

    pub(crate) fn is_healthy(&mut self) -> bool {
        self.check_health().is_ok()
    }

    pub(crate) fn bind(&mut self, binding: MediaBinding) -> Result<(), Box<dyn std::error::Error>> {
        if self.current_binding.is_some() {
            return Err("output binding requires a reset before rebind".into());
        }
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let input = self
            .input
            .as_mut()
            .ok_or("output worker media input is closed")?;
        write_output_control(input, OutputControlCommand::Bind(binding))?;
        std::io::Write::flush(input)?;
        self.current_binding = Some(binding);
        Ok(())
    }

    pub(crate) fn reset(
        &mut self,
        binding: MediaBinding,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self
            .current_binding
            .is_some_and(|current| current != binding)
        {
            return Err("output reset belongs to a different generation".into());
        }
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if self.current_binding.is_some() {
            let input = self
                .input
                .as_mut()
                .ok_or("output worker media input is closed")?;
            write_output_control(input, OutputControlCommand::Reset(binding))?;
            std::io::Write::flush(input)?;
        }
        self.current_binding = None;
        Ok(())
    }

    /// Returns the time spent writing and flushing one frame to the output process.
    pub(crate) fn write(
        &mut self,
        binding: MediaBinding,
        record: &MediaRecord,
    ) -> Result<Duration, Box<dyn std::error::Error>> {
        let started = Instant::now();
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if self.current_binding != Some(binding) {
            return Err("output frame belongs to a stale or unbound generation".into());
        }
        if record.kind != MediaRecordKind::H264AccessUnit {
            return Err("output worker accepts access units only; use Reset for Stop".into());
        }
        write_record(
            self.input
                .as_mut()
                .ok_or("output worker media input is closed")?,
            binding,
            record,
        )?;
        std::io::Write::flush(
            self.input
                .as_mut()
                .ok_or("output worker media input is closed")?,
        )?;
        Ok(started.elapsed())
    }

    pub(crate) fn shutdown(&mut self) {
        if self.input.is_some()
            && self.failure.is_none()
            && let Some(input) = self.input.as_mut()
        {
            let _ = write_output_control(input, OutputControlCommand::Shutdown);
            let _ = std::io::Write::flush(input);
        }
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for OutputWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn reject_existing_preview_path(socket: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(socket) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("preview socket path already exists: {}", socket.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "omacam-output-worker-test-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("test directory is unique");
        path
    }

    fn assert_already_exists(result: Result<OutputWorker, Box<dyn std::error::Error>>) {
        let error = result
            .err()
            .expect("occupied preview path must be rejected");
        assert_eq!(
            error
                .downcast_ref::<io::Error>()
                .expect("preview path error retains its I/O kind")
                .kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn preview_socket_refuses_to_replace_existing_socket_or_path() {
        let parent = test_directory();
        let socket = parent.join("occupied.sock");
        let listener = UnixListener::bind(&socket).expect("test socket binds");

        let result = OutputWorker::spawn_service_with_format(
            Path::new("/dev/null"),
            Some(&socket),
            1280,
            720,
            30,
        );
        assert_already_exists(result);
        assert!(
            UnixStream::connect(&socket).is_ok(),
            "existing socket remains usable"
        );
        drop(listener);

        let file = parent.join("occupied-file");
        std::fs::write(&file, b"keep this file").expect("test file writes");
        let result = OutputWorker::spawn_service_with_format(
            Path::new("/dev/null"),
            Some(&file),
            1280,
            720,
            30,
        );
        assert_already_exists(result);
        assert_eq!(
            std::fs::read(&file).expect("existing file remains readable"),
            b"keep this file"
        );

        let dangling_symlink = parent.join("dangling-link");
        symlink(parent.join("missing-target"), &dangling_symlink).expect("test symlink is created");
        let result = OutputWorker::spawn_service_with_format(
            Path::new("/dev/null"),
            Some(&dangling_symlink),
            1280,
            720,
            30,
        );
        assert_already_exists(result);
        assert!(std::fs::symlink_metadata(&dangling_symlink).is_ok());

        std::fs::remove_dir_all(parent).expect("test directory is removed");
    }
}
