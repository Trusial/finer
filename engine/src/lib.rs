mod acceleration;
mod checkpoint;
mod encode;
mod media;
mod mp4;
mod output;
mod process;
mod progress;
mod settings;

pub use checkpoint::PassCheckpoint;
pub use encode::{convert, convert_with_retry, Conversion};
pub use media::{inspect, FrameSize, Media};
pub use output::{plan_outputs, AudioPatch, Output};
pub use progress::Event;
pub use settings::{RenderMode, Settings, Theme};

use serde::Serialize;
use std::{env, io, path::PathBuf};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub code: &'static str,
    pub message: &'static str,
    pub detail: String,
}

impl Error {
    pub fn new(code: &'static str, message: &'static str, detail: impl ToString) -> Self {
        Self {
            code,
            message,
            detail: detail.to_string(),
        }
    }

    pub fn io(error: io::Error, code: &'static str, message: &'static str) -> Self {
        match error.kind() {
            io::ErrorKind::StorageFull => {
                Self::new("diskFull", "There is not enough disk space.", error)
            }
            io::ErrorKind::PermissionDenied => Self::new(
                "permissionDenied",
                "Access was denied. Check the file or folder permissions.",
                error,
            ),
            _ => Self::new(code, message, error),
        }
    }

    pub fn cancelled() -> Self {
        Self::new("cancelled", "Conversion cancelled.", "")
    }
}

#[derive(Clone)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl Tools {
    pub fn locate(settings: &Settings) -> Result<Self, Error> {
        let executable = std::env::current_exe()
            .map_err(|e| Error::io(e, "missingTool", "Could not locate the video tools."))?;
        let executable_directory = executable.parent().expect("executable has a parent");
        let suffix = std::env::consts::EXE_SUFFIX;
        let bundled_ffmpeg = executable_directory.join(format!("ffmpeg{suffix}"));
        let bundled_ffprobe = executable_directory.join(format!("ffprobe{suffix}"));
        let ffmpeg = settings.ffmpeg_path.clone().unwrap_or_else(|| {
            if bundled_ffmpeg.is_file() {
                bundled_ffmpeg.clone()
            } else {
                find_on_path(&format!("ffmpeg{suffix}")).unwrap_or_else(|| bundled_ffmpeg.clone())
            }
        });
        let sibling = ffmpeg.with_file_name(format!("ffprobe{suffix}"));
        let ffprobe = if sibling.is_file() {
            sibling
        } else if settings.ffmpeg_path.is_none() {
            find_on_path(&format!("ffprobe{suffix}")).unwrap_or(bundled_ffprobe)
        } else {
            bundled_ffprobe
        };
        for path in [&ffmpeg, &ffprobe] {
            let meta = std::fs::metadata(path).map_err(|e| {
                Error::io(
                    e,
                    "missingTool",
                    "FFmpeg or FFprobe is missing. Extract the complete portable ZIP or choose FFmpeg in Settings.",
                )
            })?;
            if !meta.is_file() {
                return Err(Error::new(
                    "missingTool",
                    "Choose a valid FFmpeg executable.",
                    path.display(),
                ));
            }
        }
        Ok(Self { ffmpeg, ffprobe })
    }
}

fn find_on_path(file_name: &str) -> Option<PathBuf> {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(file_name))
        .find(|candidate| candidate.is_file())
}
