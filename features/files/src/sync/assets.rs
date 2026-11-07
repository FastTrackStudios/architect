//! Asset libraries: `<org>/Assets/<kind>`, each child an asset.
//!
//! An asset is a folder holding a note of its own name
//! (`Praise - Elevation Worship/Praise - Elevation Worship.md`) or a note
//! on its own (`Setlists/Testing.md`). The note's frontmatter is its
//! properties (`type: song`, `artist`, `key`), its `stems` (or else the
//! sound files in its folder) its media, and its `[[links]]` what it
//! references: a setlist's songs, a project's tracks. Search covers every
//! root a device knows; peaks are a waveform per file, kept by content.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use architect::blob::Sha256;
use uuid::Uuid;

use super::engine::{Engine, EngineError};
use super::manifest::{Section, Tree};
use crate::rendition::{PeakReducer, Peaks, RenditionKind};
use crate::roots::{Asset, FileVersion, Hit, Media};
use crate::server::Transcoder;

const SOUND: &[&str] = &["wav", "ogg", "mp3", "flac", "m4a", "aif", "aiff", "opus"];

fn ext(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Whether a path is a sound file.
#[must_use]
pub fn is_sound(path: &str) -> bool {
    SOUND.contains(&ext(path).as_str())
}

/// A note's frontmatter, as flat properties and lists of maps.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    pub properties: BTreeMap<String, String>,
    pub lists: BTreeMap<String, Vec<BTreeMap<String, String>>>,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(s)
        .to_owned()
}

/// The frontmatter of `text` (the YAML a vault note opens with: scalars,
/// lists of scalars, lists of maps), and the body after it.
#[must_use]
pub fn frontmatter(text: &str) -> (Frontmatter, &str) {
    let mut out = Frontmatter::default();
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (out, text);
    };
    let Some((yaml, body)) = rest.split_once("\n---") else {
        return (out, text);
    };
    let body = body.trim_start_matches(['\r', '\n']);
    let mut key: Option<String> = None;
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = line.len() != trimmed.len();
        if let Some(item) = trimmed.strip_prefix("- ") {
            let Some(k) = &key else { continue };
            if let Some((ik, iv)) = item.split_once(": ") {
                out.lists
                    .entry(k.clone())
                    .or_default()
                    .push(BTreeMap::from([(ik.trim().to_owned(), unquote(iv))]));
            } else {
                let joined = out.properties.entry(k.clone()).or_default();
                if !joined.is_empty() {
                    joined.push_str(", ");
                }
                joined.push_str(&unquote(item));
            }
        } else if indented {
            if let (Some(k), Some((ik, iv))) = (&key, trimmed.split_once(':'))
                && let Some(last) = out.lists.get_mut(k).and_then(|l| l.last_mut())
            {
                last.insert(ik.trim().to_owned(), unquote(iv));
            }
        } else if let Some((k, v)) = trimmed.split_once(':') {
            let k = k.trim().to_owned();
            if !v.trim().is_empty() {
                out.properties.insert(k.clone(), unquote(v));
            }
            key = Some(k);
        }
    }
    (out, body)
}

/// The `[[links]]` in `body`, their targets only.
#[must_use]
pub fn links(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some((_, after)) = rest.split_once("[[") {
        let Some((inside, next)) = after.split_once("]]") else {
            break;
        };
        let target = inside.split('|').next().unwrap_or_default().trim();
        if !target.is_empty() {
            out.push(target.to_owned());
        }
        rest = next;
    }
    out
}

