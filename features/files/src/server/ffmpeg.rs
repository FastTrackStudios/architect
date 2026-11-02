//! [`FfmpegTranscoder`] (feature `ffmpeg`): renditions by the `ffmpeg`
//! and `ffprobe` on `PATH`, one process per rendition.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use facet::Facet;
use futures_util::future::BoxFuture;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;

use super::renditions::{TranscodeError, Transcoder};
use crate::rendition::{MediaClass, PeakReducer, RenditionKind};

/// The rate a source's sound is read at to make its peaks.
const PEAKS_RATE: u32 = 8000;

/// At most this many lines high, never upscaled, and even (H.264 in
/// 4:2:0 needs both sides even).
const HEIGHT_720: &str = "scale=-2:'trunc(min(720,ih)/2)*2'";

/// How much of a failed run's `stderr` a failure keeps.
const STDERR_KEPT: usize = 600;

/// Renditions by `ffmpeg`: a proxy in H.264/AAC (`+faststart`, at most
/// 720p), a poster from the most representative frame (`thumbnail`), the
/// peaks of the first sound track, and AAC for audio.
#[derive(Debug, Clone)]
pub struct FfmpegTranscoder {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl Default for FfmpegTranscoder {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}

impl FfmpegTranscoder {
    /// `ffmpeg` and `ffprobe` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// These binaries instead.
    #[must_use]
    pub fn with_binaries(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }

    /// Whether `ffmpeg` and `ffprobe` run here.
    pub async fn available(&self) -> bool {
        let runs = |bin: &Path| {
            let mut cmd = Command::new(bin);
            cmd.arg("-version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            async move { cmd.status().await.is_ok_and(|s| s.success()) }
        };
        runs(&self.ffmpeg).await && runs(&self.ffprobe).await
    }

    fn ffmpeg(&self, source: &Path) -> Command {
        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args(["-nostdin", "-hide_banner", "-v", "error", "-y", "-i"])
            .arg(source)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        cmd
    }

    async fn probe_class(&self, source: &Path) -> Result<MediaClass, TranscodeError> {
        let out = Command::new(&self.ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=format_name:stream=codec_type:stream_disposition=attached_pic",
                "-of",
                "json",
            ])
            .arg(source)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| TranscodeError::new(format!("ffprobe: {e}")))?;
        if !out.status.success() {
            // Not something ffprobe can read: not media.
            return Ok(MediaClass::Other);
        }
        let probe: Probe = facet_json::from_str(&String::from_utf8_lossy(&out.stdout))
            .map_err(|e| TranscodeError::new(format!("ffprobe's answer: {e}")))?;
        Ok(probe.class())
    }

    async fn run(&self, kind: RenditionKind, mut cmd: Command) -> Result<(), TranscodeError> {
        let out = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| TranscodeError::new(format!("ffmpeg: {e}")))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(failed(kind, &out.stderr, out.status))
        }
    }

    /// The first sound track, mono at [`PEAKS_RATE`], read from ffmpeg's
    /// stdout and reduced as it arrives.
    async fn peaks(&self, source: &Path, out: &Path) -> Result<(), TranscodeError> {
        let mut cmd = self.ffmpeg(source);
        cmd.args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar"])
            .arg(PEAKS_RATE.to_string())
            .args(["-f", "s16le", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| TranscodeError::new(format!("ffmpeg: {e}")))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| TranscodeError::new("ffmpeg gave no output"))?;
        let mut reducer = PeakReducer::new(PEAKS_RATE);
        let mut buf = vec![0_u8; 64 * 1024];
        let mut carry: Option<u8> = None;
        loop {
            let n = stdout
                .read(&mut buf)
                .await
                .map_err(|e| TranscodeError::new(format!("ffmpeg's output: {e}")))?;
            let Some(read) = buf.get(..n).filter(|r| !r.is_empty()) else {
                break;
            };
            // Keep samples whole across reads.
            let mut run = Vec::with_capacity(read.len().saturating_add(1));
            run.extend(carry.take());
            run.extend_from_slice(read);
            if run.len() % 2 == 1 {
                carry = run.pop();
            }
            reducer.push_pcm_s16le(&run);
        }
        let done = child
            .wait_with_output()
            .await
            .map_err(|e| TranscodeError::new(format!("ffmpeg: {e}")))?;
        if !done.status.success() {
            return Err(failed(RenditionKind::Peaks, &done.stderr, done.status));
        }
        let json = facet_json::to_string(&reducer.finish())
            .map_err(|e| TranscodeError::new(e.to_string()))?;
        tokio::fs::write(out, json)
            .await
            .map_err(|e| TranscodeError::new(format!("writing the peaks: {e}")))
    }
}

