use crate::{
    acceleration::{download_format, Acceleration, GpuEncoder},
    checkpoint::CheckpointKey,
    media::{detect_patch, encoder_time_base, null_device, probe, LOCAL_FORMATS},
    mp4,
    output::{deliver, output_directory, validate},
    process,
    progress::{run_pass, Log, Progress},
    AudioPatch, Error, Event, FrameSize, Media, Output, PassCheckpoint, RenderMode, Settings,
    Tools,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub struct Conversion {
    pub source: PathBuf,
    pub directory: Option<PathBuf>,
    pub frame_size: Option<FrameSize>,
    pub settings: Settings,
    pub log_directory: PathBuf,
    pub tools: Tools,
}

pub async fn convert(
    job: Conversion,
    cancel: CancellationToken,
    report: impl Fn(Event) + Send + Sync,
) -> Result<Output, Error> {
    convert_job(job, cancel, report, None).await
}

pub async fn convert_with_retry(
    job: Conversion,
    cancel: CancellationToken,
    report: impl Fn(Event) + Send + Sync,
    checkpoint: &mut Option<PassCheckpoint>,
) -> Result<Output, Error> {
    let result = convert_job(job, cancel, report, Some(checkpoint)).await;
    if result.is_ok() {
        *checkpoint = None;
    }
    result
}

async fn convert_job(
    job: Conversion,
    cancel: CancellationToken,
    report: impl Fn(Event) + Send + Sync,
    checkpoint: Option<&mut Option<PassCheckpoint>>,
) -> Result<Output, Error> {
    let started = Instant::now();
    let log = if job.settings.keep_logs {
        fs::create_dir_all(&job.log_directory)
            .map_err(|e| Error::io(e, "logs", "Could not create the logs folder."))?;
        let log = tempfile::Builder::new()
            .prefix("finer-")
            .suffix(".log")
            .tempfile_in(&job.log_directory)
            .and_then(|f| f.keep().map_err(|e| e.error))
            .map_err(|e| Error::io(e, "logs", "Could not create the conversion log."))?;
        report(Event::Log {
            line: format!("Log file: {}", log.1.display()),
        });
        Some(log.0)
    } else {
        None
    };
    let mut log = Log::new(&report, log);
    let result = encode(job, &cancel, &mut log, started, checkpoint).await;
    if let Err(error) = &result {
        log.line(
            format!("{} {}", error.message, error.detail)
                .trim()
                .to_owned(),
        );
    }
    result
}

async fn encode(
    job: Conversion,
    cancel: &CancellationToken,
    log: &mut Log<'_>,
    started: Instant,
    checkpoint: Option<&mut Option<PassCheckpoint>>,
) -> Result<Output, Error> {
    check_cancel(cancel)?;
    job.settings.validate_video_bitrate()?;
    if job.frame_size.is_some_and(|size| !size.is_supported()) {
        return Err(Error::new(
            "unsupportedDimensions",
            "Choose a supported output frame size.",
            "",
        ));
    }
    let media = probe(&job.source, &job.tools, cancel).await?;
    if detect_patch(&media.path, &media.container).await {
        return Err(Error::new(
            "alreadyPatched",
            "This video is already patched.",
            "",
        ));
    }
    check_cancel(cancel)?;
    let bitrate_kbps = job.settings.video_bitrate_kbps;
    let frame_size = job.frame_size.unwrap_or(FrameSize {
        width: media.width,
        height: media.height,
    });
    let output_fps = match job.settings.render_mode {
        RenderMode::Quality => media.fps,
        RenderMode::FrameBlend | RenderMode::MotionBlur => f64::from(job.settings.target_fps),
    };
    let level_fps = if job.settings.render_mode == RenderMode::Quality {
        media.max_fps
    } else {
        output_fps
    };
    let level = level_for(frame_size, level_fps, bitrate_kbps)?;
    let gpu_level = find_level(frame_size, level_fps, bitrate_kbps, true).unwrap_or(level);
    let time_base = if job.settings.render_mode == RenderMode::Quality {
        encoder_time_base(&media, &job.tools, cancel)
            .await?
            .to_owned()
    } else {
        format!("1:{}", job.settings.target_fps)
    };
    let shadow_lifted = job.settings.shadow_lift && job.settings.render_mode != RenderMode::Quality;
    // OBS and similar recorders tag Rec.709 primaries with the sRGB transfer curve.
    // Premiere converts that to a true Rec.709 curve on export; match it so the
    // result looks the same in players and on TikTok.
    let srgb_source = media
        .color_transfer
        .as_deref()
        .is_some_and(is_srgb_transfer);
    let video_filter = render_filter(&job.settings, job.frame_size, media.fps, srgb_source);
    let directory = output_directory(&job.source, job.directory.as_deref(), &job.settings)
        .expect("absolute video path has a parent")
        .to_owned();
    if !directory.is_absolute() || !directory.is_dir() {
        return Err(Error::new(
            "outputDirectory",
            "Choose an existing output folder.",
            directory.display(),
        ));
    }
    let work = tempfile::Builder::new()
        .prefix(".finer-")
        .tempdir_in(&directory)
        .map_err(|e| {
            Error::io(
                e,
                "outputDirectory",
                "Could not write to the output folder.",
            )
        })?;
    // Reserve room for x264 stats, faststart and a nominal output. Very simple clips can consume less.
    let expected = (media.duration * f64::from((bitrate_kbps + 160) * 125) * 1.08) as u64;
    if fs2::available_space(&directory).is_ok_and(|available| available < expected) {
        return Err(Error::new(
            "diskFull",
            "There is not enough disk space for the estimated output.",
            "",
        ));
    }
    let temp = NamedTempFile::new_in(work.path())
        .map_err(|e| {
            Error::io(
                e,
                "outputDirectory",
                "Could not create the temporary output.",
            )
        })?
        .into_temp_path();
    let temp_path: &Path = temp.as_ref();

    let encoder = Encoder {
        job: &job,
        media: &media,
        level,
        gpu_level,
        time_base,
        output_fps,
        filter: video_filter.as_deref(),
        filter_threads: resize_threads(&media, job.frame_size, job.settings.threads),
        started,
    };
    let target = ExportTarget {
        path: temp_path,
        directory: work.path(),
        frame_size,
    };
    let Encoded {
        media: mut output,
        frames,
        encoder: video_encoder,
        decoder,
    } = encoder.export(&target, cancel, log, checkpoint).await?;
    let gpu = video_encoder != "libx264";
    check_cancel(cancel)?;
    (log.report)(Event::Finalizing);
    let audio_patch = if output.audio_codec.is_none() {
        AudioPatch::NoAudio
    } else if output.sample_rate == Some(48_000)
        && output.channel_layout.as_deref() == Some("stereo")
    {
        check_cancel(cancel)?;
        let staged = temp_path.to_owned();
        let patched = tokio::task::spawn_blocking(move || mp4::patch_audio_tail(&staged))
            .await
            .map_err(|error| {
                Error::new(
                    "invalidOutput",
                    "Could not apply the MP4 audio patch.",
                    error,
                )
            })?
            .map_err(|error| {
                Error::io(
                    error,
                    "invalidOutput",
                    "Could not apply the MP4 audio patch.",
                )
            })?;
        if patched {
            output = probe(temp_path, &job.tools, cancel).await?;
            validate(
                &media,
                &output,
                frame_size,
                frames,
                if gpu { gpu_level } else { level },
                gpu,
                output_fps,
            )?;
            log.line("Applied the four-packet MP4 audio tail patch.".into());
            AudioPatch::Applied
        } else {
            AudioPatch::TooShort
        }
    } else {
        AudioPatch::NeedsStereo
    };
    check_cancel(cancel)?;
    let path = deliver(temp, &directory, &media.path)?;
    log.line(format!("Saved {}", path.display()));
    Ok(Output {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path,
        size: output.size,
        directory,
        audio_patch,
        encoder: video_encoder,
        decoder: decoder.unwrap_or("cpu"),
        shadow_lifted,
    })
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), Error> {
    if cancel.is_cancelled() {
        Err(Error::cancelled())
    } else {
        Ok(())
    }
}

pub(crate) fn level_for(
    size: FrameSize,
    max_fps: f64,
    bitrate_kbps: u32,
) -> Result<&'static str, Error> {
    if !size.width.is_multiple_of(2) || !size.height.is_multiple_of(2) {
        return Err(Error::new(
            "unsupportedDimensions",
            "This video needs even width and height for H.264.",
            "",
        ));
    }
    find_level(size, max_fps, bitrate_kbps, false).ok_or_else(|| {
        Error::new(
            "unsupportedDimensions",
            "This resolution and frame rate exceed H.264 limits.",
            "",
        )
    })
}

