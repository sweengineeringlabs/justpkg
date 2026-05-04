use std::io::Read;

use crate::api::error::JustpkgError;
use crate::api::traits::HttpClient;
use crate::api::ureq_client::UreqClient;

fn map_ureq_error(url: &str, e: ureq::Error) -> JustpkgError {
    JustpkgError::Http {
        url: url.to_string(),
        status: match &e {
            ureq::Error::Status(code, _) => *code,
            _ => 0,
        },
    }
}

impl HttpClient for UreqClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, JustpkgError> {
        let resp = ureq::get(url).call().map_err(|e| map_ureq_error(url, e))?;
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf).map_err(JustpkgError::Io)?;
        Ok(buf)
    }

    fn get_stream(&self, url: &str, dest: &mut dyn std::io::Write) -> Result<u64, JustpkgError> {
        let resp = ureq::get(url).call().map_err(|e| map_ureq_error(url, e))?;
        let n = std::io::copy(&mut resp.into_reader(), dest).map_err(JustpkgError::Io)?;
        Ok(n)
    }

    fn get_bytes_auth(&self, url: &str, token: Option<&str>) -> Result<Vec<u8>, JustpkgError> {
        let mut req = ureq::get(url);
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let resp = req.call().map_err(|e| map_ureq_error(url, e))?;
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf).map_err(JustpkgError::Io)?;
        Ok(buf)
    }

    fn get_stream_auth(
        &self,
        url: &str,
        token: Option<&str>,
        dest: &mut dyn std::io::Write,
    ) -> Result<u64, JustpkgError> {
        let mut req = ureq::get(url);
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let resp = req.call().map_err(|e| map_ureq_error(url, e))?;
        let n = std::io::copy(&mut resp.into_reader(), dest).map_err(JustpkgError::Io)?;
        Ok(n)
    }
}
