use super::*;
use std::{env, fs, path::PathBuf};
#[cfg(unix)]
use std::{process::Command as StdCommand, sync::Mutex};

fn ffmpeg_tools() -> Tools {
    Tools {
        ffmpeg: env::var_os("UPFLOW_TEST_FFMPEG").map_or_else(|| "ffmpeg".into(), PathBuf::from),
        ffprobe: env::var_os("UPFLOW_TEST_FFPROBE").map_or_else(|| "ffprobe".into(), PathBuf::from),
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe with libx264"]
async fn unavailable_gpu_retries_with_cpu() {
    use std::os::unix::fs::PermissionsExt;

    let tools = ffmpeg_tools();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    assert!(StdCommand::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=30:d=1",
            "-c:v",
            "libx264"
        ])
        .arg(&source)
        .status()
        .unwrap()
        .success());

    let mock = directory.path().join("mock-ffmpeg");
    let ffmpeg = tools.ffmpeg.to_str().unwrap().replace('\'', "'\"'\"'");
    fs::write(&mock, format!(
        "#!/bin/sh\nfor arg do\n  case \"$arg\" in\n    h264_nvenc|d3d11va|cuda) echo 'Video device unavailable' >&2; exit 1;;\n  esac\ndone\nexec '{ffmpeg}' \"$@\"\n"
    )).unwrap();
    fs::set_permissions(&mock, fs::Permissions::from_mode(0o700)).unwrap();

    let cancel = CancellationToken::new();
    let job = Conversion {
        source,
        directory: None,
        frame_size: None,
        settings: Settings {
            threads: 2,
            ..Settings::default()
        },
        log_directory: directory.path().to_owned(),
        tools: Tools {
            ffmpeg: mock,
            ..tools
        },
    };
    let media = probe(&job.source, &job.tools, &cancel).await.unwrap();
    let encoder = Encoder {
        job: &job,
        media: &media,
        level: "4.2",
        gpu_level: "3.1",
        time_base: "demux".to_owned(),
        output_fps: media.fps,
        color_adjusted: false,
        filter: None,
        filter_threads: 1,
        started: Instant::now(),
    };
    let target = ExportTarget {
        path: &directory.path().join("output.mp4"),
        directory: directory.path(),
        frame_size: FrameSize {
            width: media.width,
            height: media.height,
        },
    };
    let messages = Mutex::new(Vec::new());
    let report = |event| {
        if let Event::Log { line } = event {
            messages.lock().unwrap().push(line);
        }
    };
    let mut log = Log::new(&report, None);
    let cuda_command = encoder.command(
        Pass::GpuOutput(GpuEncoder::Nvenc),
        Some("cuda"),
        target.path,
        target.path,
    );
    let cuda_args: Vec<_> = cuda_command.as_std().get_args().collect();
    assert!(cuda_args
        .windows(2)
        .any(|args| args[0] == "-pix_fmt" && args[1] == "cuda"));
    assert!(!cuda_args.contains(&std::ffi::OsStr::new("-vf")));
    let output = encoder
        .export_using(
            Acceleration {
                decoders: vec!["d3d11va", "cuda"],
                encoders: vec![GpuEncoder::Nvenc],
            },
            &target,
            &cancel,
            &mut log,
            None,
        )
        .await
        .unwrap();
    let messages = messages.into_inner().unwrap().join("\n");
    let cuda = messages
        .find("Pass 1 of 1 | h264_nvenc | Decoder: cuda")
        .unwrap();
    let d3d11 = messages
        .find("Pass 1 of 1 | h264_nvenc | Decoder: d3d11va")
        .unwrap();
    let cpu = messages
        .find("Pass 1 of 1 | h264_nvenc | Decoder: CPU")
        .unwrap();
    assert!(cuda < d3d11 && d3d11 < cpu);
    assert!(messages.contains("GPU decoding unavailable"));
    assert!(messages.contains("Pass 2 of 2 | libx264 | Decoder: CPU"));
    assert_eq!(output.encoder, "libx264");
    assert_eq!(output.decoder, None);
    assert_eq!(output.media.frames, Some(output.frames));
}

