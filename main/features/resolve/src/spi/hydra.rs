use justpkg_pkg::HttpClient;

use crate::api::error::ResolveError;
use crate::api::types::{HydraBuildResponse, HydraEvalsResponse};

const HYDRA_BASE: &str = "https://hydra.nixos.org";

/// Find the store path for `attr` in the most recent Hydra evaluation of
/// `channel`'s jobset.
///
/// Call sequence:
///   1. GET /jobset/nixpkgs/<channel>/evals  → take evals[0].id
///   2. GET /eval/<id>/job/<attr>.<system>   → buildoutputs["out"].path
///
/// Returns `Ok(store_path)` on success.
/// Returns `Err(ResolveError::HydraAttrNotFound)` if Hydra has no build for `attr`.
pub fn hydra_lookup(
    http: &dyn HttpClient,
    channel: &str,
    attr: &str,
    system: &str,
) -> Result<String, ResolveError> {
    let eval_id = fetch_latest_eval_id(http, channel)?;
    let job_attr = format!("{attr}.{system}");
    fetch_store_path(http, eval_id, &job_attr, attr, channel)
}

fn fetch_latest_eval_id(
    http: &dyn HttpClient,
    channel: &str,
) -> Result<u64, ResolveError> {
    let url = format!("{HYDRA_BASE}/jobset/nixpkgs/{channel}/evals");
    let bytes = get_json(http, &url, channel)?;
    let resp: HydraEvalsResponse =
        serde_json::from_slice(&bytes).map_err(|e| ResolveError::HydraParse {
            url: url.clone(),
            message: e.to_string(),
        })?;
    resp.evals
        .first()
        .map(|e| e.id)
        .ok_or_else(|| ResolveError::HydraNoEval { channel: channel.to_string() })
}

fn fetch_store_path(
    http: &dyn HttpClient,
    eval_id: u64,
    job_attr: &str,
    attr: &str,
    channel: &str,
) -> Result<String, ResolveError> {
    let url = format!("{HYDRA_BASE}/eval/{eval_id}/job/{job_attr}");
    let bytes = match get_json(http, &url, channel) {
        Ok(b) => b,
        Err(ResolveError::HydraHttp { status: 404, .. }) => {
            return Err(ResolveError::HydraAttrNotFound {
                attr: attr.to_string(),
                channel: channel.to_string(),
            });
        }
        Err(e) => return Err(e),
    };

    let build: HydraBuildResponse =
        serde_json::from_slice(&bytes).map_err(|e| ResolveError::HydraParse {
            url: url.clone(),
            message: e.to_string(),
        })?;

    build
        .buildoutputs
        .get("out")
        .map(|o| o.path.clone())
        .ok_or_else(|| ResolveError::HydraNoOutput { attr: attr.to_string() })
}

/// GET `url` expecting JSON, map HTTP errors to typed `ResolveError`.
fn get_json(
    http: &dyn HttpClient,
    url: &str,
    channel: &str,
) -> Result<Vec<u8>, ResolveError> {
    http.get_bytes(url).map_err(|e| {
        // JustpkgError::Http carries the status code — surface it in the typed error.
        let msg = e.to_string();
        if let Some(status) = extract_status_from_http_error(&msg) {
            ResolveError::HydraHttp { url: url.to_string(), status }
        } else {
            ResolveError::ChannelFetch {
                channel: channel.to_string(),
                message: msg,
            }
        }
    })
}

/// Crude extraction of HTTP status from `JustpkgError::Http`'s Display string.
/// The format is "HTTP error fetching <url>: status <N>".
fn extract_status_from_http_error(msg: &str) -> Option<u16> {
    let prefix = "status ";
    let idx = msg.rfind(prefix)?;
    msg[idx + prefix.len()..].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::extract_status_from_http_error;

    #[test]
    fn test_extract_status_from_http_error_finds_code() {
        let msg = "HTTP error fetching https://hydra.nixos.org/foo: status 404";
        assert_eq!(extract_status_from_http_error(msg), Some(404));
    }

    #[test]
    fn test_extract_status_from_http_error_returns_none_for_other_errors() {
        assert_eq!(extract_status_from_http_error("connection refused"), None);
    }
}