fn find_level(
    size: FrameSize,
    max_fps: f64,
    bitrate_kbps: u32,
    allow_lower: bool,
) -> Option<&'static str> {
    let width = u64::from(size.width.div_ceil(16));
    let height = u64::from(size.height.div_ceil(16));
    let frame = width * height;
    let levels = [
        ("3.1", 3_600, 108_000, 17_500),
        ("3.2", 5_120, 216_000, 25_000),
        ("4.0", 8_192, 245_760, 25_000),
        ("4.1", 8_192, 245_760, 62_500),
        ("4.2", 8_704, 522_240, 62_500),
        ("5.0", 22_080, 589_824, 168_750),
        ("5.1", 36_864, 983_040, 300_000),
        ("5.2", 36_864, 2_073_600, 300_000),
        ("6.0", 139_264, 4_177_920, 300_000),
        ("6.1", 139_264, 8_355_840, 600_000),
        ("6.2", 139_264, 16_711_680, 1_000_000),
    ];
    // All listed levels have room for at least two reference frames at MaxFS.
    // The per-dimension limit is sqrt(8 * MaxFS) macroblocks.
    levels
        .into_iter()
        .skip(if allow_lower { 0 } else { 4 })
        .find(|&(_, max_frame, max_mbps, max_bitrate)| {
            frame <= max_frame
                && width.max(height).pow(2) <= 8 * max_frame
                && frame as f64 * max_fps <= max_mbps as f64
                && bitrate_kbps <= max_bitrate
        })
        .map(|(level, _, _, _)| level)
}

