use std::io;
use std::path::Path;
use std::process::Child;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use omacam_core::media::{
    MEDIA_HEADER_BYTES, MEDIA_MAGIC, MediaBinding, MediaRecordKind, MediaStreamValidator,
    OUTPUT_CONTROL_HEADER_BYTES, OUTPUT_CONTROL_MAGIC, OutputControlCommand, parse_output_control,
};

use crate::config::{OutputBackend, VideoFormat};
use crate::decoder::DecoderProcess;
use crate::mux::RawMux;
use crate::pipeline::spawn_raw_pipeline;

/// Service-scoped output pipeline.
///
/// `raw_pipeline` owns the selected backend sink and shared preview branch for the whole
/// service lifetime. Each capture generation gets at most one short-lived
/// H.264 decoder; Reset stops that decoder and only resumes after a fresh Bind.
/// Thus consumer handles survive reconnects while decoder state and stale
/// frames cannot cross a generation boundary.
pub(crate) struct ServiceOutput {
    raw_pipeline: Child,
    raw_mux: RawMux,
    decoder: Option<DecoderProcess>,
    binding: Option<MediaBinding>,
    validator: Option<MediaStreamValidator>,
    format: VideoFormat,
}

#[derive(Debug)]
pub(crate) enum ServiceInput {
    Control(OutputControlCommand),
    Media {
        header: [u8; MEDIA_HEADER_BYTES],
        payload: Vec<u8>,
    },
    End,
    Error(String),
}

