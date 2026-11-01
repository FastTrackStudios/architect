//! A store's blobs over HTTP, ranged: the server side of
//! [`super::HttpRemote`], and what a browser's `<audio>` seeks against.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};

use super::{LocalFs, Sha256};

/// Answer `GET …/{hash}` from `store`, honouring `Range`.
///
/// `200`/`206`/`416` by the request's `Range` (`architect::http::ranged`),
/// `404` for a blob it does not hold, `400` for a hash that is not 64 hex
/// digits. Mount it on a route of
/// your own (`#[http(get, path = "/blobs/{hash}")]` or an axum route)
/// behind whatever decides who may read.
pub async fn serve_http(store: &LocalFs, headers: &HeaderMap, hash: &str) -> Response {
    let Ok(hash) = Sha256::from_hex(hash) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let path = store.path_for(&hash);
    match crate::http::ranged::serve_file(headers, &path, "application/octet-stream").await {
        Ok(res) => res,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
