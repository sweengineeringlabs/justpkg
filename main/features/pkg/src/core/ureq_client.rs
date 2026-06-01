use std::io::Write;
use std::time::Duration;

use reqwest::blocking::ClientBuilder;

use crate::api::error::PkgError;
use crate::api::traits::HttpClient;
use crate::api::ureq_client::UreqClient;

fn make_client() -> reqwest::blocking::Client {
    ClientBuilder::new()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .use_native_tls()
        .build()
        .expect("reqwest client build")
}

fn map_error(url: &str, e: reqwest::Error) -> PkgError {
    match e.status() {
        Some(s) => PkgError::Http { url: url.to_string(), status: s.as_u16() },
        None    => PkgError::Http { url: url.to_string(), status: 0 },
    }
}

fn retry_with_backoff<F, T>(mut f: F, max_attempts: u32) -> Result<T, PkgError>
where
    F: FnMut() -> Result<T, PkgError>,
{
    let mut attempt = 0;
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(PkgError::Http { status: 0, .. }) if attempt < max_attempts => {
                attempt += 1;
                let delay_ms = 100 * (1 << (attempt - 1));
                eprintln!(
                    "[retry] transport error (attempt {}/{}), retrying in {}ms...",
                    attempt, max_attempts, delay_ms
                );
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
            Err(e) => return Err(e),
        }
    }
}

impl HttpClient for UreqClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PkgError> {
        retry_with_backoff(|| {
            let resp = make_client()
                .get(url)
                .send()
                .map_err(|e| map_error(url, e))?;
            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                return Err(PkgError::Http { url: url.to_string(), status });
            }
            resp.bytes()
                .map(|b| b.to_vec())
                .map_err(|e| map_error(url, e))
        }, 3)
    }

    fn get_stream(&self, url: &str, dest: &mut dyn Write) -> Result<u64, PkgError> {
        retry_with_backoff(|| {
            let mut resp = make_client()
                .get(url)
                .send()
                .map_err(|e| map_error(url, e))?;
            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                return Err(PkgError::Http { url: url.to_string(), status });
            }
            std::io::copy(&mut resp, dest).map_err(PkgError::Io)
        }, 3)
    }

    fn get_bytes_auth(&self, url: &str, token: Option<&str>) -> Result<Vec<u8>, PkgError> {
        let token = token.map(|t| t.to_string());
        retry_with_backoff(|| {
            let mut req = make_client().get(url);
            if let Some(ref t) = token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().map_err(|e| map_error(url, e))?;
            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                return Err(PkgError::Http { url: url.to_string(), status });
            }
            resp.bytes()
                .map(|b| b.to_vec())
                .map_err(|e| map_error(url, e))
        }, 3)
    }

    fn get_stream_auth(
        &self,
        url: &str,
        token: Option<&str>,
        dest: &mut dyn Write,
    ) -> Result<u64, PkgError> {
        let token = token.map(|t| t.to_string());
        retry_with_backoff(|| {
            let mut req = make_client().get(url);
            if let Some(ref t) = token {
                req = req.bearer_auth(t);
            }
            let mut resp = req.send().map_err(|e| map_error(url, e))?;
            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                return Err(PkgError::Http { url: url.to_string(), status });
            }
            std::io::copy(&mut resp, dest).map_err(PkgError::Io)
        }, 3)
    }
}
