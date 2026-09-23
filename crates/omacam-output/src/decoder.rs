use std::io::{self, Read as _, Write as _};
use std::process::Child;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;

use crate::config::VideoFormat;
use crate::mux::RawFrameQueue;
use crate::pipeline::build_decoder_command;

pub(crate) struct DecoderProcess {
    child: Child,
    input: Option<std::process::ChildStdin>,
    pump: Option<JoinHandle<Result<(), String>>>,
}

impl DecoderProcess {
    pub(crate) fn spawn(frames: Arc<RawFrameQueue>, format: VideoFormat) -> io::Result<Self> {
        let mut child = build_decoder_command(format).spawn()?;
        let input = child.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "decoder stdin is unavailable")
        })?;
        let mut output = child.stdout.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "decoder stdout is unavailable")
        })?;
        let pump = thread::spawn(move || {
            let mut frame = vec![0_u8; format.frame_bytes()];
            loop {
                match output.read_exact(&mut frame) {
                    Ok(()) => frames.push(std::mem::take(&mut frame)),
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(error) => return Err(format!("decoder output failed: {error}")),
                }
                frame = vec![0_u8; format.frame_bytes()];
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            pump: Some(pump),
        })
    }

    pub(crate) fn write(&mut self, payload: &[u8]) -> io::Result<()> {
        if let Some(status) = self.child.try_wait()? {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("H.264 decoder exited with {status}"),
            ));
        }
        let input = self.input.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "H.264 decoder input is closed")
        })?;
        input.write_all(payload)?;
        input.flush()
    }

    pub(crate) fn is_finished(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn stop(&mut self) -> Result<(), String> {
        self.input.take();
        let _ = self.child.kill();
        let wait_result = self
            .child
            .wait()
            .map_err(|error| format!("could not reap H.264 decoder: {error}"));
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
        wait_result.map(|_| ())
    }
}
