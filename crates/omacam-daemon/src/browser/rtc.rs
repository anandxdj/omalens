//! Host-candidate-only H.264 receive path and bounded freshness queue.

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError};
use omacam_core::media::{MediaBinding, MediaRecord, MediaRecordKind};
use str0m::change::SdpOffer;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};

use super::preview::output_loop;
use super::session::{CameraSession, VideoFormat};

const MEDIA_QUEUE_CAPACITY: usize = 2;

#[allow(clippy::needless_pass_by_value)]
pub(super) fn run_rtc(
    offer: SdpOffer,
    media_endpoint: SocketAddr,
    output_device: PathBuf,
    binding: MediaBinding,
    format: VideoFormat,
    session: Arc<Mutex<CameraSession>>,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut rtc = RtcConfig::new()
        .clear_codecs()
        .enable_h264(true)
        .build(Instant::now());
    // TCP and UDP have independent port spaces, so signaling and media can use
    // the configured LAN port without sharing a socket or adding cloud ICE.
    let socket = UdpSocket::bind(media_endpoint)?;
    let candidate = Candidate::host(socket.local_addr()?, "udp")?;
    rtc.add_local_candidate(candidate)
        .ok_or("could not add the LAN ICE candidate")?;
    let answer = rtc.sdp_api().accept_offer(offer)?;
    let encoded = serde_json::to_value(answer)?;
    let rtc_session = session.clone();
    thread::spawn(move || {
        let reason = match drive_rtc(rtc, &socket, output_device, binding, format, &rtc_session) {
            Ok(()) => "Media connection ended".to_owned(),
            Err(error) => {
                eprintln!("WebRTC session stopped: {error}");
                error.to_string()
            }
        };
        rtc_session.lock().unwrap().stop(&reason);
    });
    Ok(encoded)
}

fn drive_rtc(
    mut rtc: Rtc,
    socket: &UdpSocket,
    output_device: PathBuf,
    binding: MediaBinding,
    format: VideoFormat,
    session: &Arc<Mutex<CameraSession>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (media_tx, media_rx) = crossbeam_channel::bounded(MEDIA_QUEUE_CAPACITY);
    let eviction_rx = media_rx.clone();
    let output_session = Arc::clone(session);
    let output_thread = thread::spawn(move || {
        output_loop(&output_device, binding, format, media_rx, &output_session)
    });
    let mut buffer = vec![0_u8; 2_000];
    let mut first_media_time = None;
    let mut waiting_keyframe = true;

    loop {
        {
            let mut state = session.lock().unwrap();
            state.expire_heartbeat_if_needed(Instant::now());
            state.expire_pending_if_needed(Instant::now());
            if state.cancel.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
        }
        let timeout = match rtc.poll_output()? {
            Output::Timeout(value) => value,
            Output::Transmit(value) => {
                socket.send_to(&value.contents, value.destination)?;
                continue;
            }
            Output::Event(Event::MediaData(data)) => {
                if !data.contiguous || data.time.denom() == 0 {
                    session.lock().unwrap().observe_dropped_frames(1);
                    waiting_keyframe = true;
                    continue;
                }
                let is_keyframe = data.is_keyframe();
                let payload = data.data.to_vec();
                session
                    .lock()
                    .unwrap()
                    .observe_received_frame(is_keyframe, payload.len());
                let base = *first_media_time.get_or_insert(data.time.numer());
                let elapsed = data.time.numer().saturating_sub(base);
                let presentation_time_us =
                    elapsed.saturating_mul(1_000_000) / u64::from(data.time.denom());
                let record = MediaRecord {
                    kind: MediaRecordKind::H264AccessUnit,
                    key_frame: is_keyframe,
                    sequence: 0,
                    presentation_time_us,
                    payload,
                };
                send_fresh(
                    &media_tx,
                    &eviction_rx,
                    record,
                    &mut waiting_keyframe,
                    session,
                )?;
                continue;
            }
            Output::Event(Event::IceConnectionStateChange(IceConnectionState::Disconnected)) => {
                break;
            }
            Output::Event(_) => continue,
        };

        let wait = timeout.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            rtc.handle_input(Input::Timeout(Instant::now()))?;
            continue;
        }
        socket.set_read_timeout(Some(wait.min(Duration::from_millis(100))))?;
        let input = match socket.recv_from(&mut buffer) {
            Ok((length, source)) => Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: socket.local_addr()?,
                    contents: (&buffer[..length]).try_into()?,
                },
            ),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Input::Timeout(Instant::now())
            }
            Err(error) => return Err(error.into()),
        };
        rtc.handle_input(input)?;
    }
    drop(media_tx);
    output_thread
        .join()
        .map_err(|_| "output thread panicked")?
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn send_fresh(
    sender: &Sender<MediaRecord>,
    receiver: &Receiver<MediaRecord>,
    record: MediaRecord,
    waiting_keyframe: &mut bool,
    session: &Arc<Mutex<CameraSession>>,
) -> Result<(), &'static str> {
    if *waiting_keyframe && !record.key_frame {
        session.lock().unwrap().observe_dropped_frames(1);
        return Ok(());
    }
    if record.key_frame {
        *waiting_keyframe = false;
    }
    match sender.try_send(record) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(record)) => {
            let mut evicted = 0_u64;
            while receiver.try_recv().is_ok() {
                evicted += 1;
            }
            session.lock().unwrap().observe_dropped_frames(evicted);
            *waiting_keyframe = true;
            if record.key_frame {
                sender
                    .try_send(record)
                    .map_err(|_| "output worker disconnected")?;
                *waiting_keyframe = false;
            } else {
                session.lock().unwrap().observe_dropped_frames(1);
            }
            Ok(())
        }
        Err(TrySendError::Disconnected(_)) => Err("output worker disconnected"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::session::AppState;
    use super::*;

    #[test]
    fn terminal_cancellation_is_independent_of_a_full_media_queue() {
        let (sender, receiver) = crossbeam_channel::bounded(MEDIA_QUEUE_CAPACITY);
        let make_record = |sequence| MediaRecord {
            kind: MediaRecordKind::H264AccessUnit,
            key_frame: true,
            sequence,
            presentation_time_us: sequence,
            payload: vec![1],
        };
        sender.try_send(make_record(1)).unwrap();
        sender.try_send(make_record(2)).unwrap();
        let state = AppState::new(
            "phone".into(),
            "desktop".into(),
            "192.168.1.5:8443".parse().unwrap(),
            "/dev/video0".into(),
            "session".into(),
            Instant::now(),
        );
        state.session.lock().unwrap().stop("test stop");
        assert!(
            state
                .session
                .lock()
                .unwrap()
                .cancel
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert_eq!(receiver.len(), MEDIA_QUEUE_CAPACITY);
    }
}
