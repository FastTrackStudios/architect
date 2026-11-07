//! A device's roots over `WebDAV`: the placeholder surface.
//!
//! Every root this device knows, pinned or not, at
//! `<mount>/<org>/<Projects|Assets>/<name>/…`. A folder is listed from
//! the manifest, so a project nobody fetched browses offline; a `GET`
//! of a file whose bytes are not here streams them from a peer while
//! they arrive (`Fetcher::open_at`) and keeps them, so it opens offline
//! next time; a `PUT` saves a version. Finder (Connect to Server), GNOME
//! Files and `gio mount`, davfs2 and Windows mount it with nothing in the
//! kernel. FUSE and a File Provider are later faces over the same calls.
//!
//! What it speaks: `OPTIONS`, `PROPFIND` (depth 0 and 1), `GET`/`HEAD`
//! with `Range`, `PUT`, `MKCOL`, `DELETE`, `MOVE`, `COPY`, and the
//! `LOCK`/`UNLOCK` a Mac asks for before it writes (a token, nothing held).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use architect::blob::Sha256;
use architect::http::axum::body::Body;
use architect::http::axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use architect::http::axum::response::{IntoResponse as _, Response};
use architect::http::ranged::{reader_body, serve_body};
use futures_util::StreamExt as _;
use tokio::io::AsyncReadExt as _;
use uuid::Uuid;

use super::engine::Engine;
use super::manifest::{Section, doc_id_for};

/// Folders made over `WebDAV` and not holding a file yet (the manifest
/// keeps files; a client makes a folder before it puts into it).
#[derive(Clone, Default)]
pub struct Dav {
    made: Arc<Mutex<BTreeSet<String>>>,
}

/// Where a path in the mount lands.
enum At {
    Top,
    Org(String),
    Section(String, Section),
    Root {
        id: Uuid,
        place: String,
        path: String,
    },
}

impl Dav {
    /// Answer one request: `method` on `rel` (in-mount, decoded), the
    /// mount at `mount` (for hrefs).
    pub async fn handle(
        &self,
        engine: &Engine,
        method: &Method,
        mount: &str,
        rel: &str,
        headers: &HeaderMap,
        body: Body,
    ) -> Response {
        let rel = rel.trim_matches('/');
        let mount = mount.trim_end_matches('/');
        match method.as_str() {
            "OPTIONS" => options(),
            "PROPFIND" => self.propfind(engine, mount, rel, headers).await,
            "GET" | "HEAD" => self.get(engine, rel, headers, method == Method::HEAD).await,
            "PUT" => self.put(engine, rel, body).await,
            "MKCOL" => self.mkcol(engine, rel).await,
            "DELETE" => self.delete(engine, rel).await,
            "MOVE" | "COPY" => {
                self.relocate(engine, mount, rel, headers, method.as_str() == "MOVE")
                    .await
            }
            "LOCK" => lock(rel),
            "UNLOCK" => StatusCode::NO_CONTENT.into_response(),
            "PROPPATCH" => multistatus(""),
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        }
    }

    async fn locate(&self, engine: &Engine, rel: &str) -> Option<At> {
        let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        let roots = engine.status().await;
        match segs.as_slice() {
            [] => Some(At::Top),
            [org] => roots
                .iter()
                .any(|r| r.org == *org)
                .then(|| At::Org((*org).to_owned())),
            [org, section] => {
                let section = Section::from_dir(section)?;
                Some(At::Section((*org).to_owned(), section))
            }
            [org, section, name, rest @ ..] => {
                let place = format!("{org}/{section}/{name}");
                let id = doc_id_for(&place);
                engine.knows(id).then(|| At::Root {
                    id,
                    place,
                    path: rest.join("/"),
                })
            }
        }
    }