/// A library's assets from its tree; `note` reads a note's text.
pub fn assets_of(
    place: &str,
    kind: &str,
    tree: &Tree,
    note: impl Fn(&FileVersion) -> Option<String>,
) -> Vec<Asset> {
    let live = tree.live();
    let mut by_name: BTreeMap<String, Asset> = BTreeMap::new();
    for (path, v) in &live {
        let (top, rest) = path
            .split_once('/')
            .map_or((path.as_str(), None), |(t, r)| (t, Some(r)));
        let (name, note_path) = match rest {
            // A note on its own: `Testing.md`.
            None if ext(top) == "md" => {
                (top.trim_end_matches(".md").to_owned(), Some(path.clone()))
            }
            None => continue,
            Some(rest) if rest == format!("{top}.md") => (top.to_owned(), Some(path.clone())),
            Some(_) => (top.to_owned(), None),
        };
        let asset = by_name.entry(name.clone()).or_insert_with(|| Asset {
            root: place.to_owned(),
            kind: kind.to_owned(),
            folder: if rest.is_some() {
                top.to_owned()
            } else {
                String::new()
            },
            name: name.clone(),
            note: None,
            properties: BTreeMap::new(),
            media: Vec::new(),
            links: Vec::new(),
            at_us: 0,
        });
        asset.at_us = asset.at_us.max(v.at_us);
        if let Some(note_path) = note_path
            && let Some(text) = note(v)
        {
            let (fm, body) = frontmatter(&text);
            asset.note = Some(note_path);
            asset.properties = fm.properties;
            asset.links = links(body);
            if let Some(stems) = fm.lists.get("stems") {
                let base = if asset.folder.is_empty() {
                    String::new()
                } else {
                    format!("{}/", asset.folder)
                };
                for stem in stems {
                    let Some(rel) = stem.get("path") else {
                        continue;
                    };
                    let full = format!("{base}{rel}");
                    asset.media.push(Media {
                        name: stem.get("name").cloned().unwrap_or_else(|| rel.clone()),
                        size: live.get(&full).map_or(0, |v| v.size),
                        path: full,
                    });
                }
            }
        }
    }
    // Without stems, an asset's media are the sound files in its folder.
    for asset in by_name.values_mut() {
        if asset.media.is_empty() && !asset.folder.is_empty() {
            let prefix = format!("{}/", asset.folder);
            for (path, v) in &live {
                if path.starts_with(&prefix) && is_sound(path) {
                    asset.media.push(Media {
                        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
                        path: path.clone(),
                        size: v.size,
                    });
                }
            }
        }
    }
    by_name.into_values().collect()
}

/// A WAV's peaks, read as it streams (16, 24 or 32-bit PCM, 32-bit float).
///
/// # Errors
///
/// It is not a WAV this reads, or cannot be read.
pub fn wav_peaks(path: &Path) -> std::io::Result<Peaks> {
    let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_owned());
    let mut f = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut head = [0u8; 12];
    f.read_exact(&mut head)?;
    if head.get(..4) != Some(b"RIFF") || head.get(8..) != Some(b"WAVE") {
        return Err(bad("not a WAV"));
    }
    let mut format = None;
    loop {
        let mut chunk = [0u8; 8];
        f.read_exact(&mut chunk)?;
        let (id, len) = chunk.split_at(4);
        let len = u32::from_le_bytes(len.try_into().map_err(|_| bad("a short chunk header"))?);
        if id == b"data" {
            break;
        }
        let padded = u64::from(len).saturating_add(u64::from(len & 1));
        if id == b"fmt " {
            let mut fmt = vec![0u8; usize::try_from(padded).map_err(|_| bad("a huge fmt chunk"))?];
            f.read_exact(&mut fmt)?;
            format = Some(WavFormat::of(&fmt).ok_or_else(|| bad("an unsupported sample format"))?);
        } else {
            std::io::copy(&mut (&mut f).take(padded), &mut std::io::sink())?;
        }
    }
    let format = format.ok_or_else(|| bad("no fmt chunk before the data"))?;
    let frame = format.frame();
    let mut reducer = PeakReducer::new(format.rate);
    let mut buf = vec![0u8; frame.saturating_mul(4096)];
    let mut mono = Vec::with_capacity(4096);
    loop {
        let n = f.read(&mut buf)?;
        let whole = buf
            .get(..n.saturating_sub(n.checked_rem(frame).unwrap_or(0)))
            .unwrap_or_default();
        if whole.is_empty() {
            break;
        }
        mono.clear();
        mono.extend(whole.chunks_exact(frame).map(|fr| format.first_sample(fr)));
        reducer.push(&mono);
    }
    Ok(reducer.finish())
}

/// What a WAV's `fmt ` chunk says, as far as peaks need.
struct WavFormat {
    channels: usize,
    rate: u32,
    /// Bytes per sample: 2, 3 or 4.
    width: usize,
    float: bool,
}

