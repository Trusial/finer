use crate::{Conversion, FrameSize, RenderMode};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

pub struct PassCheckpoint {
    pub(crate) directory: TempDir,
    pub(crate) frames: u64,
    pub(crate) speed: Option<f64>,
    pub(crate) key: CheckpointKey,
}

#[derive(PartialEq, Eq)]
pub(crate) struct CheckpointKey {
    source: FileDigest,
    encoder: FileDigest,
    frame_size: Option<FrameSize>,
    bitrate: u32,
    threads: u16,
    time_base: String,
    pub(crate) decoder: Option<&'static str>,
    gpu_encoding: bool,
    gpu_decoding: bool,
    render_mode: RenderMode,
    target_fps: u32,
    blur_samples: u8,
    shadow_lift: bool,
}

impl CheckpointKey {
    pub(crate) async fn read(
        job: &Conversion,
        time_base: &str,
        decoder: Option<&'static str>,
        cancel: &CancellationToken,
    ) -> Option<Self> {
        let source = job.source.clone();
        let encoder = job.tools.ffmpeg.clone();
        let source_stamp = stamp(&source).ok()?;
        let encoder_stamp = stamp(&encoder).ok()?;
        let settings = job.settings.clone();
        let frame_size = job.frame_size;
        let time_base = time_base.to_owned();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            Some(Self {
                source: FileDigest::read(source, source_stamp, &cancel).ok()?,
                encoder: FileDigest::read(encoder, encoder_stamp, &cancel).ok()?,
                frame_size,
                bitrate: settings.video_bitrate_kbps,
                threads: settings.threads,
                time_base,
                decoder,
                gpu_encoding: settings.gpu_encoding,
                gpu_decoding: settings.gpu_decoding,
                render_mode: settings.render_mode,
                target_fps: settings.target_fps,
                blur_samples: settings.blur_samples,
                shadow_lift: settings.shadow_lift,
            })
        })
        .await
        .ok()
        .flatten()
    }

    pub(crate) fn files_unchanged(&self) -> bool {
        self.source.unchanged() && self.encoder.unchanged()
    }
}

#[derive(PartialEq, Eq)]
struct FileDigest {
    path: PathBuf,
    stamp: (u64, SystemTime),
    sha256: [u8; 32],
}

impl FileDigest {
    fn read(
        path: PathBuf,
        before: (u64, SystemTime),
        cancel: &CancellationToken,
    ) -> io::Result<Self> {
        let mut file = File::open(&path)?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
        if stamp(&path)? != before {
            return Err(io::Error::other(
                "The file changed while it was being read.",
            ));
        }
        Ok(Self {
            path,
            stamp: before,
            sha256: hash.finalize().into(),
        })
    }

    fn unchanged(&self) -> bool {
        stamp(&self.path).is_ok_and(|current| current == self.stamp)
    }
}

fn stamp(path: &Path) -> io::Result<(u64, SystemTime)> {
    let metadata = fs::metadata(path)?;
    Ok((metadata.len(), metadata.modified()?))
}