    async fn propfind(
        &self,
        engine: &Engine,
        mount: &str,
        rel: &str,
        headers: &HeaderMap,
    ) -> Response {
        let depth_one = headers.get("depth").and_then(|d| d.to_str().ok()) != Some("0");
        let Some(at) = self.locate(engine, rel).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let href = |p: &str| format!("{mount}/{}", encode(p));
        let mut xml = String::new();
        match at {
            At::Top | At::Org(_) | At::Section(..) => {
                xml.push_str(&entry(&href(rel), leaf(rel), None));
                if depth_one {
                    let roots = engine.status().await;
                    let names: BTreeSet<String> = match &at {
                        At::Top => roots.iter().map(|r| r.org.clone()).collect(),
                        At::Org(org) => roots
                            .iter()
                            .filter(|r| &r.org == org)
                            .map(|r| r.section.dir().to_owned())
                            .collect(),
                        At::Section(org, section) => roots
                            .iter()
                            .filter(|r| &r.org == org && &r.section == section)
                            .map(|r| r.name.clone())
                            .collect(),
                        At::Root { .. } => BTreeSet::new(),
                    };
                    for name in names {
                        let child = join(rel, &name);
                        xml.push_str(&entry(&href(&child), &name, None));
                    }
                }
            }
            At::Root { id, place, path } => {
                let Ok(tree) = engine.tree(id).await else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                if let Some(v) = (!path.is_empty()).then(|| tree.current(&path)).flatten() {
                    xml.push_str(&entry(
                        &href(rel),
                        leaf(rel),
                        Some((v.size, v.at_us, v.content.as_deref())),
                    ));
                    return multistatus(&xml);
                }
                let entries = engine.entries(&tree, &path);
                let made = self.made_under(&place, &path);
                if !path.is_empty() && entries.is_empty() && !self.is_made(&place, &path) {
                    return StatusCode::NOT_FOUND.into_response();
                }
                xml.push_str(&entry(&href(rel), leaf(rel), None));
                if depth_one {
                    let mut seen = BTreeSet::new();
                    for e in &entries {
                        seen.insert(e.name.clone());
                        let child = join(rel, &e.name);
                        let file = (!e.dir).then_some((e.size, e.at_us, e.content.as_deref()));
                        xml.push_str(&entry(&href(&child), &e.name, file));
                    }
                    for name in made.into_iter().filter(|n| !seen.contains(n)) {
                        xml.push_str(&entry(&href(&join(rel, &name)), &name, None));
                    }
                }
            }
        }
        multistatus(&xml)
    }

    async fn get(&self, engine: &Engine, rel: &str, headers: &HeaderMap, head: bool) -> Response {
        let Some(At::Root { id, path, .. }) = self.locate(engine, rel).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let Some(v) = engine.tree(id).await.ok().and_then(|t| t.current(&path)) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let Some(hash) = v.content.as_deref().and_then(|c| Sha256::from_hex(c).ok()) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let mime = mime_of(&path);
        let served = serve_body(headers, v.size, mime, |offset, len| async move {
            if head {
                return Ok(Body::empty());
            }
            let reader: architect::blob::Reader = match engine.store.file_for(&hash) {
                Some(file) => {
                    super::store::open_from(&file, offset)
                        .await
                        .map_err(std::io::Error::other)?
                        .reader
                }
                // Not here: stream it from a peer as it arrives; it stays.
                None => {
                    engine
                        .fetcher
                        .open_at(hash, offset)
                        .await
                        .map_err(std::io::Error::other)?
                        .reader
                }
            };
            Ok::<_, std::io::Error>(reader_body(reader.take(len)))
        })
        .await;
        let mut response =
            served.unwrap_or_else(|e| (StatusCode::BAD_GATEWAY, e.to_string()).into_response());
        if let Some(content) = &v.content
            && let Ok(etag) = HeaderValue::from_str(&format!("\"{content}\""))
        {
            response.headers_mut().insert(header::ETAG, etag);
        }
        response
    }