impl WavFormat {
    fn of(fmt: &[u8]) -> Option<Self> {
        let word = |i: usize| {
            Some(u16::from_le_bytes([
                *fmt.get(i)?,
                *fmt.get(i.checked_add(1)?)?,
            ]))
        };
        let tag = word(0)?;
        let channels = usize::from(word(2)?.max(1));
        let rate = u32::from_le_bytes([*fmt.get(4)?, *fmt.get(5)?, *fmt.get(6)?, *fmt.get(7)?]);
        let width = usize::from(word(14)? / 8);
        let float = tag == 3 || (tag == 0xFFFE && word(24) == Some(3));
        (rate > 0 && (2..=4).contains(&width)).then_some(Self {
            channels,
            rate,
            width,
            float,
        })
    }

    const fn frame(&self) -> usize {
        self.width.saturating_mul(self.channels)
    }

    /// The first channel's sample of `frame`, as 16 bits.
    fn first_sample(&self, frame: &[u8]) -> i16 {
        let byte = |i: usize| frame.get(i).copied().unwrap_or(0);
        match (self.width, self.float) {
            (4, true) => {
                let v = f32::from_le_bytes([byte(0), byte(1), byte(2), byte(3)]).clamp(-1.0, 1.0);
                // In range by the clamp: a float in [-1, 1] scaled to i16.
                #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                let sample = (v * 32767.0) as i16;
                sample
            }
            (2, _) => i16::from_le_bytes([byte(0), byte(1)]),
            (3, _) => i16::from_le_bytes([byte(1), byte(2)]),
            _ => i16::from_le_bytes([byte(2), byte(3)]),
        }
    }
}

impl Engine {
    /// A library's assets (its notes fetched if they are not here).
    ///
    /// # Errors
    ///
    /// The root is not known, or is not a library.
    pub async fn assets(&self, doc_id: Uuid) -> Result<Vec<Asset>, EngineError> {
        let meta = self
            .meta(doc_id)
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        if meta.section != Section::Assets {
            return Err(EngineError::NoRoot(format!(
                "{} is not a library",
                meta.place()
            )));
        }
        let tree = self.tree(doc_id).await?;
        let mut notes = BTreeMap::new();
        for (path, v) in tree.live() {
            if ext(&path) == "md"
                && let Some(file) = self.local_file(&v).await
            {
                notes.insert(v.id, std::fs::read_to_string(file).unwrap_or_default());
            }
        }
        Ok(assets_of(&meta.place(), &meta.name, &tree, |v| {
            notes.get(&v.id).cloned()
        }))
    }

    /// The file on this device holding a version's bytes, fetched if need be.
    pub async fn local_file(&self, v: &FileVersion) -> Option<PathBuf> {
        let hash = Sha256::from_hex(v.content.as_deref()?).ok()?;
        if let Some(file) = self.store.file_for(&hash) {
            return Some(file);
        }
        self.fetcher.fetch(hash).await.ok()?;
        self.store.file_for(&hash)
    }