impl ServiceOutput {
    pub(crate) fn spawn(
        device: &Path,
        preview_socket: Option<&Path>,
        format: VideoFormat,
        backend: OutputBackend,
    ) -> io::Result<Self> {
        let mut raw_pipeline = spawn_raw_pipeline(device, preview_socket, format, backend)?;
        let input = raw_pipeline.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "raw output stdin is unavailable")
        })?;
        let raw_mux = RawMux::spawn(input, format);
        Ok(Self {
            raw_pipeline,
            raw_mux,
            decoder: None,
            binding: None,
            validator: None,
            format,
        })
    }

    fn check_health(&mut self) -> Result<(), String> {
        self.raw_mux.check_health()?;
        if let Some(status) = self
            .raw_pipeline
            .try_wait()
            .map_err(|error| format!("could not inspect raw output pipeline: {error}"))?
        {
            return Err(format!(
                "raw output pipeline exited unexpectedly with {status}"
            ));
        }
        if let Some(decoder) = self.decoder.as_mut()
            && let Some(status) = decoder
                .is_finished()
                .map_err(|error| format!("could not inspect H.264 decoder: {error}"))?
        {
            return Err(format!("H.264 decoder exited unexpectedly with {status}"));
        }
        Ok(())
    }

    fn bind(&mut self, binding: MediaBinding) -> Result<(), Box<dyn std::error::Error>> {
        if self.binding.is_some() {
            return Err("output bind requires a reset of the active generation".into());
        }
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let decoder = DecoderProcess::spawn(Arc::clone(&self.raw_mux.frames), self.format)?;
        if let Err(error) = self.raw_mux.activate() {
            let mut decoder = decoder;
            let _ = decoder.stop();
            return Err(error.into());
        }
        self.decoder = Some(decoder);
        self.binding = Some(binding);
        self.validator = Some(MediaStreamValidator::new(binding));
        Ok(())
    }

    fn reset(&mut self, binding: MediaBinding) -> Result<(), Box<dyn std::error::Error>> {
        if self.binding.is_some_and(|current| current != binding) {
            return Err("output reset belongs to a different generation".into());
        }
        // Deactivate first so frames racing with a callback are discarded;
        // stop/join the decoder next, then clear anything produced in flight.
        let deactivate = self.raw_mux.deactivate();
        let decoder = self.decoder.take();
        let decoder_result = decoder.map_or(Ok(()), |mut decoder| decoder.stop());
        self.raw_mux.clear();
        self.binding = None;
        self.validator = None;
        if let Err(error) = deactivate {
            if let Err(decoder_error) = decoder_result {
                return Err(format!("{error}; decoder cleanup failed: {decoder_error}").into());
            }
            return Err(error.into());
        }
        decoder_result.map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Ok(())
    }

    fn write_record(
        &mut self,
        record: &omacam_core::media::MediaRecord,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let decoder = self
            .decoder
            .as_mut()
            .ok_or("media arrived before an output binding")?;
        decoder.write(&record.payload)?;
        Ok(())
    }

    pub(crate) fn run<R: io::Read + Send + 'static>(
        &mut self,
        reader: R,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (input_tx, input_rx) = mpsc::sync_channel(2);
        thread::spawn(move || {
            let mut reader = reader;
            loop {
                match read_service_input(&mut reader) {
                    Ok(Some(input)) => {
                        if input_tx.send(input).is_err() {
                            return;
                        }
                    }
                    Ok(None) => {
                        let _ = input_tx.send(ServiceInput::End);
                        return;
                    }
                    Err(error) => {
                        let _ = input_tx.send(ServiceInput::Error(error));
                        return;
                    }
                }
            }
        });

        loop {
            self.check_health()
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            match input_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ServiceInput::Control(command)) => match command {
                    OutputControlCommand::Bind(binding) => self.bind(binding)?,
                    OutputControlCommand::Reset(binding) => self.reset(binding)?,
                    OutputControlCommand::Shutdown => return Ok(()),
                },
                Ok(ServiceInput::Media { header, payload }) => {
                    let validator = self
                        .validator
                        .as_mut()
                        .ok_or("media arrived before an output binding")?;
                    let pending = validator.validate_header(&header)?;
                    let record = validator.finish_record(pending, payload)?;
                    if record.kind == MediaRecordKind::Stop {
                        let binding = self.binding.ok_or("media Stop has no active binding")?;
                        self.reset(binding)?;
                    } else {
                        self.write_record(&record)?;
                    }
                }
                Ok(ServiceInput::End) => return Ok(()),
                Ok(ServiceInput::Error(error)) => return Err(error.into()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("output service input reader stopped".into());
                }
            }
        }
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        let reset_result = self
            .binding
            .map(|binding| self.reset(binding).map_err(|error| error.to_string()));
        let mux_result = self.raw_mux.shutdown();
        let _ = self.raw_pipeline.kill();
        let wait_result = self
            .raw_pipeline
            .wait()
            .map_err(|error| format!("could not reap raw output pipeline: {error}"));
        if let Some(result) = reset_result {
            result?;
        }
        mux_result?;
        wait_result.map(|_| ())
    }
}

