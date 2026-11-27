use crate::{process, Error, Tools};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    format: Format,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Stream {
    index: u32,
    codec_type: String,
    codec_name: String,
    profile: Option<String>,
    width: u32,
    height: u32,
    sample_aspect_ratio: Option<String>,
    avg_frame_rate: String,
    time_base: String,
    r_frame_rate: String,
    duration: Option<String>,
    start_time: Option<String>,
    tags: Tags,
    nb_frames: Option<String>,
    bit_rate: Option<String>,
    sample_rate: Option<String>,
    channel_layout: Option<String>,
    disposition: Disposition,
    pix_fmt: Option<String>,
    level: Option<i32>,
    color_space: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    color_range: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Tags {
    #[serde(rename = "DURATION", alias = "duration")]
    duration: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Disposition {
    attached_pic: u8,
}

#[derive(Debug, Deserialize)]
struct Format {
    format_name: String,
    duration: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    const SUPPORTED: [Self; 3] = [
        Self {
            width: 1920,
            height: 1080,
        },
        Self {
            width: 1080,
            height: 1920,
        },
        Self {
            width: 1080,
            height: 1080,
        },
    ];

    pub(crate) fn is_supported(self) -> bool {
        Self::SUPPORTED.contains(&self)
    }

    fn recommended(self, display_ratio: f64) -> Option<Self> {
        if self.is_supported() {
            return None;
        }
        // At 4:3 and 3:4 the square and oriented frames use the same area for the picture.
        let [landscape, portrait, square] = Self::SUPPORTED;
        Some(if display_ratio >= 4.0 / 3.0 {
            landscape
        } else if display_ratio <= 3.0 / 4.0 {
            portrait
        } else {
            square
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub path: PathBuf,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub suggested_frame_size: Option<FrameSize>,
    pub fps: f64,
    pub duration: f64,
    pub size: u64,
    pub already_patched: bool,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub channel_layout: Option<String>,
    pub container: String,
    pub frames: Option<u64>,
    pub bitrate: Option<u64>,
    pub color_space: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    pub color_range: Option<String>,
    pub(crate) video_index: u32,
    pub(crate) audio_index: Option<u32>,
    pub(crate) max_fps: f64,
    pub(crate) time_base: String,
    pub(crate) profile: Option<String>,
    pub(crate) level: Option<i32>,
    pub(crate) pixel_format: Option<String>,
    pub(crate) audio_profile: Option<String>,
    pub(crate) has_video_duration: bool,
}

fn positive(value: Option<&str>) -> Option<f64> {
    value?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn fps(rate: &str) -> Option<f64> {
    let (num, den) = rate.split_once('/')?;
    let value = positive(Some(num))? / positive(Some(den))?;
    value.is_finite().then_some(value)
}

fn tagged_duration(stream: &Stream) -> Option<f64> {
    let (hours, rest) = stream.tags.duration.as_deref()?.split_once(':')?;
    let (minutes, seconds) = rest.split_once(':')?;
    let end = hours.parse::<u32>().ok()? as f64 * 3600.0
        + minutes.parse::<u32>().ok()? as f64 * 60.0
        + seconds.parse::<f64>().ok()?;
    let duration = end - positive(stream.start_time.as_deref()).unwrap_or(0.0);
    (duration.is_finite() && duration > 0.0).then_some(duration)
}

pub(crate) const LOCAL_FORMATS: &str =
    "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,mpeg,mpegts,mpegvideo,flv,ogg,asf,wmv,rm,ivf,mxf";

pub(crate) async fn probe(
    path: &Path,
    tools: &Tools,
    cancel: &CancellationToken,
) -> Result<Media, Error> {
    if !path.is_absolute() {
        return Err(Error::new(
            "invalidInput",
            "Choose a local video file.",
            path.display(),
        ));
    }
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::io(e, "invalidInput", "The video file could not be read."))?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(Error::new(
            "invalidInput",
            "Choose a readable video file.",
            path.display(),
        ));
    }
    let output = process::capture(
        process::command(&tools.ffprobe)
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                LOCAL_FORMATS,
                "-show_entries",
                "stream=index,codec_type,codec_name,profile,width,height,sample_aspect_ratio,avg_frame_rate,r_frame_rate,time_base,duration,start_time,nb_frames,bit_rate,sample_rate,channel_layout,pix_fmt,level,color_space,color_transfer,color_primaries,color_range:stream_disposition=attached_pic:stream_tags=DURATION:format=format_name,duration",
                "-of",
                "json",
                "-i",
            ])
            .arg(path),
        cancel,
    )
    .await?;
    let detail = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(ffmpeg_error(
            &detail,
            "invalidInput",
            "This video is unreadable or uses an unsupported container.",
        ));
    }
    let data: Probe = serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::new("invalidInput", "Could not read the video information.", e))?;
    let video = data
        .streams
        .iter()
        .find(|s| s.codec_type == "video" && s.disposition.attached_pic == 0)
        .ok_or_else(|| Error::new("invalidInput", "This file has no video stream.", ""))?;
    let audio = data.streams.iter().find(|s| s.codec_type == "audio");
    let frame_rate = fps(&video.avg_frame_rate).or_else(|| fps(&video.r_frame_rate));
    let video_duration = positive(video.duration.as_deref()).or_else(|| tagged_duration(video));
    let duration = video_duration.or_else(|| positive(data.format.duration.as_deref()));
    let (frame_rate, duration) = match (frame_rate, duration) {
        (Some(rate), Some(duration)) if video.width > 0 && video.height > 0 => (rate, duration),
        _ => {
            return Err(Error::new(
                "invalidInput",
                "Could not determine the video size, frame rate or duration.",
                "",
            ))
        }
    };
    let sample_ratio = video
        .sample_aspect_ratio
        .as_deref()
        .and_then(|ratio| ratio.split_once(':'))
        .and_then(|(num, den)| Some(positive(Some(num))? / positive(Some(den))?))
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .unwrap_or(1.0);
    let frame_size = FrameSize {
        width: video.width,
        height: video.height,
    };
    let display_ratio = sample_ratio * f64::from(video.width) / f64::from(video.height);
    Ok(Media {
        path: path.to_owned(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        width: video.width,
        height: video.height,
        suggested_frame_size: frame_size.recommended(display_ratio),
        fps: frame_rate,
        duration,
        size: meta.len(),
        video_codec: video.codec_name.clone(),
        audio_codec: audio.map(|s| s.codec_name.clone()),
        sample_rate: parse(audio.and_then(|s| s.sample_rate.as_deref())),
        channel_layout: audio.and_then(|s| s.channel_layout.clone()),
        container: data.format.format_name,
        frames: parse(video.nb_frames.as_deref()),
        bitrate: parse(video.bit_rate.as_deref()),
        color_space: video.color_space.clone(),
        color_transfer: video.color_transfer.clone(),
        color_primaries: video.color_primaries.clone(),
        color_range: video.color_range.clone(),
        video_index: video.index,
        already_patched: false,
        audio_index: audio.map(|s| s.index),
        max_fps: fps(&video.r_frame_rate)
            .unwrap_or(frame_rate)
            .max(frame_rate),
        time_base: video.time_base.clone(),
        profile: video.profile.clone(),
        level: video.level,
        pixel_format: video.pix_fmt.clone(),
        audio_profile: audio.and_then(|s| s.profile.clone()),
        has_video_duration: video_duration.is_some(),
    })
}

pub(crate) async fn encoder_time_base(
    media: &Media,
    tools: &Tools,
    cancel: &CancellationToken,
) -> Result<&'static str, Error> {
    if media.fps != 60.0 || media.max_fps != 60.0 {
        return Ok("demux");
    }
    let ticks_per_frame = media.time_base.split_once('/').and_then(|(num, den)| {
        let divisor = num.parse::<i64>().ok()?.checked_mul(60)?;
        let ticks = den.parse::<i64>().ok()?;
        (divisor > 0 && ticks >= divisor && ticks % divisor == 0).then(|| ticks / divisor)
    });
    let Some(ticks_per_frame) = ticks_per_frame else {
        return Ok("demux");
    };

    // Match the reference's 60 fps SPS timing only if rounding cannot shift a frame.
    let result = process::capture(
        process::command(&tools.ffprobe)
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                LOCAL_FORMATS,
                "-select_streams",
            ])
            .arg(media.video_index.to_string())
            .args([
                "-show_packets",
                "-show_entries",
                "packet=pts",
                "-of",
                "csv=p=0",
                "-i",
            ])
            .arg(&media.path),
        cancel,
    )
    .await;
    let output = match result {
        Ok(output) if output.status.success() => output.stdout,
        Err(error) if error.code == "cancelled" => return Err(error),
        _ => return Ok("demux"),
    };
    let pts = String::from_utf8_lossy(&output);
    let on_grid = !pts.is_empty()
        && pts.lines().all(|line| {
            line.parse::<i64>()
                .is_ok_and(|ticks| ticks % ticks_per_frame == 0)
        });
    Ok(if on_grid { "1:60" } else { "demux" })
}

