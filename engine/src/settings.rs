use crate::Error;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) const DEFAULT_VIDEO_BITRATE_KBPS: u32 = 16_617;
const VIDEO_BITRATE_STEPS: u32 = 7;
const MAX_VIDEO_BITRATE_KBPS: u32 = DEFAULT_VIDEO_BITRATE_KBPS * VIDEO_BITRATE_STEPS;

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RenderMode {
    #[default]
    Quality,
    FrameBlend,
    MotionBlur,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    pub output_directory: Option<PathBuf>,
    pub use_source_directory: bool,
    pub threads: u16,
    pub gpu_decoding: bool,
    pub gpu_encoding: bool,
    pub video_bitrate_kbps: u32,
    pub open_folder_after_conversion: bool,
    pub keep_logs: bool,
    pub ffmpeg_path: Option<PathBuf>,
    pub render_mode: RenderMode,
    pub target_fps: u32,
    pub blur_samples: u8,
    pub color_management: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            output_directory: None,
            use_source_directory: true,
            threads: 16,
            gpu_decoding: true,
            gpu_encoding: true,
            video_bitrate_kbps: DEFAULT_VIDEO_BITRATE_KBPS,
            open_folder_after_conversion: false,
            keep_logs: false,
            ffmpeg_path: None,
            render_mode: RenderMode::Quality,
            target_fps: 60,
            blur_samples: 4,
            color_management: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if !(1..=256).contains(&settings.threads) {
            settings.threads = 16;
        }
        let step = settings
            .video_bitrate_kbps
            .saturating_add(DEFAULT_VIDEO_BITRATE_KBPS / 2)
            / DEFAULT_VIDEO_BITRATE_KBPS;
        settings.video_bitrate_kbps =
            step.clamp(1, VIDEO_BITRATE_STEPS) * DEFAULT_VIDEO_BITRATE_KBPS;
        let has_valid_output_directory = settings
            .output_directory
            .as_ref()
            .is_some_and(|path| path.is_absolute());
        if !has_valid_output_directory {
            settings.output_directory = None;
            settings.use_source_directory = true;
        }
        let has_valid_ffmpeg_path = settings
            .ffmpeg_path
            .as_ref()
            .is_some_and(|path| path.is_absolute());
        if !has_valid_ffmpeg_path {
            settings.ffmpeg_path = None;
        }
        settings.target_fps = settings.target_fps.clamp(24, 240);
        settings.blur_samples = settings.blur_samples.clamp(2, 16);
        settings
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        self.validate_video_bitrate()?;
        if !(1..=256).contains(&self.threads) {
            return Err(Error::new(
                "settings",
                "Enter a thread count from 1 to 256.",
                "",
            ));
        }
        let has_valid_output_directory = self.use_source_directory
            || self
                .output_directory
                .as_ref()
                .is_some_and(|path| path.is_absolute() && path.is_dir());
        if !has_valid_output_directory {
            return Err(Error::new(
                "outputDirectory",
                "Choose an existing output folder.",
                "",
            ));
        }
        let has_valid_ffmpeg_path = self
            .ffmpeg_path
            .as_ref()
            .is_none_or(|path| path.is_absolute() && path.is_file());
        if !has_valid_ffmpeg_path {
            return Err(Error::new(
                "missingTool",
                "Choose a valid FFmpeg executable.",
                "",
            ));
        }
        if !(24..=240).contains(&self.target_fps) {
            return Err(Error::new(
                "settings",
                "Choose a target FPS from 24 to 240.",
                "",
            ));
        }
        if !(2..=16).contains(&self.blur_samples) {
            return Err(Error::new(
                "settings",
                "Choose 2 to 16 motion blur samples.",
                "",
            ));
        }
        self.write(path)
    }

    pub(crate) fn validate_video_bitrate(&self) -> Result<(), Error> {
        if (DEFAULT_VIDEO_BITRATE_KBPS..=MAX_VIDEO_BITRATE_KBPS).contains(&self.video_bitrate_kbps)
            && self
                .video_bitrate_kbps
                .is_multiple_of(DEFAULT_VIDEO_BITRATE_KBPS)
        {
            Ok(())
        } else {
            Err(Error::new(
                "settings",
                "Choose one of the seven bitrate options.",
                "",
            ))
        }
    }

    fn write(&self, path: &Path) -> Result<(), Error> {
        let directory = path.parent().expect("settings path has a parent");
        let write = || -> std::io::Result<()> {
            fs::create_dir_all(directory)?;
            let mut temp = tempfile::NamedTempFile::new_in(directory)?;
            serde_json::to_writer_pretty(temp.as_file_mut(), self)?;
            temp.write_all(b"\n")?;
            temp.as_file().sync_all()?;
            temp.persist(path).map_err(|e| e.error)?;
            Ok(())
        };
        write().map_err(|e| Error::io(e, "settings", "Could not save settings."))
    }
}