impl Drop for ServiceOutput {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

pub(crate) fn read_service_input<R: io::Read>(
    reader: &mut R,
) -> Result<Option<ServiceInput>, String> {
    let Some(prefix) = read_prefix(reader).map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    if prefix == MEDIA_MAGIC {
        let mut header = [0_u8; MEDIA_HEADER_BYTES];
        header[..MEDIA_MAGIC.len()].copy_from_slice(&prefix);
        reader
            .read_exact(&mut header[MEDIA_MAGIC.len()..])
            .map_err(|error| error.to_string())?;
        let payload_length =
            u32::from_be_bytes([header[100], header[101], header[102], header[103]]) as usize;
        if payload_length > omacam_core::media::MAX_H264_ACCESS_UNIT_BYTES {
            return Err("media payload exceeds the 1 MiB bound".to_owned());
        }
        let mut payload = vec![0_u8; payload_length];
        reader
            .read_exact(&mut payload)
            .map_err(|error| error.to_string())?;
        return Ok(Some(ServiceInput::Media { header, payload }));
    }
    if prefix != OUTPUT_CONTROL_MAGIC {
        return Err("output service command magic is invalid".to_owned());
    }
    let mut header = [0_u8; OUTPUT_CONTROL_HEADER_BYTES];
    header[..OUTPUT_CONTROL_MAGIC.len()].copy_from_slice(&prefix);
    reader
        .read_exact(&mut header[OUTPUT_CONTROL_MAGIC.len()..])
        .map_err(|error| error.to_string())?;
    parse_output_control(&header)
        .map(ServiceInput::Control)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Reads the service control envelope and media records from one private pipe.
///
/// A Bind/Reset only changes the validator; it never recreates the `GStreamer`
/// pipeline. A reset therefore leaves the virtual-camera handle attached while
/// the compositor's neutral branch takes over. Every media header is validated
/// against the currently bound generation before its payload is allocated.
#[cfg(test)]
pub(crate) fn forward_service<R: io::Read, W: io::Write>(
    reader: &mut R,
    decoder_input: &mut W,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut active_binding = None;
    let mut validator: Option<MediaStreamValidator> = None;
    loop {
        let Some(prefix) = read_prefix(reader)? else {
            return Ok(());
        };
        if prefix == MEDIA_MAGIC {
            let mut header = [0_u8; MEDIA_HEADER_BYTES];
            header[..MEDIA_MAGIC.len()].copy_from_slice(&prefix);
            reader.read_exact(&mut header[MEDIA_MAGIC.len()..])?;
            let Some(current) = validator.as_mut() else {
                return Err("media arrived before an output binding".into());
            };
            let pending = current.validate_header(&header)?;
            let mut payload = vec![0_u8; pending.payload_length()];
            reader.read_exact(&mut payload)?;
            let record = current.finish_record(pending, payload)?;
            if record.kind == MediaRecordKind::Stop {
                active_binding = None;
                validator = None;
            } else {
                decoder_input.write_all(&record.payload)?;
                decoder_input.flush()?;
            }
            continue;
        }
        if prefix != OUTPUT_CONTROL_MAGIC {
            return Err("output service command magic is invalid".into());
        }
        let mut header = [0_u8; OUTPUT_CONTROL_HEADER_BYTES];
        header[..OUTPUT_CONTROL_MAGIC.len()].copy_from_slice(&prefix);
        reader.read_exact(&mut header[OUTPUT_CONTROL_MAGIC.len()..])?;
        match parse_output_control(&header)? {
            OutputControlCommand::Bind(binding) => {
                // A new bind is valid only after the previous owner has been
                // reset by the daemon. The child also rejects a reset that is
                // not for the currently active binding below.
                if active_binding.is_some() {
                    return Err("output bind requires a reset of the active generation".into());
                }
                active_binding = Some(binding);
                validator = Some(MediaStreamValidator::new(binding));
            }
            OutputControlCommand::Reset(binding) => {
                if active_binding.is_some_and(|current| current != binding) {
                    return Err("output reset belongs to a different generation".into());
                }
                active_binding = None;
                validator = None;
            }
            OutputControlCommand::Shutdown => return Ok(()),
        }
    }
}

fn read_prefix<R: io::Read>(reader: &mut R) -> io::Result<Option<[u8; 8]>> {
    let mut prefix = [0_u8; 8];
    match reader.read(&mut prefix[..1])? {
        0 => return Ok(None),
        1 => {}
        _ => unreachable!("one-byte read returned more than one byte"),
    }
    reader.read_exact(&mut prefix[1..])?;
    Ok(Some(prefix))
}
pub(crate) fn forward_media<R: io::Read, W: io::Write>(
    reader: &mut R,
    mut decoder_input: W,
    validator: &mut MediaStreamValidator,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some(record) = validator.read_next(reader)? {
        match record.kind {
            MediaRecordKind::H264AccessUnit => {
                decoder_input.write_all(&record.payload)?;
                decoder_input.flush()?;
            }
            MediaRecordKind::Stop => return Ok(()),
        }
    }
    Ok(())
}