    /// Search every root this device knows: projects by name, assets by
    /// name and property, files by path. Every word must match.
    pub async fn search(&self, query: &str) -> Vec<Hit> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let matches = |text: &str| {
            let text = text.to_lowercase();
            words.iter().all(|w| text.contains(w))
        };
        let mut hits = Vec::new();
        for root in self.status().await {
            let Ok(doc_id) = root.doc_id.parse::<Uuid>() else {
                continue;
            };
            if root.section == Section::Projects && matches(&root.name) {
                hits.push(Hit {
                    root: root.place.clone(),
                    path: String::new(),
                    name: root.name.clone(),
                    what: "project".to_owned(),
                    detail: format!("{} files", root.files),
                });
            }
            if root.section == Section::Assets
                && let Ok(assets) = self.assets(doc_id).await
            {
                for a in assets {
                    let props: Vec<String> = a.properties.values().cloned().collect();
                    if matches(&format!("{} {}", a.name, props.join(" "))) {
                        hits.push(Hit {
                            root: root.place.clone(),
                            path: a.note.clone().unwrap_or_else(|| a.folder.clone()),
                            name: a.name.clone(),
                            what: a
                                .properties
                                .get("type")
                                .cloned()
                                .unwrap_or_else(|| "asset".to_owned()),
                            detail: props.join(" · "),
                        });
                    }
                }
            }
            if let Ok(tree) = self.tree(doc_id).await {
                for (path, v) in tree.live() {
                    if matches(&path) {
                        hits.push(Hit {
                            root: root.place.clone(),
                            name: path.rsplit('/').next().unwrap_or(&path).to_owned(),
                            path,
                            what: "file".to_owned(),
                            detail: format!("{} bytes", v.size),
                        });
                    }
                }
            }
            if hits.len() >= 200 {
                break;
            }
        }
        hits.truncate(200);
        hits
    }

    /// A file's waveform: a WAV read here, anything else through the
    /// engine's transcoder; kept by content.
    ///
    /// # Errors
    ///
    /// No such file, its bytes cannot be had, or no transcoder reads it.
    pub async fn peaks(&self, doc_id: Uuid, path: &str) -> Result<Peaks, EngineError> {
        let v = self
            .tree(doc_id)
            .await?
            .current(path)
            .ok_or_else(|| EngineError::NoRoot(path.to_owned()))?;
        let content = v.content.clone().unwrap_or_default();
        let cache = self
            .config()
            .data
            .join("peaks")
            .join(format!("{content}.json"));
        if let Some(peaks) = std::fs::read_to_string(&cache)
            .ok()
            .and_then(|s| facet_json::from_str::<Peaks>(&s).ok())
        {
            return Ok(peaks);
        }
        let file = self
            .local_file(&v)
            .await
            .ok_or_else(|| EngineError::Io(format!("the bytes of {path} are not reachable")))?;
        let peaks = if ext(path) == "wav" {
            tokio::task::spawn_blocking(move || wav_peaks(&file))
                .await
                .map_err(|e| EngineError::Io(e.to_string()))??
        } else {
            let transcoder = self
                .transcoder()
                .ok_or_else(|| EngineError::Io(format!("no transcoder for .{}", ext(path))))?;
            let out = cache.with_extension("tmp.json");
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            transcoder
                .render(RenditionKind::Peaks, &file, &out)
                .await
                .map_err(|e| EngineError::Io(e.to_string()))?;
            let json = std::fs::read_to_string(&out)?;
            let _ = std::fs::remove_file(&out);
            facet_json::from_str(&json).map_err(|e| EngineError::Io(e.to_string()))?
        };
        if let Some(dir) = cache.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&cache, facet_json::to_string(&peaks).unwrap_or_default())?;
        Ok(peaks)
    }

    /// The transcoder this engine makes peaks with (beside WAV), if any.
    #[must_use]
    pub fn transcoder(&self) -> Option<Arc<dyn Transcoder>> {
        self.transcoder.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_note_reads_its_properties_stems_and_links() {
        let text = "---\ntype: song\nartist: Elevation Worship\nkey: A\nstems:\n  - name: \"Click\"\n    path: \"audio/ogg/01 - Click.ogg\"\n  - name: \"Keys\"\n    path: \"audio/ogg/08 - Keys.ogg\"\ntags:\n  - worship\n  - fast\n---\n# Praise\n\n[[Holy Forever - Bethel Music]] and [[Washed|the other]]\n";
        let (fm, body) = frontmatter(text);
        assert_eq!(
            fm.properties.get("artist").map(String::as_str),
            Some("Elevation Worship")
        );
        assert_eq!(
            fm.properties.get("tags").map(String::as_str),
            Some("worship, fast")
        );
        let stems = &fm.lists["stems"];
        assert_eq!(stems.len(), 2);
        assert_eq!(stems[1]["path"], "audio/ogg/08 - Keys.ogg");
        assert_eq!(links(body), vec!["Holy Forever - Bethel Music", "Washed"]);
    }

    #[test]
    fn a_wav_reads_to_peaks() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("t.wav");
        let samples: Vec<i16> = (0..8000)
            .map(|i| if i % 2 == 0 { 1000 } else { -1000 })
            .collect();
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(8000u32.to_le_bytes());
        wav.extend(16000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
        wav.extend(data);
        std::fs::write(&p, wav).unwrap();
        let peaks = wav_peaks(&p).unwrap();
        assert!(peaks.length > 0);
        assert_eq!(peaks.data.iter().copied().max(), Some(1000));
        assert_eq!(peaks.data.iter().copied().min(), Some(-1000));
    }
}
