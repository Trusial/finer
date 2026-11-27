use crate::{process, Error, Media, Settings, Tools};
use std::path::Path;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const HARDWARE_DECODERS: [&str; 2] = ["d3d11va", "cuda"];

#[derive(Clone, Copy)]
pub(crate) enum GpuEncoder {
    Nvenc,
    MediaFoundation,
}

impl GpuEncoder {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::MediaFoundation => "h264_mf",
        }
    }

    /// Near-lossless settings for Blender masters (quality over file size).
    pub(crate) fn configure_master(self, command: &mut Command, level: &str) {
        command.args(["-c:v", self.name()]);
        match self {
            Self::Nvenc => {
                command.args([
                    "-profile:v", "high", "-level:v", level, "-pix_fmt", "yuv420p",
                    "-preset", "p7", "-tune", "hq", "-rc", "constqp", "-qp", "8",
                    "-bf", "3", "-b_ref_mode", "middle", "-spatial-aq", "1", "-temporal-aq", "1",
                ]);
            }
            Self::MediaFoundation => {
                command.args([
                    "-hw_encoding", "1", "-profile:v", "100", "-pix_fmt", "nv12",
                    "-rate_control", "quality", "-quality", "95", "-bf", "2",
                ]);
            }
        }
    }

    pub(crate) fn configure(self, command: &mut Command, level: &str, cuda_frames: bool) {
        command.args(["-c:v", self.name()]);
        match self {
            Self::Nvenc => {
                command.args([
                    "-profile:v",
                    "high",
                    "-level:v",
                    level,
                    "-pix_fmt",
                    if cuda_frames { "cuda" } else { "yuv420p" },
                    "-preset",
                    "p6",
                    "-tune",
                    "hq",
                    "-rc",
                    "vbr",
                    "-multipass",
                    "fullres",
                    "-rc-lookahead",
                    "20",
                    "-spatial-aq",
                    "1",
                    "-bf",
                    "3",
                ]);
            }
            Self::MediaFoundation => {
                command.args([
                    "-hw_encoding",
                    "1",
                    "-profile:v",
                    "100",
                    "-pix_fmt",
                    "nv12",
                    "-rate_control",
                    "u_vbr",
                    "-compression_level",
                    "85",
                    "-bf",
                    "2",
                ]);
            }
        }
    }
}

pub(crate) struct Acceleration {
    pub(crate) decoders: Vec<&'static str>,
    pub(crate) encoders: Vec<GpuEncoder>,
}

impl Acceleration {
    pub(crate) async fn available(
        tools: &Tools,
        settings: &Settings,
        media: &Media,
        cancel: &CancellationToken,
    ) -> Result<Self, Error> {
        let mut acceleration = Self {
            decoders: Vec::new(),
            encoders: Vec::new(),
        };
        if !cfg!(windows) {
            return Ok(acceleration);
        }
        if settings.gpu_decoding
            && download_format(media).is_some()
            && matches!(
                media.video_codec.as_str(),
                "h264" | "hevc" | "av1" | "vp9" | "mpeg2video" | "vc1" | "wmv3"
            )
        {
            let methods = ffmpeg_list(&tools.ffmpeg, "-hwaccels", cancel).await?;
            acceleration.decoders = HARDWARE_DECODERS
                .into_iter()
                .filter(|method| methods.lines().any(|line| line.trim() == *method))
                .collect();
        }
        if settings.gpu_encoding {
            let list = ffmpeg_list(&tools.ffmpeg, "-encoders", cancel).await?;
            acceleration.encoders = [GpuEncoder::Nvenc, GpuEncoder::MediaFoundation]
                .into_iter()
                .filter(|encoder| {
                    list.lines()
                        .any(|line| line.split_ascii_whitespace().nth(1) == Some(encoder.name()))
                })
                .collect();
        }
        Ok(acceleration)
    }

    pub(crate) fn decode_options(
        &self,
        encoder: Option<GpuEncoder>,
    ) -> impl Iterator<Item = Option<&'static str>> + '_ {
        let mut order = HARDWARE_DECODERS;
        if matches!(encoder, Some(GpuEncoder::Nvenc)) {
            order.reverse();
        }
        order
            .into_iter()
            .filter(|method| self.decoders.contains(method))
            .map(Some)
            .chain(std::iter::once(None))
    }
}

pub(crate) fn download_format(media: &Media) -> Option<&'static str> {
    match media.pixel_format.as_deref() {
        Some("yuv420p" | "yuvj420p" | "nv12") => Some("nv12"),
        Some("yuv420p10le" | "p010le") => Some("p010le"),
        _ => None,
    }
}

async fn ffmpeg_list(
    ffmpeg: &Path,
    flag: &str,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let output = process::capture(
        process::command(ffmpeg).args(["-hide_banner", flag]),
        cancel,
    )
    .await;
    match output {
        Ok(list) if list.status.success() => Ok(String::from_utf8_lossy(&list.stdout).into_owned()),
        Err(error) if error.code == "cancelled" => Err(error),
        _ => Ok(String::new()),
    }
}
