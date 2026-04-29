use std::io::Read;

use crate::api::traits::HttpClient;
use crate::api::ureq_client::UreqClient;
use crate::api::error::JustpkgError;

impl HttpClient for UreqClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, JustpkgError> {
        let resp = ureq::get(url).call().map_err(|e| JustpkgError::Http {
            url: url.to_string(),
            status: match &e {
                ureq::Error::Status(code, _) => *code,
                _ => 0,
            },
        })?;
        let mut buf = Vec::new();
        resp.into_reader()
            .read_to_end(&mut buf)
            .map_err(JustpkgError::Io)?;
        Ok(buf)
    }

    fn get_stream(&self, url: &str, dest: &mut dyn std::io::Write) -> Result<u64, JustpkgError> {
        let resp = ureq::get(url).call().map_err(|e| JustpkgError::Http {
            url: url.to_string(),
            status: match &e {
                ureq::Error::Status(code, _) => *code,
                _ => 0,
            },
        })?;
        let n = std::io::copy(&mut resp.into_reader(), dest).map_err(JustpkgError::Io)?;
        Ok(n)
    }
}
