//! File transfer over plain HTTP.
//!
//! Bytes ride raw request and response bodies — no multipart, no base64, no
//! JSON envelope — so a transfer moves at whatever the link and the disk
//! allow. Both routes sit outside the REST timeout: a 4 GiB upload over Wi-Fi
//! is supposed to take a while.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use teresa_proto::{FileId, FileKind, Message, RoomId};
use tokio_util::io::ReaderStream;

use crate::error::{Result, ServerError};
use crate::services::files::UploadMeta;
use crate::state::SharedState;

use super::auth::bearer_token;

/// Read buffer for downloads; matches the server's write buffer.
const READ_BUFFER: usize = 1024 * 1024;

/// The header carrying the stored SHA-256, so a receiver can verify the bytes
/// without trusting the message it learned about the file from.
pub const SHA256_HEADER: &str = "x-teresa-sha256";

/// Query string of an upload.
#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    /// File name as it should appear to receivers.
    pub name: String,
    /// File, folder or archive; plain file if omitted.
    #[serde(default)]
    pub kind: FileKind,
    /// Packed file count, for folders.
    #[serde(default)]
    pub entries: Option<u32>,
}

/// `POST /api/v1/rooms/{room}/files?name=…&kind=…` — body is the raw bytes.
pub async fn upload(
    State(state): State<SharedState>,
    Path(room): Path<String>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Message>)> {
    let actor = state.core.authenticate(bearer_token(&headers)?).await?;
    let room: RoomId = room
        .parse()
        .map_err(|_| ServerError::BadRequest("malformed room id".to_owned()))?;

    let meta = UploadMeta {
        name: query.name,
        kind: query.kind,
        entries: query.entries,
    };
    let message = state
        .core
        .receive_file(&actor, room, meta, body.into_data_stream())
        .await?;
    Ok((StatusCode::CREATED, Json(message)))
}

/// `GET /api/v1/files/{id}` — streams the stored bytes.
pub async fn download(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let actor = state.core.authenticate(bearer_token(&headers)?).await?;
    let id: FileId = id.parse().map_err(|_| ServerError::NotFound("file"))?;
    let (row, path) = state.core.open_file(&actor, id).await?;

    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ServerError::Internal(anyhow::anyhow!("opening {}: {e}", path.display())))?;
    let stream = ReaderStream::with_capacity(file, READ_BUFFER);

    let mut response = Body::from_stream(stream).into_response();
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    out.insert(header::CONTENT_LENGTH, HeaderValue::from(row.size));
    if let Ok(sha) = HeaderValue::from_str(&row.sha256) {
        out.insert(SHA256_HEADER, sha);
    }
    Ok(response)
}