    async fn put(&self, engine: &Engine, rel: &str, body: Body) -> Response {
        let Some(At::Root { id, place, path }) = self.locate(engine, rel).await else {
            return StatusCode::CONFLICT.into_response();
        };
        if path.is_empty() || super::layout::ignored(&path) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let stream = body
            .into_data_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other));
        let reader = tokio_util::io::StreamReader::new(stream);
        let existed = engine
            .tree(id)
            .await
            .ok()
            .and_then(|t| t.current(&path))
            .is_some();
        match engine.save_reader(id, &path, Box::pin(reader)).await {
            Ok(_) => {
                self.unmake(&place, &path);
                engine.pass(id).await;
                if existed {
                    StatusCode::NO_CONTENT
                } else {
                    StatusCode::CREATED
                }
                .into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }

    async fn mkcol(&self, engine: &Engine, rel: &str) -> Response {
        let Some(At::Root { place, path, .. }) = self.locate(engine, rel).await else {
            return StatusCode::FORBIDDEN.into_response();
        };
        if path.is_empty() {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        self.made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(format!("{place}/{path}"));
        StatusCode::CREATED.into_response()
    }

    async fn delete(&self, engine: &Engine, rel: &str) -> Response {
        let Some(At::Root { id, place, path }) = self.locate(engine, rel).await else {
            return StatusCode::FORBIDDEN.into_response();
        };
        if path.is_empty() {
            return StatusCode::FORBIDDEN.into_response();
        }
        let made = self
            .made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&format!("{place}/{path}"));
        match engine.delete_path(id, &path).await {
            Ok(0) if !made => StatusCode::NOT_FOUND.into_response(),
            Ok(_) => {
                engine.pass(id).await;
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }

    async fn relocate(
        &self,
        engine: &Engine,
        mount: &str,
        rel: &str,
        headers: &HeaderMap,
        remove: bool,
    ) -> Response {
        let Some(At::Root { id, path, .. }) = self.locate(engine, rel).await else {
            return StatusCode::FORBIDDEN.into_response();
        };
        let Some(dest) = headers
            .get("destination")
            .and_then(|d| d.to_str().ok())
            .and_then(|d| destination_rel(d, mount))
        else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let Some(At::Root {
            id: to_id,
            path: to_path,
            ..
        }) = self.locate(engine, &dest).await
        else {
            return StatusCode::CONFLICT.into_response();
        };
        if to_id != id || to_path.is_empty() || path.is_empty() {
            return StatusCode::BAD_GATEWAY.into_response();
        }
        match engine.relocate(id, &path, &to_path, remove).await {
            Ok(0) => StatusCode::NOT_FOUND.into_response(),
            Ok(_) => {
                engine.pass(id).await;
                StatusCode::CREATED.into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }

    fn made_under(&self, place: &str, path: &str) -> Vec<String> {
        let prefix = if path.is_empty() {
            format!("{place}/")
        } else {
            format!("{place}/{path}/")
        };
        self.made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(|m| m.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(str::to_owned)
            .collect()
    }

    fn is_made(&self, place: &str, path: &str) -> bool {
        self.made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&format!("{place}/{path}"))
    }

    fn unmake(&self, place: &str, path: &str) {
        let mut made = self.made.lock().unwrap_or_else(PoisonError::into_inner);
        let full = format!("{place}/{path}");
        made.retain(|m| !full.starts_with(&format!("{m}/")));
    }
}

fn options() -> Response {
    let mut r = StatusCode::OK.into_response();
    let h = r.headers_mut();
    h.insert("dav", HeaderValue::from_static("1, 2"));
    h.insert("ms-author-via", HeaderValue::from_static("DAV"));
    h.insert(
        header::ALLOW,
        HeaderValue::from_static(
            "OPTIONS, PROPFIND, PROPPATCH, GET, HEAD, PUT, MKCOL, DELETE, MOVE, COPY, LOCK, UNLOCK",
        ),
    );
    r
}

fn lock(rel: &str) -> Response {
    let token = format!(
        "opaquelocktoken:{}",
        Uuid::new_v5(&Uuid::NAMESPACE_URL, rel.as_bytes())
    );
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock>\
         <D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope><D:depth>0</D:depth>\
         <D:timeout>Second-3600</D:timeout><D:locktoken><D:href>{token}</D:href></D:locktoken>\
         </D:activelock></D:lockdiscovery></D:prop>"
    );
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml; charset=utf-8"),
    );
    if let Ok(v) = HeaderValue::from_str(&format!("<{token}>")) {
        h.insert("lock-token", v);
    }
    r
}

fn multistatus(entries: &str) -> Response {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:multistatus xmlns:D=\"DAV:\">{entries}</D:multistatus>"
    );
    let mut r = (StatusCode::MULTI_STATUS, body).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml; charset=utf-8"),
    );
    r
}

/// One `<D:response>`: a folder, or a file of `(size, at_us, content)`.
fn entry(href: &str, name: &str, file: Option<(u64, i64, Option<&str>)>) -> String {
    let name = xml_escape(name);
    let props = match file {
        None => "<D:resourcetype><D:collection/></D:resourcetype>".to_owned(),
        Some((size, at_us, content)) => {
            let when = UNIX_EPOCH
                .checked_add(Duration::from_micros(u64::try_from(at_us).unwrap_or(0)))
                .unwrap_or(UNIX_EPOCH);
            format!(
                "<D:resourcetype/><D:getcontentlength>{size}</D:getcontentlength>\
                 <D:getlastmodified>{}</D:getlastmodified><D:getetag>\"{}\"</D:getetag>\
                 <D:getcontenttype>{}</D:getcontenttype>",
                httpdate::fmt_http_date(when),
                content.unwrap_or_default(),
                mime_of(&name),
            )
        }
    };
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>{name}</D:displayname>{props}\
         </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        xml_escape(href)
    )
}

fn leaf(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_owned()
    } else {
        format!("{a}/{b}")
    }
}

/// Percent-encode every segment of a path for an href.
fn encode(path: &str) -> String {
    path.split('/')
        .map(|seg| percent_encoding::utf8_percent_encode(seg, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

const SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The in-mount path a `Destination` header names.
fn destination_rel(dest: &str, mount: &str) -> Option<String> {
    let path = match dest.find("://") {
        Some(i) => {
            let after = dest.get(i.saturating_add(3)..)?;
            after.find('/').and_then(|j| after.get(j..))?
        }
        None => dest,
    };
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    let rest = decoded.strip_prefix(mount)?;
    Some(rest.trim_matches('/').to_owned())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A content type by extension, for the few kinds a project holds.
#[must_use]
pub fn mime_of(path: &str) -> &'static str {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "aac" => "audio/mp4",
        "mp4" | "mov" => "video/mp4",
        "md" | "txt" | "rpp" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}
