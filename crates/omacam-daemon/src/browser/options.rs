use std::net::SocketAddr;
use std::path::PathBuf;

use crate::{default_data_path, default_runtime_path};

#[derive(Debug)]
pub(crate) struct BrowserServeOptions {
    pub(super) endpoint: SocketAddr,
    pub(super) output_device: PathBuf,
    pub(super) qr_path: PathBuf,
    pub(super) identity_path: PathBuf,
}

impl BrowserServeOptions {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut endpoint = None;
        let mut output_device = None;
        let mut qr_path = default_runtime_path("browser-camera.svg");
        let mut identity_path = default_data_path("desktop-identity.json");
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag {
                "--endpoint" => {
                    endpoint = Some(value.parse::<SocketAddr>().map_err(|_| {
                        "--endpoint must be a private LAN IP and HTTPS port".to_owned()
                    })?);
                }
                "--output-device" => output_device = Some(PathBuf::from(value)),
                "--qr" => qr_path = PathBuf::from(value),
                "--identity" => identity_path = PathBuf::from(value),
                _ => return Err(format!("unknown browser option: {flag}")),
            }
            index += 2;
        }
        let endpoint = endpoint.ok_or_else(|| "--endpoint is required".to_owned())?;
        if endpoint.ip().is_unspecified() || endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err("--endpoint must be a concrete LAN address and non-zero port".to_owned());
        }
        Ok(Self {
            endpoint,
            output_device: output_device.ok_or_else(|| "--output-device is required".to_owned())?,
            qr_path,
            identity_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_options_require_endpoint_and_output() {
        assert_eq!(
            BrowserServeOptions::parse(&[]).unwrap_err(),
            "--endpoint is required"
        );
    }

    #[test]
    fn browser_options_reject_loopback_and_unknown_flags() {
        assert!(
            BrowserServeOptions::parse(&[
                "--endpoint".into(),
                "127.0.0.1:8443".into(),
                "--output-device".into(),
                "/dev/video0".into()
            ])
            .is_err()
        );
        assert!(
            BrowserServeOptions::parse(&[
                "--endpoint".into(),
                "192.168.1.10:8443".into(),
                "--output-device".into(),
                "/dev/video0".into(),
                "--cloud".into(),
                "yes".into()
            ])
            .is_err()
        );
    }
}
