//! Local-only browser camera service.

mod http;
mod options;
mod preview;
mod rtc;
mod runtime;
mod session;

use std::io::Write as IoWrite;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Instant;

use axum_server::tls_rustls::RustlsConfig;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use omacam_core::browser::BrowserInvitation;
use qrcode::QrCode;
use qrcode::render::{svg, unicode};
use sha2::{Digest as _, Sha256};

use crate::{hostname_display_name, load_or_create_identity};

pub(crate) use options::BrowserServeOptions;
use runtime::{create_private_file, ensure_private_parent, validate_qr_target};
use session::AppState;

const PAGE: &str = include_str!("../../../../webapp/index.html");
const APP_CSS: &str = include_str!("../../../../webapp/app.css");
const SHARED_CONTROL_JS: &str = include_str!("../../../../webapp/shared-control.js");
const PHONE_JS: &str = include_str!("../../../../webapp/phone.js");
const DESKTOP_JS: &str = include_str!("../../../../webapp/desktop.js");

pub(crate) async fn run(options: BrowserServeOptions) -> Result<(), Box<dyn std::error::Error>> {
    str0m::crypto::from_feature_flags().install_process_default();
    let identity = load_or_create_identity(&options.identity_path)?;
    let certificate_digest = Sha256::digest(&identity.certificate_der);
    let mut phone_token = [0_u8; 32];
    getrandom::fill(&mut phone_token)?;
    let mut desktop_token = [0_u8; 32];
    getrandom::fill(&mut desktop_token)?;
    let mut session_id = [0_u8; 16];
    getrandom::fill(&mut session_id)?;

    let invitation = BrowserInvitation::create(
        &phone_token,
        certificate_digest.as_slice(),
        options.endpoint,
        &hostname_display_name(),
    )?;
    let url = invitation.qr_payload();
    write_qr(&options.qr_path, &url)?;
    let terminal_qr = QrCode::new(url.as_bytes())?
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .build();

    let desktop_token = URL_SAFE_NO_PAD.encode(desktop_token);
    println!("OmaCam local WebRTC camera");
    println!("Scan with the phone camera:\n{terminal_qr}");
    println!("QR image: {}", options.qr_path.display());
    println!("URL: {url}");
    println!(
        "Desktop: https://{}/desktop#token={desktop_token}",
        options.endpoint
    );
    println!("Waiting for one phone on https://{}", options.endpoint);

    let tls =
        RustlsConfig::from_der(vec![identity.certificate_der], identity.private_key_der).await?;
    let state = AppState::new(
        URL_SAFE_NO_PAD.encode(phone_token),
        desktop_token,
        options.endpoint,
        options.output_device,
        URL_SAFE_NO_PAD.encode(session_id),
        Instant::now(),
    );
    axum_server::bind_rustls(
        SocketAddr::new(IpAddr::from([0, 0, 0, 0]), options.endpoint.port()),
        tls,
    )
    .serve(http::router(state).into_make_service())
    .await?;
    Ok(())
}

fn write_qr(path: &Path, payload: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parent = ensure_private_parent(path)?;
    let file_name = path.file_name().ok_or("browser QR path must name a file")?;
    let path = parent.join(file_name);
    validate_qr_target(&path)?;
    let svg = QrCode::new(payload.as_bytes())?
        .render::<svg::Color>()
        .min_dimensions(420, 420)
        .build();
    let temporary = parent.join(format!(
        "{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let mut file = create_private_file(&temporary)?;
    file.write_all(svg.as_bytes())?;
    file.sync_all()?;
    drop(file);
    // Revalidate after writing so an unsafe pre-existing target is never replaced.
    validate_qr_target(&path)?;
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}
