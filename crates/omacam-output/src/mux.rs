use std::collections::VecDeque;
use std::io::Write as _;
use std::process::ChildStdin;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::{RAW_FRAME_QUEUE_CAPACITY, STALE_FRAME_LIMIT, VideoFormat};

/// A bounded freshness-biased queue for decoded output frames.
///
/// The output writer owns the only consumer. Producers drop the oldest frame
/// when the queue is full so a slow decoder or sink cannot build latency.
pub(crate) struct RawFrameQueue {
    frames: Mutex<VecDeque<Vec<u8>>>,
    frame_bytes: usize,
}

impl RawFrameQueue {
    pub(crate) fn new(frame_bytes: usize) -> Self {
        Self {
            frames: Mutex::new(VecDeque::with_capacity(RAW_FRAME_QUEUE_CAPACITY)),
            frame_bytes,
        }
    }

    pub(crate) fn push(&self, frame: Vec<u8>) {
        if frame.len() != self.frame_bytes {
            return;
        }
        let mut frames = self.frames.lock().expect("raw frame queue mutex poisoned");
        while frames.len() >= RAW_FRAME_QUEUE_CAPACITY {
            frames.pop_front();
        }
        frames.push_back(frame);
    }

    pub(crate) fn take_latest(&self) -> Option<Vec<u8>> {
        let mut frames = self.frames.lock().expect("raw frame queue mutex poisoned");
        let latest = frames.pop_back();
        frames.clear();
        latest
    }

    pub(crate) fn clear(&self) {
        self.frames
            .lock()
            .expect("raw frame queue mutex poisoned")
            .clear();
    }
}

enum RawMuxCommand {
    Activate(mpsc::Sender<Result<(), String>>),
    Deactivate(mpsc::Sender<Result<(), String>>),
    Shutdown(mpsc::Sender<Result<(), String>>),
}

/// Writes one fixed raw stream to the service-lifetime `GStreamer` pipeline.
///
/// The mux has no decoder knowledge: it emits neutral I420 frames whenever no
/// current generation has a fresh decoded frame. That keeps the selected sink and
/// preview branch alive across every Stop/reconnect/rebind while ensuring an
/// old decoded frame cannot remain visible after reset.
pub(crate) struct RawMux {
    commands: mpsc::Sender<RawMuxCommand>,
    pub(crate) frames: Arc<RawFrameQueue>,
    error: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl RawMux {
    pub(crate) fn spawn(input: ChildStdin, format: VideoFormat) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let frames = Arc::new(RawFrameQueue::new(format.frame_bytes()));
        let error = Arc::new(Mutex::new(None));
        let thread_frames = Arc::clone(&frames);
        let thread_error = Arc::clone(&error);
        let thread = thread::spawn(move || {
            run_raw_mux(input, command_rx, thread_frames, thread_error, format);
        });
        Self {
            commands,
            frames,
            error,
            thread: Some(thread),
        }
    }

    pub(crate) fn activate(&self) -> Result<(), String> {
        self.frames.clear();
        self.send_command(RawMuxCommand::Activate)
    }

    pub(crate) fn deactivate(&self) -> Result<(), String> {
        self.frames.clear();
        self.send_command(RawMuxCommand::Deactivate)
    }

    pub(crate) fn clear(&self) {
        self.frames.clear();
    }

    pub(crate) fn check_health(&self) -> Result<(), String> {
        if let Some(error) = self
            .error
            .lock()
            .expect("raw mux error mutex poisoned")
            .clone()
        {
            return Err(error);
        }
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err("raw output mux stopped unexpectedly".to_owned());
        }
        Ok(())
    }

    fn send_command<F>(&self, make: F) -> Result<(), String>
    where
        F: FnOnce(mpsc::Sender<Result<(), String>>) -> RawMuxCommand,
    {
        self.check_health()?;
        let (ack_tx, ack_rx) = mpsc::channel();
        self.commands
            .send(make(ack_tx))
            .map_err(|_| "raw output mux stopped".to_owned())?;
        ack_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "raw output mux command timed out".to_owned())?
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        let command_result = if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            self.send_command(RawMuxCommand::Shutdown)
        } else {
            Ok(())
        };
        if let Some(thread) = self.thread.take()
            && (command_result.is_ok() || thread.is_finished())
        {
            let _ = thread.join();
        }
        command_result
    }
}

#[allow(clippy::needless_pass_by_value)]
fn run_raw_mux(
    mut input: ChildStdin,
    commands: mpsc::Receiver<RawMuxCommand>,
    frames: Arc<RawFrameQueue>,
    error: Arc<Mutex<Option<String>>>,
    format: VideoFormat,
) {
    let neutral = black_i420_frame(format);
    let mut active = false;
    let mut last_frame: Option<Vec<u8>> = None;
    let mut last_frame_at = None;
    let mut next_tick = Instant::now();

    loop {
        let wait = next_tick.saturating_duration_since(Instant::now());
        match commands.recv_timeout(wait) {
            Ok(RawMuxCommand::Activate(ack)) => {
                frames.clear();
                active = true;
                last_frame = None;
                last_frame_at = None;
                let _ = ack.send(Ok(()));
            }
            Ok(RawMuxCommand::Deactivate(ack)) => {
                frames.clear();
                active = false;
                last_frame = None;
                last_frame_at = None;
                let _ = ack.send(Ok(()));
            }
            Ok(RawMuxCommand::Shutdown(ack)) => {
                frames.clear();
                let _ = ack.send(Ok(()));
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                if active && let Some(frame) = frames.take_latest() {
                    last_frame = Some(frame);
                    last_frame_at = Some(now);
                }
                let frame = if active
                    && last_frame_at
                        .is_some_and(|at| now.saturating_duration_since(at) < STALE_FRAME_LIMIT)
                {
                    last_frame.as_deref().unwrap_or(&neutral)
                } else {
                    &neutral
                };
                if let Err(write_error) = input.write_all(frame) {
                    *error.lock().expect("raw mux error mutex poisoned") = Some(format!(
                        "service-scoped raw output writer failed: {write_error}"
                    ));
                    return;
                }
                next_tick = now + format.frame_interval();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

pub(crate) fn black_i420_frame(format: VideoFormat) -> Vec<u8> {
    let luma_bytes = format.width * format.height;
    let mut frame = vec![0_u8; format.frame_bytes()];
    frame[..luma_bytes].fill(16);
    frame[luma_bytes..].fill(128);
    frame
}
