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
use std::{io, path::PathBuf};

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
        let ffmpeg = settings
            .ffmpeg_path
            .clone()
            .unwrap_or_else(|| executable_directory.join(format!("ffmpeg{suffix}")));
        let sibling = ffmpeg.with_file_name(format!("ffprobe{suffix}"));
        let ffprobe = if sibling.is_file() {
            sibling
        } else {
            executable_directory.join(format!("ffprobe{suffix}"))
        };
        for path in [&ffmpeg, &ffprobe] {
            let meta = std::fs::metadata(path).map_err(|e| {
                Error::io(
                    e,
                    "missingTool",
                    "FFmpeg or FFprobe is missing. Keep both tools beside Finer.",
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