fn fit_filter(FrameSize { width, height }: FrameSize) -> String {
    format!(
        "scale=w='max(2,trunc(min({width},{height}*dar)/2)*2)':h='max(2,trunc(min({height},{width}/dar)/2)*2)':flags=lanczos,setsar=1,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
    )
}

fn is_srgb_transfer(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "srgb" | "iec61966-2-1")
}

fn render_filter(
    settings: &Settings,
    resize: Option<FrameSize>,
    source_fps: f64,
    srgb_source: bool,
) -> Option<String> {
    let mut filters = Vec::new();
    if srgb_source {
        filters.push("colorspace=ispace=bt709:iprimaries=bt709:itrc=iec61966-2-1:space=bt709:primaries=bt709:trc=bt709:format=yuv420p".to_owned());
    }
    if let Some(size) = resize {
        filters.push(fit_filter(size));
    }
    match settings.render_mode {
        RenderMode::Quality => {}
        RenderMode::FrameBlend | RenderMode::MotionBlur => {
            if let Some(blend) = temporal_blend(settings, source_fps) {
                filters.push(blend);
            }
        }
    }
    if settings.shadow_lift && settings.render_mode != RenderMode::Quality {
        // Lift dark and mid-tone luminance while protecting highlights. This is
        // intentionally a creative Minecraft look, not automatic color management.
        filters.push("eq=gamma=1.18:gamma_weight=0.82".to_owned());
    }
    (!filters.is_empty()).then(|| filters.join(","))
}

/// Number of source frames covered by one output frame (f0e's `frame_gap`).
fn frame_gap(source_fps: f64, target_fps: u32) -> u32 {
    (source_fps / f64::from(target_fps)).floor().max(1.0) as u32
}

