use crate::{Error, FrameSize, Media, Settings};
use serde::Serialize;
use std::{
    collections::HashSet,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};
use tempfile::TempPath;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub directory: PathBuf,
    pub audio_patch: AudioPatch,
    pub encoder: &'static str,
    pub decoder: &'static str,
    pub color_adjusted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioPatch {
    Applied,
    NoAudio,
    NeedsStereo,
    TooShort,
}

pub fn plan_outputs(sources: &[PathBuf], settings: &Settings) -> Vec<Option<PathBuf>> {
    let mut reserved = HashSet::new();
    sources
        .iter()
        .map(|source| {
            let directory = output_directory(source, None, settings)?;
            (1..=10_000).find_map(|number| {
                let path = output_path(source, directory, number);
                #[cfg(windows)]
                let key = path.to_string_lossy().to_lowercase();
                #[cfg(not(windows))]
                let key = path.clone();
                (path.symlink_metadata().is_err() && reserved.insert(key)).then_some(path)
            })
        })
        .collect()
}

pub(crate) fn output_directory<'a>(
    source: &'a Path,
    requested: Option<&'a Path>,
    settings: &'a Settings,
) -> Option<&'a Path> {
    let configured = if settings.use_source_directory {
        None
    } else {
        settings.output_directory.as_deref()
    };
    requested.or(configured).or_else(|| source.parent())
}

pub(crate) fn validate(
    input: &Media,
    output: &Media,
    frame_size: FrameSize,
    encoded_frames: u64,
    level: &str,
    gpu: bool,
    expected_fps: f64,
) -> Result<(), Error> {
    // A container may estimate VFR frame rate from the span between its first and last timestamps.
    let rate_tolerance = (expected_fps * (2.0 / encoded_frames as f64).max(0.01)).max(0.02);
    let duration_tolerance = (2.0 / input.fps).max(0.12);
    let required_level = level
        .replace('.', "")
        .parse::<i32>()
        .expect("known H.264 level");
    let valid_level = output.level.is_some_and(|level| {
        if gpu {
            level >= required_level
        } else {
            level == required_level
        }
    });
    if output.video_codec != "h264"
        || output.profile.as_deref() != Some("High")
        || !valid_level
        || output.pixel_format.as_deref() != Some("yuv420p")
        || !output.container.split(',').any(|format| format == "mp4")
        || (output.width, output.height) != (frame_size.width, frame_size.height)
        || output.frames != Some(encoded_frames)
        || (expected_fps - output.fps).abs() > rate_tolerance
        || ((input.has_video_duration || input.audio_index.is_none())
            && (input.duration - output.duration).abs() > duration_tolerance)
        || (input.audio_index.is_some()
            && (output.audio_codec.as_deref() != Some("aac")
                || output.audio_profile.as_deref() != Some("LC")))
        || (input.audio_index.is_none() && output.audio_index.is_some())
    {
        return Err(Error::new(
            "invalidOutput",
            "The completed video did not pass validation.",
            format!("Input: {input:?}\nOutput: {output:?}\nEncoded frames: {encoded_frames}"),
        ));
    }
    Ok(())
}

fn output_path(input: &Path, directory: &Path, attempt: u32) -> PathBuf {
    let stem = input
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("video"));
    let mut name = OsString::from(stem);
    name.push("_finer");
    if attempt > 1 {
        name.push(format!("_{attempt}"));
    }
    name.push(".mp4");
    directory.join(name)
}

pub(crate) fn deliver(
    mut temp: TempPath,
    directory: &Path,
    input: &Path,
) -> Result<PathBuf, Error> {
    for attempt in 1..=10_000 {
        let path = output_path(input, directory, attempt);
        match temp.persist_noclobber(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => temp = error.path,
            Err(error) => {
                return Err(Error::io(
                    error.error,
                    "outputDirectory",
                    "Could not save the completed video.",
                ))
            }
        }
    }
    Err(Error::new(
        "outputDirectory",
        "Could not find an unused output filename.",
        directory.display(),
    ))
}
