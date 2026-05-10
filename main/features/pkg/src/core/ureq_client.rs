use std::io::Read;

use crate::api::error::JustpkgError;
use crate::api::traits::HttpClient;
use crate::api::ureq_client::UreqClient;

fn map_ureq_error(url: &str, e: ureq::Error) -> JustpkgError {
    match &e {
        ureq::Error::Status(code, _) => JustpkgError::Http {
            url: url.to_string(),
            status: *code,
        },
        // Transport errors (network, TLS, timeouts, etc.) — retry-eligible.
        // Status 0 indicates a transport/connection error, not an HTTP status code.
        _ => JustpkgError::Http {
            url: url.to_string(),
            status: 0,
        },
    }
}

/// Retry configuration for transient HTTP errors (status 0 = transport error).
/// Retries up to 3 times with exponential backoff: 100ms, 200ms, 400ms.
fn retry_with_backoff<F, T>(mut f: F, max_attempts: u32) -> Result<T, JustpkgError>
where
    F: FnMut() -> Result<T, JustpkgError>,
{
    let mut attempt = 0;
    loop {
        match f() {
            Ok(result) => return Ok(result),
            Err(JustpkgError::Http { status: 0, .. }) if attempt < max_attempts => {
                attempt += 1;
                let delay_ms = 100 * (1 << (attempt - 1)); // 100, 200, 400
                eprintln!("[retry] transport error (attempt {}/{}), retrying in {}ms...", attempt, max_attempts, delay_ms);
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            Err(e) => return Err(e),
        }
    }
}

impl HttpClient for UreqClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, JustpkgError> {
        let url = url.to_string();
        retry_with_backoff(
            || {
                let resp = ureq::get(&url).call().map_err(|e| map_ureq_error(&url, e))?;
                let mut buf = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(JustpkgError::Io)?;
                Ok(buf)
            },
            3,
        )
    }

    fn get_stream(&self, url: &str, dest: &mut dyn std::io::Write) -> Result<u64, JustpkgError> {
        let url = url.to_string();
        retry_with_backoff(
            || {
                let resp = ureq::get(&url).call().map_err(|e| map_ureq_error(&url, e))?;
                let n = std::io::copy(&mut resp.into_reader(), dest).map_err(JustpkgError::Io)?;
                Ok(n)
            },
            3,
        )
    }

    fn get_bytes_auth(&self, url: &str, token: Option<&str>) -> Result<Vec<u8>, JustpkgError> {
        let url = url.to_string();
        let token = token.map(|t| t.to_string());
        retry_with_backoff(
            || {
                let mut req = ureq::get(&url);
                if let Some(ref t) = token {
                    req = req.set("Authorization", &format!("Bearer {t}"));
                }
                let resp = req.call().map_err(|e| map_ureq_error(&url, e))?;
                let mut buf = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(JustpkgError::Io)?;
                Ok(buf)
            },
            3,
        )
    }

    fn get_stream_auth(
        &self,
        url: &str,
        token: Option<&str>,
        dest: &mut dyn std::io::Write,
    ) -> Result<u64, JustpkgError> {
        let url = url.to_string();
        let token = token.map(|t| t.to_string());
        retry_with_backoff(
            || {
                let mut req = ureq::get(&url);
                if let Some(ref t) = token {
                    req = req.set("Authorization", &format!("Bearer {t}"));
                }
                let resp = req.call().map_err(|e| map_ureq_error(&url, e))?;
                let n = std::io::copy(&mut resp.into_reader(), dest).map_err(JustpkgError::Io)?;
                Ok(n)
            },
            3,
        )
    }
}