#[tokio::test]
#[ignore = "manual benchmark; set UPFLOW_BENCH_VIDEO to a local video"]
async fn benchmark_acceleration() {
    use sha2::{Digest, Sha256};
    use std::io;

    let directory = tempfile::tempdir().unwrap();
    let tools = ffmpeg_tools();
    let source = PathBuf::from(env::var_os("UPFLOW_BENCH_VIDEO").expect("Set UPFLOW_BENCH_VIDEO"))
        .canonicalize()
        .unwrap();
    let cancel = CancellationToken::new();
    let job = Conversion {
        source,
        directory: None,
        frame_size: None,
        settings: Settings::default(),
        log_directory: directory.path().to_owned(),
        tools,
    };
    let media = probe(&job.source, &job.tools, &cancel).await.unwrap();
    let hardware = Acceleration::available(&job.tools, &job.settings, &media, &cancel)
        .await
        .unwrap();
    let time_base = encoder_time_base(&media, &job.tools, &cancel)
        .await
        .unwrap();
    let report = |event| {
        if let Event::Log { line } = event {
            if line.starts_with("Pass") {
                println!("{line}");
            }
        }
    };
    let mut log = Log::new(&report, None);
    for resize in std::iter::once(None).chain(media.suggested_frame_size.map(Some)) {
        let frame_size = resize.unwrap_or(FrameSize {
            width: media.width,
            height: media.height,
        });
        let filter = resize.map(fit_filter);
        let target = ExportTarget {
            path: &directory.path().join("bench.mp4"),
            directory: directory.path(),
            frame_size,
        };
        let mut encoder = Encoder {
            job: &job,
            media: &media,
            level: level_for(frame_size, media.max_fps, job.settings.video_bitrate_kbps).unwrap(),
            gpu_level: find_level(
                frame_size,
                media.max_fps,
                job.settings.video_bitrate_kbps,
                true,
            )
            .unwrap(),
            time_base: time_base.to_owned(),
            output_fps: media.fps,
            color_adjusted: false,
            filter: filter.as_deref(),
            filter_threads: 1,
            started: Instant::now(),
        };
        let mut cases = vec![("CPU", None, None, 1)];
        if resize.is_some() {
            cases.push(("Four filter threads", None, None, 4));
        }
        if let Some(Some(decoder)) = hardware.decode_options(None).next() {
            cases.push(("GPU decoding", None, Some(decoder), 1));
        }
        for gpu in &hardware.encoders {
            let decoder = hardware.decode_options(Some(*gpu)).next().flatten();
            cases.push((gpu.name(), Some(*gpu), decoder, 1));
        }

        let mut baseline = None;
        for (label, gpu, decoder, threads) in cases {
            encoder.filter_threads = threads;
            let start = Instant::now();
            let result = if let Some(gpu) = gpu {
                encoder
                    .gpu_export(gpu, decoder, &target, &cancel, &mut log)
                    .await
            } else {
                encoder
                    .cpu_export(decoder, &target, &cancel, &mut log, None)
                    .await
            };
            match result {
                Ok(export) => {
                    let output = export.media;
                    if output.sample_rate == Some(48_000)
                        && output.channel_layout.as_deref() == Some("stereo")
                    {
                        mp4::patch_audio_tail(target.path).unwrap();
                    }
                    let mut hash = Sha256::new();
                    io::copy(&mut fs::File::open(target.path).unwrap(), &mut hash).unwrap();
                    let hash = hash.finalize();
                    let matches = &hash == baseline.get_or_insert(hash);
                    println!(
                        "{}x{} | {label} | {:.2}s | {hash:x} | {}",
                        frame_size.width,
                        frame_size.height,
                        start.elapsed().as_secs_f64(),
                        if matches {
                            "same bytes"
                        } else {
                            "different bytes"
                        }
                    );
                    if label == "Four filter threads"
                        && threads == resize_threads(&media, resize, job.settings.threads)
                    {
                        assert!(matches);
                    }
                }
                Err(error) if gpu.is_some() || decoder.is_some() => {
                    println!("{label}: {} {}", error.message, error.detail)
                }
                Err(error) => panic!("{error:?}"),
            }
        }
    }
}