impl Transcoder for FfmpegTranscoder {
    fn probe<'a>(&'a self, source: &'a Path) -> BoxFuture<'a, Result<MediaClass, TranscodeError>> {
        Box::pin(self.probe_class(source))
    }

    fn render<'a>(
        &'a self,
        kind: RenditionKind,
        source: &'a Path,
        out: &'a Path,
    ) -> BoxFuture<'a, Result<(), TranscodeError>> {
        Box::pin(async move {
            let mut cmd = self.ffmpeg(source);
            match kind {
                RenditionKind::Peaks => return self.peaks(source, out).await,
                RenditionKind::Proxy => {
                    cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-vf", HEIGHT_720])
                        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "23"])
                        .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "160k"])
                        .args(["-movflags", "+faststart"]);
                }
                RenditionKind::Poster => {
                    cmd.args(["-map", "0:v:0", "-vf"])
                        .arg(format!("thumbnail,{HEIGHT_720}"))
                        .args(["-frames:v", "1", "-q:v", "3"]);
                }
                RenditionKind::Audio => {
                    cmd.args(["-map", "0:a:0", "-vn", "-c:a", "aac", "-b:a", "192k"])
                        .args(["-movflags", "+faststart"]);
                }
            }
            cmd.arg(out);
            self.run(kind, cmd).await
        })
    }
}

fn failed(kind: RenditionKind, stderr: &[u8], status: std::process::ExitStatus) -> TranscodeError {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    let start = text
        .char_indices()
        .rev()
        .nth(STDERR_KEPT)
        .map_or(0, |(i, _)| i);
    TranscodeError::new(format!(
        "ffmpeg ({kind}) exited {status}: {}",
        text.get(start..).unwrap_or(text)
    ))
}

/// The part of `ffprobe -of json` the class is read from.
#[derive(Debug, Default, Facet)]
struct Probe {
    #[facet(default)]
    streams: Vec<Stream>,
    #[facet(default)]
    format: Format,
}

#[derive(Debug, Default, Facet)]
struct Stream {
    #[facet(default)]
    codec_type: String,
    #[facet(default)]
    disposition: Disposition,
}

#[derive(Debug, Default, Facet)]
struct Disposition {
    #[facet(default)]
    attached_pic: u8,
}

#[derive(Debug, Default, Facet)]
struct Format {
    #[facet(default)]
    format_name: String,
}

impl Probe {
    /// A picture that moves is video (cover art on a song is not, nor is
    /// a still image); otherwise sound is audio.
    fn class(&self) -> MediaClass {
        let still = self.format.format_name.contains("image2")
            || self.format.format_name.ends_with("_pipe");
        let moving = self
            .streams
            .iter()
            .any(|s| s.codec_type == "video" && s.disposition.attached_pic == 0);
        if moving && !still {
            MediaClass::Video
        } else if self.streams.iter().any(|s| s.codec_type == "audio") {
            MediaClass::Audio
        } else {
            MediaClass::Other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(json: &str) -> MediaClass {
        facet_json::from_str::<Probe>(json).unwrap().class()
    }

    #[test]
    fn the_class_is_read_from_the_streams() {
        let video = r#"{"streams":[{"codec_type":"video","disposition":{"attached_pic":0}},
            {"codec_type":"audio","disposition":{"attached_pic":0}}],
            "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}}"#;
        assert_eq!(class(video), MediaClass::Video);
        let song_with_art = r#"{"streams":[{"codec_type":"audio","disposition":{"attached_pic":0}},
            {"codec_type":"video","disposition":{"attached_pic":1}}],"format":{"format_name":"mp3"}}"#;
        assert_eq!(class(song_with_art), MediaClass::Audio);
        let still = r#"{"streams":[{"codec_type":"video","disposition":{"attached_pic":0}}],
            "format":{"format_name":"png_pipe"}}"#;
        assert_eq!(class(still), MediaClass::Other);
        assert_eq!(class(r#"{"streams":[],"format":{}}"#), MediaClass::Other);
    }
}
