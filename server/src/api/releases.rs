//! Client builds for self-update, plus the one-line installers.
//!
//! `scripts/deploy-friday.sh` drops prebuilt `teresa` binaries and a
//! `manifest.json` into the releases directory; these routes only read it.
//! A teammate gets a new feature with `teresa update` — no Rust toolchain,
//! no `cargo install`. All routes are open: the binaries are no secret, and a
//! machine without the client cannot have logged in yet.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use teresa_proto::ClientRelease;
use tokio_util::io::ReaderStream;

use crate::error::{Result, ServerError};
use crate::state::SharedState;

use super::files::SHA256_HEADER;

/// Read buffer for binary downloads.
const READ_BUFFER: usize = 1024 * 1024;

/// Where the installers fetch from; replaced by the address they were
/// downloaded from.
const BASE_PLACEHOLDER: &str = "__TERESA_BASE__";

const INSTALL_SH: &str = include_str!("install.sh");
const INSTALL_PS1: &str = include_str!("install.ps1");

/// Reads the manifest the deploy script wrote, if any.
async fn manifest(state: &SharedState) -> Result<ClientRelease> {
    let path = state.config().releases_path().join("manifest.json");
    let raw = match tokio::fs::read(&path).await {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ServerError::NotFound("client release"))
        }
        Err(e) => {
            return Err(ServerError::Internal(anyhow::anyhow!(
                "reading {}: {e}",
                path.display()
            )))
        }
    };
    serde_json::from_slice(&raw)
        .map_err(|e| ServerError::Internal(anyhow::anyhow!("parsing {}: {e}", path.display())))
}

/// `GET /api/v1/client/latest` — the build on offer and its platforms.
pub async fn latest(State(state): State<SharedState>) -> Result<Json<ClientRelease>> {
    Ok(Json(manifest(&state).await?))
}

/// `GET /api/v1/client/download/{platform}` — streams that platform's binary.
pub async fn download(
    State(state): State<SharedState>,
    Path(platform): Path<String>,
) -> Result<Response> {
    let release = manifest(&state).await?;
    let asset = release
        .platforms
        .get(&platform)
        .ok_or(ServerError::NotFound("client build for that platform"))?;
    // The manifest is ours, but a path out of the directory is never right.
    if asset.file.is_empty() || asset.file.contains(['/', '\\']) || asset.file.starts_with('.') {
        return Err(ServerError::Internal(anyhow::anyhow!(
            "manifest names a bad file: {:?}",
            asset.file
        )));
    }

    let path = state.config().releases_path().join(&asset.file);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ServerError::Internal(anyhow::anyhow!("opening {}: {e}", path.display())))?;
    let size = file
        .metadata()
        .await
        .map_err(|e| ServerError::Internal(e.into()))?
        .len();

    let save_as = if platform.starts_with("windows") {
        "teresa.exe"
    } else {
        "teresa"
    };
    let mut response =
        Body::from_stream(ReaderStream::with_capacity(file, READ_BUFFER)).into_response();
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    out.insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    if let Ok(disposition) = HeaderValue::from_str(&format!("attachment; filename=\"{save_as}\"")) {
        out.insert(header::CONTENT_DISPOSITION, disposition);
    }
    if let Ok(sha) = HeaderValue::from_str(&asset.sha256) {
        out.insert(SHA256_HEADER, sha);
    }
    Ok(response)
}

/// `GET /install.sh` — `curl -fsSL http://<node>:7737/install.sh | sh`.
pub async fn install_sh(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    installer(&state, &headers, INSTALL_SH)
}

/// `GET /install.ps1` — `irm http://<node>:7737/install.ps1 | iex`.
pub async fn install_ps1(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    installer(&state, &headers, INSTALL_PS1)
}

/// Serves an installer pointed back at the address it was fetched from, so
/// the same script works by mDNS name, by IP, or over Tailscale.
fn installer(state: &SharedState, headers: &HeaderMap, script: &str) -> Response {
    let scheme = if state.config().tls.enabled() {
        "https"
    } else {
        "http"
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        // Only characters a host:port can contain; anything else would be
        // pasted into a shell script.
        .filter(|h| {
            !h.is_empty()
                && h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        })
        .map_or_else(
            || format!("127.0.0.1:{}", state.config().port),
            str::to_owned,
        );
    let body = script.replace(BASE_PLACEHOLDER, &format!("{scheme}://{host}"));
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}