/// f0e/blur `gaussian_sym`: mean 0, std dev 1, sampled evenly over [-2, 2].
fn gaussian_sym_weights(frames: u32) -> String {
    if frames <= 1 {
        return "1".to_owned();
    }
    let step = 4.0 / f64::from(frames - 1);
    (0..frames)
        .map(|i| {
            let x = -2.0 + step * f64::from(i);
            // Integer weights keep tmix exact; 1000 gives ample precision.
            ((-x * x / 2.0).exp() * 1000.0).round().max(1.0).to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Temporal blending for Frame Blend (Premiere/f0e "equal") and Motion Blur
/// (f0e "gaussian_sym").
///
/// Improvements over both references:
/// * Frames are mixed at 16-bit precision and dithered back to 8-bit once,
///   so the blend adds no banding on top of the source.
/// * The window is centred on the output frame, keeping picture and audio in
///   sync (tmix itself only looks backwards).
/// * Each output frame is exactly one blended frame, chosen on the source's own
///   timeline, never a re-blend or a dropped neighbour.
/// * High-frame-rate recordings use their real frames instead of synthetic
///   interpolated ones, so no interpolation artefacts can appear.
fn temporal_blend(settings: &Settings, source_fps: f64) -> Option<String> {
    let target = settings.target_fps;
    let target_f = f64::from(target);
    let odd = |n: u32| if n % 2 == 0 { n + 1 } else { n };
    let blur = settings.render_mode == RenderMode::MotionBlur;

    // Sources slower than the target need synthetic in-between frames first.
    let (prefix, working_fps) = if source_fps < target_f * if blur { 2.0 } else { 0.995 } {
        if !blur {
            return Some(format!(
                "minterpolate=fps={target}:mi_mode=blend:scd=fdiff:scd_threshold=10,fps=fps={target}:round=near"
            ));
        }
        // Motion search is single-threaded, so cap it to stay usable.
        let working = (target * u32::from(settings.blur_samples).clamp(3, 8)).min(480);
        (
            format!("minterpolate=fps={working}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1:scd=fdiff:scd_threshold=10,"),
            f64::from(working),
        )
    } else {
        (String::new(), source_fps)
    };

    let gap = frame_gap(working_fps, target);
    if !blur && gap <= 1 {
        return Some(format!("fps=fps={target}:round=near"));
    }
    let frames = if blur {
        // Samples act as f0e's blur amount: more samples, longer shutter,
        // capped at two output frames so motion stays readable.
        odd(u32::from(settings.blur_samples).min(2 * gap + 1).max(3))
    } else {
        gap.max(2)
    };
    let weights = if blur {
        gaussian_sym_weights(frames)
    } else {
        vec!["1"; frames as usize].join(" ")
    };
    let pad = frames / 2;
    let centre = f64::from(frames - 1) / 2.0 / working_fps;
    Some(format!(
        "{prefix}format=yuv420p16le,tpad=stop_mode=clone:stop={pad},tmix=frames={frames}:weights='{weights}',setpts=PTS-{centre:.6}/TB,fps=fps={target}:round=near:start_time=0,scale=sws_dither=none,format=yuv420p"
    ))
}
#[allow(dead_code)]
fn binomial_weights(samples: u32) -> String {
    let row = samples.saturating_sub(1);
    let mut value = 1_u64;
    let mut weights = Vec::with_capacity(samples as usize);
    for index in 0..samples {
        weights.push(value.to_string());
        if index < row {
            value = value * u64::from(row - index) / u64::from(index + 1);
        }
    }
    weights.join(" ")
}

fn resize_threads(media: &Media, resize: Option<FrameSize>, threads: u16) -> u8 {
    let standard_4k = matches!((media.width, media.height), (3840, 2160) | (2160, 3840));
    if threads >= 4
        && resize.is_some()
        && standard_4k
        && media.pixel_format.as_deref() == Some("yuv420p")
    {
        4
    } else {
        1
    }
}

struct Encoder<'a> {
    job: &'a Conversion,
    media: &'a Media,
    level: &'static str,
    gpu_level: &'static str,
    time_base: String,
    output_fps: f64,
    filter: Option<&'a str>,
    filter_threads: u8,
    started: Instant,
}

#[derive(Clone, Copy)]
enum Pass {
    Analysis,
    SoftwareOutput,
    MasterOutput,
    GpuMaster(GpuEncoder),
    GpuOutput(GpuEncoder),
}

impl Pass {
    fn name(self) -> &'static str {
        match self {
            Self::GpuOutput(encoder) | Self::GpuMaster(encoder) => encoder.name(),
            _ => "libx264",
        }
    }
}

struct Encoded {
    media: Media,
    frames: u64,
    encoder: &'static str,
    decoder: Option<&'static str>,
}

struct ExportTarget<'a> {
    path: &'a Path,
    directory: &'a Path,
    frame_size: FrameSize,
}

fn can_retry(error: &Error) -> bool {
    !matches!(
        error.code,
        "cancelled" | "permissionDenied" | "diskFull" | "outputDirectory"
    )
}