fn parse<T: std::str::FromStr>(value: Option<&str>) -> Option<T> {
    value?.parse().ok()
}

pub async fn inspect(
    path: &Path,
    tools: &Tools,
    cancel: &CancellationToken,
) -> Result<Media, Error> {
    let mut media = probe(path, tools, cancel).await?;
    media.already_patched = detect_patch(&media.path, &media.container).await;
    if media.already_patched {
        return Ok(media);
    }
    if media.suggested_frame_size.is_none() {
        crate::encode::level_for(
            FrameSize {
                width: media.width,
                height: media.height,
            },
            media.max_fps,
            crate::settings::DEFAULT_VIDEO_BITRATE_KBPS,
        )?;
    }
    // Catch missing decoders and inputs whose headers probe but whose frames cannot decode.
    let result = process::capture(
        process::command(&tools.ffmpeg)
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-xerror",
                "-nostdin",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                LOCAL_FORMATS,
                "-i",
            ])
            .arg(path)
            .args([
                "-map",
                &format!("0:{}", media.video_index),
                "-frames:v",
                "1",
                "-an",
                "-progress",
                "pipe:1",
                "-f",
                "null",
                null_device(),
            ]),
        cancel,
    )
    .await?;
    let progress = String::from_utf8_lossy(&result.stdout);
    if !result.status.success() || !progress.lines().any(|line| line.trim() == "frame=1") {
        return Err(ffmpeg_error(
            &String::from_utf8_lossy(&result.stderr),
            "invalidInput",
            "Could not decode this video.",
        ));
    }
    Ok(media)
}

pub(crate) async fn detect_patch(path: &Path, container: &str) -> bool {
    if !container.split(',').any(|name| name == "mp4") {
        return false;
    }
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || crate::mp4::is_patched(&path))
        .await
        .unwrap_or(false)
}

pub(crate) fn null_device() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

pub(crate) fn ffmpeg_error(detail: &str, code: &'static str, message: &'static str) -> Error {
    let lower = detail.to_ascii_lowercase();
    let (code, message) = if lower.contains("no space left")
        || lower.contains("not enough space")
        || lower.contains("disk full")
    {
        ("diskFull", "There is not enough disk space.")
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        (
            "permissionDenied",
            "Access was denied. Check the file or folder permissions.",
        )
    } else if lower.contains("decoder not found")
        || lower.contains("unknown decoder")
        || lower.contains("unsupported codec")
    {
        ("unsupportedCodec", "This video uses an unsupported codec.")
    } else if lower.contains("unknown encoder") && lower.contains("libx264") {
        (
            "encoder",
            "This FFmpeg build does not include the libx264 encoder.",
        )
    } else {
        (code, message)
    };
    Error::new(code, message, detail.trim())
}