fn report_decode_failure(log: &mut Log<'_>, decoder: &str, error: &Error) {
    log.line(format!(
        "GPU decoding unavailable ({decoder}): {} {}",
        error.message, error.detail
    ));
}

impl Encoder<'_> {
    async fn export(
        &self,
        target: &ExportTarget<'_>,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
        checkpoint: Option<&mut Option<PassCheckpoint>>,
    ) -> Result<Encoded, Error> {
        let acceleration =
            Acceleration::available(&self.job.tools, &self.job.settings, self.media, cancel)
                .await?;
        self.export_using(acceleration, target, cancel, log, checkpoint)
            .await
    }

    async fn export_using(
        &self,
        mut acceleration: Acceleration,
        target: &ExportTarget<'_>,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
        mut checkpoint: Option<&mut Option<PassCheckpoint>>,
    ) -> Result<Encoded, Error> {
        if self.job.settings.render_mode != RenderMode::Quality {
            if let Some(slot) = checkpoint.as_deref_mut() {
                *slot = None;
            }
            // Blender renders one visually lossless master, on the GPU when chosen
            // and available, otherwise (or after a GPU failure) with x264.
            if self.job.settings.gpu_encoding {
                for &encoder in &acceleration.encoders {
                    for decode in acceleration.decode_options(Some(encoder)) {
                        match self.gpu_master(encoder, decode, target, cancel, log).await {
                            Ok(output) => return Ok(output),
                            Err(error) if can_retry(&error) => log.line(format!(
                                "{} with {} decoding: {} {}",
                                encoder.name(),
                                decode.unwrap_or("CPU"),
                                error.message,
                                error.detail
                            )),
                            Err(error) => return Err(error),
                        }
                    }
                }
                if !acceleration.encoders.is_empty() {
                    log.line("Using the CPU encoder.".into());
                }
            }
            for decode in acceleration.decode_options(None) {
                match self.master_export(decode, target, cancel, log).await {
                    Err(error) if decode.is_some() && can_retry(&error) => {
                        report_decode_failure(log, decode.expect("checked above"), &error);
                    }
                    result => return result,
                }
            }
            unreachable!("CPU decoding is always tried")
        }

        // Finer can use the configured encoder. Blender returned above through the
        // visually lossless master path; GPU decoding may still accelerate source reads.
        let mut try_gpu =
            self.job.settings.gpu_encoding && self.job.settings.render_mode == RenderMode::Quality;
        if let Some(slot) = checkpoint.as_deref_mut() {
            if let Some(saved) = slot.as_ref() {
                let cached_decoder = saved.key.decoder;
                if cached_decoder.is_none_or(|method| acceleration.decoders.contains(&method)) {
                    let key =
                        CheckpointKey::read(self.job, &self.time_base, cached_decoder, cancel)
                            .await;
                    check_cancel(cancel)?;
                    if key.as_ref() == Some(&saved.key) && saved.key.files_unchanged() {
                        log.line("Reusing the completed first pass.".into());
                        Progress::resume(self.started, saved.speed, log.report);
                        let passlog = saved.directory.path().join("x264");
                        let result = self
                            .second_pass(
                                cached_decoder,
                                target,
                                saved.frames,
                                &passlog,
                                cancel,
                                log,
                            )
                            .await;
                        match (result, cached_decoder) {
                            (Err(error), Some(method)) if can_retry(&error) => {
                                report_decode_failure(log, method, &error);
                                acceleration.decoders.clear();
                                try_gpu = false;
                            }
                            (result, _) => return result,
                        }
                    }
                }
                *slot = None;
            }
        }

        if try_gpu {
            for &encoder in &acceleration.encoders {
                for decode in acceleration.decode_options(Some(encoder)) {
                    match self.gpu_export(encoder, decode, target, cancel, log).await {
                        Ok(output) => return Ok(output),
                        Err(error) if can_retry(&error) => {
                            log.line(format!(
                                "{} with {} decoding: {} {}",
                                encoder.name(),
                                decode.unwrap_or("CPU"),
                                error.message,
                                error.detail
                            ));
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            log.line("Using the CPU encoder.".into());
        }

        for decode in acceleration.decode_options(None) {
            let result = self
                .cpu_export(decode, target, cancel, log, checkpoint.as_deref_mut())
                .await;
            match (result, decode) {
                (Err(error), Some(method)) if can_retry(&error) => {
                    report_decode_failure(log, method, &error);
                    if let Some(slot) = checkpoint.as_deref_mut() {
                        *slot = None;
                    }
                }
                (result, _) => return result,
            }
        }
        unreachable!("CPU decoding is always tried")
    }

    async fn master_export(
        &self,
        decoder: Option<&'static str>,
        target: &ExportTarget<'_>,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
    ) -> Result<Encoded, Error> {
        let frames = self
            .run(
                Pass::MasterOutput,
                decoder,
                target.path,
                target.path,
                cancel,
                log,
            )
            .await?
            .frame;
        let media = self.verify(target, frames, false, cancel).await?;
        Ok(Encoded {
            media,
            frames,
            encoder: "libx264",
            decoder,
        })
    }

    async fn gpu_master(
        &self,
        encoder: GpuEncoder,
        decoder: Option<&'static str>,
        target: &ExportTarget<'_>,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
    ) -> Result<Encoded, Error> {
        let frames = self
            .run(Pass::GpuMaster(encoder), decoder, target.path, target.path, cancel, log)
            .await?
            .frame;
        let media = self.verify(target, frames, true, cancel).await?;
        Ok(Encoded {
            media,
            frames,
            encoder: encoder.name(),
            decoder,
        })
    }

    async fn gpu_export(
        &self,
        encoder: GpuEncoder,
        decoder: Option<&'static str>,
        target: &ExportTarget<'_>,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
    ) -> Result<Encoded, Error> {
        let frames = self
            .run(
                Pass::GpuOutput(encoder),
                decoder,
                target.path,
                target.path,
                cancel,
                log,
            )
            .await?
            .frame;
        if self
            .media
            .frames
            .is_some_and(|input_frames| input_frames != frames)
        {
            return Err(Error::new(
                "invalidOutput",
                "GPU encoding changed the frame count.",
                "",
            ));
        }
        let media = self.verify(target, frames, true, cancel).await?;
        Ok(Encoded {
            media,
            frames,
            encoder: encoder.name(),
            decoder,
        })
    }

    async fn cpu_export(
        &self,
        decoder: Option<&'static str>,
        target: &ExportTarget<'_>,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
        checkpoint: Option<&mut Option<PassCheckpoint>>,
    ) -> Result<Encoded, Error> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(".finer-pass-");
        let stats = builder
            .tempdir()
            .or_else(|_| builder.tempdir_in(target.directory))
            .map_err(|e| Error::io(e, "outputDirectory", "Could not prepare the video encoder."))?;
        let passlog = stats.path().join("x264");
        let first_pass = self.run(Pass::Analysis, decoder, &passlog, target.path, cancel, log);
        let (first, key) = if checkpoint.is_some() {
            let (key, pass) = tokio::join!(
                CheckpointKey::read(self.job, &self.time_base, decoder, cancel),
                first_pass
            );
            (pass?, key)
        } else {
            (first_pass.await?, None)
        };
        let _stats = match (checkpoint, key.filter(CheckpointKey::files_unchanged)) {
            (Some(slot), Some(key)) => {
                *slot = Some(PassCheckpoint {
                    directory: stats,
                    frames: first.frame,
                    speed: first.speed,
                    key,
                });
                None
            }
            _ => Some(stats),
        };
        self.second_pass(decoder, target, first.frame, &passlog, cancel, log)
            .await
    }

    async fn second_pass(
        &self,
        decoder: Option<&'static str>,
        target: &ExportTarget<'_>,
        expected_frames: u64,
        passlog: &Path,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
    ) -> Result<Encoded, Error> {
        let frames = self
            .run(
                Pass::SoftwareOutput,
                decoder,
                passlog,
                target.path,
                cancel,
                log,
            )
            .await?
            .frame;
        if frames != expected_frames {
            return Err(Error::new(
                "encoder",
                "The encoder returned an unexpected number of frames.",
                format!("First pass: {expected_frames}, second pass: {frames}"),
            ));
        }
        let media = self.verify(target, frames, false, cancel).await?;
        Ok(Encoded {
            media,
            frames,
            encoder: "libx264",
            decoder,
        })
    }

    async fn verify(
        &self,
        target: &ExportTarget<'_>,
        frames: u64,
        using_gpu: bool,
        cancel: &CancellationToken,
    ) -> Result<Media, Error> {
        let media = probe(target.path, &self.job.tools, cancel)
            .await
            .map_err(|error| {
                if error.code == "cancelled" {
                    error
                } else {
                    Error::new(
                        "invalidOutput",
                        "The exported video could not be checked.",
                        format!("{} {}", error.message, error.detail),
                    )
                }
            })?;
        validate(
            self.media,
            &media,
            target.frame_size,
            frames,
            if using_gpu {
                self.gpu_level
            } else {
                self.level
            },
            using_gpu,
            self.output_fps,
        )?;
        Ok(media)
    }

    async fn run(
        &self,
        pass: Pass,
        decoder: Option<&str>,
        passlog: &Path,
        output: &Path,
        cancel: &CancellationToken,
        log: &mut Log<'_>,
    ) -> Result<Progress, Error> {
        check_cancel(cancel)?;
        let (number, total) = match pass {
            Pass::Analysis => (1, 2),
            Pass::SoftwareOutput => (2, 2),
            Pass::MasterOutput => (1, 1),
            Pass::GpuOutput(_) | Pass::GpuMaster(_) => (1, 1),
        };
        let encoder = pass.name();
        let mut command = self.command(pass, decoder, passlog, output);
        let mut progress = Progress::new(number, total, encoder, self.media.duration, self.started);
        progress.emit(log.report, false);
        check_cancel(cancel)?;
        log.line(format!(
            "Pass {number} of {total} | {encoder} | Decoder: {}",
            decoder.unwrap_or("CPU")
        ));
        log.line(format!("{command:?}"));
        run_pass(&mut command, &mut progress, log, cancel).await?;
        if progress.frame == 0 {
            return Err(Error::new(
                "encoder",
                "The encoder returned an empty video.",
                "",
            ));
        }
        progress.finish(log.report);
        Ok(progress)
    }

    fn command(&self, pass: Pass, decoder: Option<&str>, passlog: &Path, output: &Path) -> Command {
        let media = self.media;
        let settings = &self.job.settings;
        let mut command = process::command(&self.job.tools.ffmpeg);
        command.args([
            "-hide_banner",
            "-loglevel",
            "info",
            "-nostdin",
            "-y",
            "-xerror",
            "-filter_threads",
            &self.filter_threads.to_string(),
            "-nostats",
            "-stats_period",
            "0.25",
            "-progress",
            "pipe:1",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            LOCAL_FORMATS,
            "-noautorotate",
        ]);
        if let Some(decoder) = decoder {
            let surface = if decoder == "cuda" { "cuda" } else { "d3d11" };
            command.args([
                format!("-hwaccel:{}", media.video_index),
                decoder.to_owned(),
                format!("-hwaccel_output_format:{}", media.video_index),
                surface.to_owned(),
            ]);
        }
        command
            .arg("-i")
            .arg(&media.path)
            .args(["-map", &format!("0:{}", media.video_index)]);
        let hardware_format = decoder.and_then(|_| download_format(media));
        let cuda_frames = decoder == Some("cuda")
            && matches!(pass, Pass::GpuOutput(GpuEncoder::Nvenc))
            && self.filter.is_none()
            && hardware_format == Some("nv12");
        if let Some(format) = hardware_format.filter(|_| !cuda_frames) {
            let mut filter = format!("hwdownload,format={format}");
            if let Some(resize) = self.filter {
                filter.push(',');
                filter.push_str(resize);
            }
            command.arg("-vf").arg(filter);
        } else if let Some(filter) = self.filter {
            command.args(["-vf", filter]);
        }
        if !matches!(pass, Pass::Analysis) {
            if let Some(index) = media.audio_index {
                command.args(["-map", &format!("0:{index}")]);
                if media.audio_codec.as_deref() == Some("aac")
                    && media.audio_profile.as_deref() == Some("LC")
                    && media.sample_rate == Some(48_000)
                    && media.channel_layout.as_deref() == Some("stereo")
                {
                    command.args(["-c:a", "copy"]);
                } else {
                    command.args([
                        "-c:a",
                        "aac",
                        "-profile:a",
                        "aac_low",
                        "-b:a",
                        "320k",
                        "-ar",
                        "48000",
                        "-ac",
                        "2",
                    ]);
                }
            }
        }
        command.args([
            "-map_chapters",
            "-1",
            "-fps_mode:v",
            "passthrough",
            "-enc_time_base:v",
            &self.time_base,
            "-g",
            "250",
        ]);
        match pass {
            Pass::GpuOutput(encoder) => {
                command.args(["-b:v", &format!("{}k", settings.video_bitrate_kbps)]);
                encoder.configure(&mut command, self.gpu_level, cuda_frames)
            }
            Pass::Analysis | Pass::SoftwareOutput => {
                let number = if matches!(pass, Pass::Analysis) {
                    "1"
                } else {
                    "2"
                };
                command.args([
                "-b:v", &format!("{}k", settings.video_bitrate_kbps),
                "-c:v", "libx264", "-preset", "slow",
                "-profile:v", "high", "-level:v", self.level, "-pix_fmt", "yuv420p",
                "-keyint_min", "25", "-sc_threshold", "40", "-refs", "5", "-bf", "3",
                "-x264-params", "b-adapt=2:b-pyramid=2:rc-lookahead=50:mbtree=1:aq-mode=3:aq-strength=0.8:psy-rd=1.0,0.0",
                "-threads:v", &settings.threads.to_string(), "-pass", number, "-passlogfile",
            ]).arg(passlog);
            }
            Pass::GpuMaster(encoder) => encoder.configure_master(&mut command, self.gpu_level),
            Pass::MasterOutput => {
                command.args([
                    "-c:v", "libx264", "-preset", "slow", "-crf", "1",
                    "-profile:v", "high", "-level:v", self.level, "-pix_fmt", "yuv420p",
                    "-keyint_min", "25", "-sc_threshold", "40", "-refs", "5", "-bf", "3",
                    "-x264-params", "b-adapt=2:b-pyramid=2:rc-lookahead=50:mbtree=1:aq-mode=3:aq-strength=0.8:psy-rd=1.0,0.0",
                    "-threads:v", &settings.threads.to_string(),
                ]);
            }
        }
        if matches!(pass, Pass::Analysis) {
            command.args(["-an", "-f", "null", null_device()]);
        } else {
            command
                .args(["-movflags", "+faststart", "-f", "mp4"])
                .arg(output);
        }
        command
    }
}

#[cfg(test)]
#[path = "encoding_checks.rs"]
mod checks;

#[cfg(test)]
mod render_filter_tests {
    use super::render_filter;
    use crate::{RenderMode, Settings};

    #[test]
    fn frame_blend_accumulates_high_fps_frames_before_retiming() {
        let settings = Settings {
            render_mode: RenderMode::FrameBlend,
            target_fps: 60,
            ..Settings::default()
        };
        assert_eq!(
            render_filter(&settings, None, 120.0, false).as_deref(),
            Some("format=yuv420p16le,tpad=stop_mode=clone:stop=1,tmix=frames=2:weights='1 1',setpts=PTS-0.004167/TB,fps=fps=60:round=near:start_time=0,scale=sws_dither=none,format=yuv420p")
        );
    }

    #[test]
    fn motion_blur_interpolates_mixes_and_retimes() {
        let settings = Settings {
            render_mode: RenderMode::MotionBlur,
            target_fps: 60,
            blur_samples: 5,
            ..Settings::default()
        };
        assert_eq!(
            render_filter(&settings, None, 120.0, false).as_deref(),
            Some("format=yuv420p16le,tpad=stop_mode=clone:stop=2,tmix=frames=5:weights='135 607 1000 607 135',setpts=PTS-0.016667/TB,fps=fps=60:round=near:start_time=0,scale=sws_dither=none,format=yuv420p")
        );
    }

    #[test]
    fn shadow_lift_brightens_dark_tones_without_color_conversion() {
        let settings = Settings {
            render_mode: RenderMode::FrameBlend,
            shadow_lift: true,
            ..Settings::default()
        };
        assert_eq!(
            render_filter(&settings, None, 60.0, false).as_deref(),
            Some("fps=fps=60:round=near,eq=gamma=1.18:gamma_weight=0.82")
        );
        let quality = Settings { shadow_lift: true, ..Settings::default() };
        assert_eq!(render_filter(&quality, None, 60.0, false), None);
    }
}
