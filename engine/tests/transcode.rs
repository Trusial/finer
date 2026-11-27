use finer_engine::{
    convert, convert_with_retry, inspect, plan_outputs, AudioPatch, Conversion, Event, FrameSize,
    RenderMode, Settings, Tools,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

fn tools() -> Tools {
    let binary = |key, name: &str| {
        std::env::var_os(key).map_or_else(
            || {
                let name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
                if let Some(path) = std::env::var_os("PATH") {
                    if let Some(tool) = std::env::split_paths(&path)
                        .map(|folder| folder.join(&name))
                        .find(|path| path.is_file())
                    {
                        return tool;
                    }
                }
                PathBuf::from(name)
            },
            PathBuf::from,
        )
    };
    Tools {
        ffmpeg: binary("UPFLOW_TEST_FFMPEG", "ffmpeg"),
        ffprobe: binary("UPFLOW_TEST_FFPROBE", "ffprobe"),
    }
}

fn run(cmd: &mut Command) -> Output {
    let result = cmd.output().unwrap();
    assert!(
        result.status.success(),
        "{cmd:?}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

fn fixture(ffmpeg: &Path, path: &Path, source: &str, codec: &str, rate: Option<&str>) {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-f", "lavfi", "-i", source]);
    if let Some(rate) = rate {
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=750:sample_rate={rate}"),
        ]);
    }
    cmd.args([
        "-shortest",
        "-c:v",
        codec,
        "-threads:v",
        "2",
        "-pix_fmt",
        "yuv420p",
    ]);
    if codec == "libx265" {
        cmd.args(["-x265-params", "pools=1:log-level=error"]);
    }
    if rate.is_some() {
        cmd.args([
            "-c:a",
            if path.extension() == Some(OsStr::new("webm")) {
                "libopus"
            } else {
                "aac"
            },
        ]);
    }
    run(cmd.arg(path));
}

fn probe(ffprobe: &Path, path: &Path, frames: bool) -> Value {
    let mut command = Command::new(ffprobe);
    command.args([
        "-v",
        "error",
        "-of",
        "json",
        "-show_streams",
        "-show_format",
    ]);
    if frames {
        command.args([
            "-select_streams",
            "v:0",
            "-show_frames",
            "-show_entries",
            "frame=best_effort_timestamp_time,key_frame",
        ]);
    }
    serde_json::from_slice(&run(command.arg(path)).stdout).unwrap()
}

fn job(source: &Path, base: &Path, tools: &Tools) -> Conversion {
    Conversion {
        source: source.to_owned(),
        directory: Some(base.to_owned()),
        frame_size: None,
        settings: Settings {
            threads: 2,
            keep_logs: true,
            gpu_decoding: false,
            gpu_encoding: false,
            ..Settings::default()
        },
        log_directory: base.join("logs"),
        tools: tools.clone(),
    }
}

fn no_staging(base: &Path) {
    assert!(fs::read_dir(base).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".finer-")
    }));
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn resize_and_higher_bitrate() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path();
    let tools = tools();
    let input = base.join("square-ish.mp4");
    fixture(
        &tools.ffmpeg,
        &input,
        "testsrc2=size=640x480:rate=24:duration=0.5",
        "libx264",
        None,
    );
    let media = inspect(&input, &tools, &CancellationToken::new())
        .await
        .unwrap();
    let suggested = media.suggested_frame_size.unwrap();
    assert_eq!(
        suggested,
        FrameSize {
            width: 1920,
            height: 1080
        }
    );

    let kept = convert(job(&input, base, &tools), CancellationToken::new(), |_| {})
        .await
        .unwrap();
    let kept = probe(&tools.ffprobe, &kept.path, false);
    assert_eq!(kept["streams"][0]["width"], 640);
    assert_eq!(kept["streams"][0]["height"], 480);
    assert_eq!(kept["streams"][0]["level"], 50);

    let mut resized = job(&input, base, &tools);
    resized.frame_size = Some(suggested);
    resized.settings.video_bitrate_kbps = 116_319;
    let commands = Mutex::new(Vec::new());
    let output = convert(resized, CancellationToken::new(), |event| {
        if let Event::Log { line } = event {
            if line.contains("-b:v") {
                commands.lock().unwrap().push(line);
            }
        }
    })
    .await
    .unwrap();
    let encoded = probe(&tools.ffprobe, &output.path, false);
    let stream = &encoded["streams"][0];
    assert_eq!(stream["width"], suggested.width);
    assert_eq!(stream["height"], suggested.height);
    assert_eq!(stream["sample_aspect_ratio"], "1:1");
    assert_eq!(stream["level"], 50);
    let commands = commands.into_inner().unwrap();
    assert_eq!(commands.len(), 2);
    assert!(commands
        .iter()
        .all(|line| line.contains("116319k") && line.contains("pad=1920:1080")));
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn blender_modes_render_single_pass_master_quality() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path();
    let tools = tools();
    let input = base.join("motion-source.mp4");
    fixture(
        &tools.ffmpeg,
        &input,
        "testsrc2=size=320x180:rate=120:duration=0.25",
        "libx264",
        None,
    );

    for mode in [RenderMode::FrameBlend, RenderMode::MotionBlur] {
        let mut conversion = job(&input, base, &tools);
        conversion.settings.render_mode = mode;
        conversion.settings.target_fps = 60;
        conversion.settings.blur_samples = 5;
        conversion.settings.shadow_lift = true;
        let events = Mutex::new(Vec::new());
        let commands = Mutex::new(Vec::new());
        let output = convert(conversion, CancellationToken::new(), |event| {
            if let Event::Progress { pass, passes, .. } = &event {
                events.lock().unwrap().push((*pass, *passes));
            }
            if let Event::Log { line } = event {
                if line.contains("libx264") {
                    commands.lock().unwrap().push(line);
                }
            }
        })
        .await
        .unwrap();

        let metadata = probe(&tools.ffprobe, &output.path, false);
        assert_eq!(metadata["streams"][0]["avg_frame_rate"], "60/1");
        assert_eq!(metadata["streams"][0]["nb_frames"], "15");
        assert!(output.shadow_lifted);
        assert!(events
            .into_inner()
            .unwrap()
            .iter()
            .all(|&(pass, passes)| pass == 1 && passes == 1));
        let command = commands.into_inner().unwrap().join("\n");
        assert!(command.contains("\"-crf\" \"3\""));
        assert!(command.contains("eq=gamma=1.18:gamma_weight=0.82"));
        assert!(!command.contains("\"-pass\""));
    }
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn inputs_progress_collision_and_errors() {
    let b = tools();
    let base = tempfile::tempdir().unwrap();
    let folder = base.path();
    let cases = [
        (
            "montaÅ¼ & test.mp4",
            "testsrc2=s=320x180:r=60:d=2",
            "libx264",
            Some("44100"),
            "60/1",
            120,
        ),
        (
            "sample 2.mov",
            "testsrc2=s=384x216:r=30000/1001:d=2",
            "hevc",
            Some("48000"),
            "30000/1001",
            60,
        ),
        (
            "silent.mkv",
            "testsrc2=s=240x136:r=25:d=2",
            "libx264",
            None,
            "25/1",
            50,
        ),
        (
            "web input.webm",
            "testsrc2=s=320x180:r=30:d=2",
            "libvpx-vp9",
            Some("48000"),
            "30/1",
            60,
        ),
    ];
    for (name, source, codec, rate, fps, frame_count) in cases {
        let input = folder.join(name);
        // The minimal release build decodes HEVC, but doesn't bundle libx265 for creating fixtures.
        fixture(
            &b.ffmpeg,
            &input,
            source,
            if codec == "hevc" { "libx265" } else { codec },
            rate,
        );
        let media = inspect(&input, &b, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(media.sample_rate, rate.map(|r| r.parse().unwrap()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let output = convert(
            job(&input, folder, &b),
            CancellationToken::new(),
            move |event| sink.lock().unwrap().push(event),
        )
        .await
        .unwrap();
        assert!(output.path.exists());
        let metadata = probe(&b.ffprobe, &output.path, false);
        let video = &metadata["streams"][0];
        assert_eq!(video["codec_name"], "h264");
        assert_eq!(video["profile"], "High");
        assert_eq!(video["level"], 50);
        assert_eq!(video["avg_frame_rate"], fps);
        assert_eq!(video["nb_frames"], frame_count.to_string());
        assert_eq!(video["width"], media.width);
        assert_eq!(video["height"], media.height);
        if let Some(rate) = rate {
            let audio = &metadata["streams"][1];
            assert_eq!(audio["codec_name"], "aac");
            assert_eq!(audio["profile"], "LC");
            // Finer normalizes every re-encoded track to 48 kHz stereo for TikTok.
            let _ = rate;
            assert_eq!(audio["sample_rate"], "48000");
        } else {
            assert_eq!(metadata["streams"].as_array().unwrap().len(), 1);
        }
        let events = events.lock().unwrap();
        let sizes: Vec<f64> = events
            .iter()
            .filter_map(|event| {
                if let Event::Progress { percent, .. } = event {
                    Some(*percent)
                } else {
                    None
                }
            })
            .collect();
        assert!(sizes.windows(2).all(|items| items[0] <= items[1]));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Progress { pass: 2, .. })));
        assert!(events.iter().any(|e| matches!(e, Event::Finalizing)));
        no_staging(folder);
    }

    let input = folder.join("montaÅ¼ & test.mp4");
    let old_file = folder.join("montaÅ¼ & test_finer.mp4");
    let old_length = fs::metadata(&old_file).unwrap().len();
    let output = convert(job(&input, folder, &b), CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(output.name, "montaÅ¼ & test_finer_2.mp4");
    assert_eq!(fs::metadata(&old_file).unwrap().len(), old_length);
    assert_eq!(fs::read_dir(folder.join("logs")).unwrap().count(), 5);
    no_staging(folder);

    let corrupt = folder.join("corrupt.mp4");
    fs::write(&corrupt, b"this isn't a video").unwrap();
    assert_eq!(
        inspect(&corrupt, &b, &CancellationToken::new())
            .await
            .unwrap_err()
            .code,
        "invalidInput"
    );
    let mut invalid = job(&input, folder, &b);
    invalid.directory = Some(folder.join("nonexistent"));
    assert_eq!(
        convert(invalid, CancellationToken::new(), |_| {})
            .await
            .unwrap_err()
            .code,
        "outputDirectory"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let read_only = folder.join("read-only");
        fs::create_dir(&read_only).unwrap();
        fs::set_permissions(&read_only, fs::Permissions::from_mode(0o555)).unwrap();
        let issue = convert(
            job(&input, &read_only, &b),
            CancellationToken::new(),
            |_| {},
        )
        .await;
        fs::set_permissions(&read_only, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(issue.unwrap_err().code, "permissionDenied");
        let limited = Path::new("/dev/shm");
        if let Ok(available) = fs2::available_space(limited) {
            // A low-frame-rate source gives a large expected output without a large fixture.
            let long = folder.join("long.mp4");
            let seconds = available / 2_000_000 + 30;
            fixture(
                &b.ffmpeg,
                &long,
                &format!("testsrc2=s=64x64:r=1:d={seconds}"),
                "libx264",
                None,
            );
            let mut limited_job = job(&long, limited, &b);
            limited_job.settings.keep_logs = false;
            assert_eq!(
                convert(limited_job, CancellationToken::new(), |_| {})
                    .await
                    .unwrap_err()
                    .code,
                "diskFull"
            );
        }
    }
}

#[test]
fn settings_round_trip_and_invalid_json() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("prefs/settings.json");
    let defaults = Settings::load(&path);
    assert_eq!(defaults.threads, 16);
    assert_eq!(defaults.video_bitrate_kbps, 16_617 * 7);
    assert!(defaults.use_source_directory);
    assert!(!defaults.gpu_encoding && defaults.gpu_decoding);
    let custom = Settings {
        theme: finer_engine::Theme::Dark,
        output_directory: Some(folder.path().to_owned()),
        use_source_directory: false,
        threads: 8,
        video_bitrate_kbps: 49_851,
        open_folder_after_conversion: true,
        gpu_encoding: false,
        gpu_decoding: true,
        keep_logs: true,
        ..defaults
    };
    custom.save(&path).unwrap();
    let round_trip = Settings::load(&path);
    assert_eq!(round_trip.threads, 8);
    assert_eq!(round_trip.video_bitrate_kbps, 49_851);
    assert_eq!(round_trip.output_directory, custom.output_directory);
    assert!(!round_trip.use_source_directory);
    assert!(round_trip.keep_logs);
    assert!(!round_trip.gpu_encoding && round_trip.gpu_decoding);
    assert!(matches!(round_trip.theme, finer_engine::Theme::Dark));
    fs::write(&path, r#"{"threads":8,"theme":"dark"}"#).unwrap();
    assert_eq!(Settings::load(&path).video_bitrate_kbps, 16_617 * 7);
    fs::write(&path, r#"{"threads":8,"videoBitrateKbps":100001}"#).unwrap();
    assert_eq!(Settings::load(&path).video_bitrate_kbps, 99_702);
    assert_eq!(Settings::load(&path).threads, 8);
    let invalid = Settings {
        video_bitrate_kbps: 100_001,
        ..Settings::default()
    };
    assert_eq!(invalid.save(&path).unwrap_err().code, "settings");
    Settings::default().save(&path).unwrap();
    assert_eq!(Settings::load(&path).threads, 16);
    fs::write(&path, "{garbled").unwrap();
    assert!(matches!(
        Settings::load(&path).theme,
        finer_engine::Theme::System
    ));
    fs::write(&path, r#"{"theme":"light","threads":0}"#).unwrap();
    let repaired = Settings::load(&path);
    assert_eq!(repaired.threads, 16);
    assert!(matches!(repaired.theme, finer_engine::Theme::Light));
}

#[test]
fn plans_queue_filenames_without_collisions() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path();
    fs::write(base.join("clip_finer.mp4"), "existing").unwrap();
    let paths = [base.join("clip.mov"), base.join("clip.mkv")];
    assert_eq!(
        plan_outputs(&paths, &Settings::default()),
        vec![
            Some(base.join("clip_finer_2.mp4")),
            Some(base.join("clip_finer_3.mp4"))
        ],
    );
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn level_and_audio_offset() {
    let b = tools();
    let base = tempfile::tempdir().unwrap();
    let folder = base.path();
    let large = folder.join("large.mp4");
    fixture(
        &b.ffmpeg,
        &large,
        "testsrc2=s=3840x2160:r=60:d=0.2",
        "libx264",
        None,
    );
    let output = convert(job(&large, folder, &b), CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        probe(&b.ffprobe, &output.path, false)["streams"][0]["level"],
        52
    );

    let sync = folder.join("offset.mp4");
    run(Command::new(&b.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=30:d=2",
            "-itsoffset",
            "0.35",
            "-f",
            "lavfi",
            "-i",
            "sine=sample_rate=48000:duration=1",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
        ])
        .arg(&sync));
    let output = convert(job(&sync, folder, &b), CancellationToken::new(), |_| {})
        .await
        .unwrap();
    let offset = |path: &Path| {
        let metadata = probe(&b.ffprobe, path, false);
        let start = |i: usize| {
            metadata["streams"][i]["start_time"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        };
        start(1) - start(0)
    };
    assert!((offset(&sync) - offset(&output.path)).abs() < 0.03);
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn variable_timestamps_and_cancellation() {
    let b = tools();
    let base = tempfile::tempdir().unwrap();
    let folder = base.path();
    let input = folder.join("variable.mp4");
    run(Command::new(&b.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=60:d=5",
            "-vf",
            "select=not(mod(n\\,2))+not(mod(n\\,5))",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-threads:v",
            "2",
        ])
        .arg(&input));
    let output = convert(job(&input, folder, &b), CancellationToken::new(), |_| {})
        .await
        .unwrap();
    let original = probe(&b.ffprobe, &input, true);
    let encoded = probe(&b.ffprobe, &output.path, true);
    let before = original["frames"].as_array().unwrap();
    let after = encoded["frames"].as_array().unwrap();
    assert_eq!(before.len(), after.len());
    for (a, b) in before.iter().zip(after) {
        assert_eq!(
            a["best_effort_timestamp_time"],
            b["best_effort_timestamp_time"]
        );
    }
    for pass in [1, 2] {
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let error = convert(job(&input, folder, &b), cancel, move |event| {
            if let Event::Progress {
                pass: current,
                percent,
                ..
            } = event
            {
                if current == pass && percent > f64::from(pass - 1) * 50.0 {
                    trigger.cancel();
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(!folder.join("variable_finer_2.mp4").exists());
        no_staging(folder);
    }
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn cfr_1080p60_against_reference_flags() {
    let b = tools();
    let base = tempfile::tempdir().unwrap();
    let folder = base.path();
    let input = folder.join("reference source.mp4");
    run(Command::new(&b.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=60:d=5,noise=alls=12:allf=t+u",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=750:sample_rate=48000",
            "-shortest",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "13",
            "-threads:v",
            "4",
            "-c:a",
            "aac",
            "-ac",
            "2",
        ])
        .arg(&input));
    let mut conversion = job(&input, folder, &b);
    conversion.settings.threads = 16;
    let output = convert(conversion, CancellationToken::new(), |_| {})
        .await
        .unwrap();
    let audio_frames: Value = serde_json::from_slice(
        &run(Command::new(&b.ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_frames",
                "-show_entries",
                "frame=channel_layout,pkt_size",
                "-of",
                "json",
            ])
            .arg(&output.path))
        .stdout,
    )
    .unwrap();
    let frames = audio_frames["frames"].as_array().unwrap();
    let (stereo, tail) = frames.split_at(frames.len() - 4);
    assert!(stereo
        .iter()
        .all(|frame| frame["channel_layout"] == "stereo"));
    assert!(tail.iter().all(|frame| {
        frame["pkt_size"] == "15" && frame["channel_layout"] == "2 channels (LFE+LFE2)"
    }));
    let reference = folder.join("reference output.mp4");
    for pass in ["1", "2"] {
        let mut cmd = Command::new(&b.ffmpeg);
        cmd.args(["-v", "error", "-y", "-i"])
            .arg(&input)
            .args(["-map", "0:v:0"]);
        if pass == "2" {
            cmd.args(["-map", "0:a:0?"]);
        }
        cmd.args([
            "-c:v",
            "libx264",
            "-preset",
            "slow",
            "-b:v",
            "116319k",
            "-profile:v",
            "high",
            "-level:v",
            "5.0",
            "-refs",
            "5",
            "-bf",
            "3",
            "-x264-params",
            "b-adapt=2:b-pyramid=2:rc-lookahead=50:mbtree=1:aq-mode=3:aq-strength=0.8:psy-rd=1.0,0.0",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "250",
            "-keyint_min",
            "25",
            "-sc_threshold",
            "40",
            "-threads",
            "16",
            "-pass",
            pass,
            "-passlogfile",
        ])
        .arg(folder.join("reference-pass"));
        if pass == "1" {
            cmd.args([
                "-an",
                "-f",
                "null",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ]);
        } else {
            cmd.args(["-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart"])
                .arg(&reference);
        }
        run(&mut cmd);
    }
    let keyframes = |path: &Path| {
        probe(&b.ffprobe, path, true)["frames"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["key_frame"] == 1)
            .map(|f| f["best_effort_timestamp_time"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(keyframes(&reference), keyframes(&output.path));
    let bitrate = |path: &Path| {
        let data = probe(&b.ffprobe, path, false);
        assert_eq!(data["streams"][0]["nb_frames"], "300");
        data["streams"][0]["bit_rate"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap()
    };
    let reference_rate = bitrate(&reference);
    let actual_rate = bitrate(&output.path);
    eprintln!(
        "Reference: {reference_rate:.0} bps; Finer: {actual_rate:.0} bps; keyframes: {:?}",
        keyframes(&output.path)
    );
    assert!(((reference_rate - actual_rate) / reference_rate).abs() < 0.015);
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn interrupted_second_pass_reuses_exact_stats() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path();
    let tools = tools();
    let input = base.join("retry.mp4");
    fixture(
        &tools.ffmpeg,
        &input,
        "testsrc2=s=320x180:r=60:d=2",
        "libx264",
        Some("48000"),
    );
    let baseline = convert(job(&input, base, &tools), CancellationToken::new(), |_| {})
        .await
        .unwrap();

    let mut checkpoint = None;
    for change in ["unchanged", "bitrate", "source"] {
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let error = convert_with_retry(
            job(&input, base, &tools),
            cancel,
            move |event| {
                if matches!(event, Event::Progress { pass: 2, percent, .. } if percent > 50.0) {
                    trigger.cancel();
                }
            },
            &mut checkpoint,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(checkpoint.is_some());

        let mut retry = job(&input, base, &tools);
        if change == "bitrate" {
            retry.settings.video_bitrate_kbps = 33_234;
        }
        if change == "source" {
            let mut bytes = fs::read(&input).unwrap();
            bytes.extend_from_slice(b"\0\0\0\x08free");
            fs::write(&input, bytes).unwrap();
        }
        let first_pass_ran = Mutex::new(false);
        let output = convert_with_retry(
            retry,
            CancellationToken::new(),
            |event| {
                if matches!(event, Event::Log { line } if line.starts_with("Pass 1 of 2")) {
                    *first_pass_ran.lock().unwrap() = true;
                }
            },
            &mut checkpoint,
        )
        .await
        .unwrap();
        assert_eq!(first_pass_ran.into_inner().unwrap(), change != "unchanged");
        assert!(checkpoint.is_none());
        if change == "unchanged" {
            assert_eq!(
                Sha256::digest(fs::read(&output.path).unwrap()),
                Sha256::digest(fs::read(&baseline.path).unwrap())
            );
        }
        no_staging(base);
    }
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn recognizes_patched_files_and_prevents_double_patching() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path();
    let tools = tools();
    let source = base.join("stereo.mp4");
    run(Command::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=30:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=750:sample_rate=48000",
            "-shortest",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-ac",
            "2",
        ])
        .arg(&source));

    let cancel = CancellationToken::new();
    let output = convert(job(&source, base, &tools), cancel.clone(), |_| {})
        .await
        .unwrap();
    assert_eq!(output.audio_patch, AudioPatch::Applied);
    assert_eq!(output.encoder, "libx264");
    assert_eq!(output.decoder, "cpu");

    let bytes = fs::read(&output.path).unwrap();
    let mut boxes = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let size = u32::from_be_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        assert!(size >= 8);
        boxes.push((&bytes[cursor + 4..cursor + 8], size));
        cursor += size;
    }
    assert_eq!(cursor, bytes.len());
    let index = boxes.iter().position(|(kind, _)| *kind == b"moov").unwrap();
    let media = boxes.iter().position(|(kind, _)| *kind == b"mdat").unwrap();
    assert!(index < media);
    assert_eq!(boxes.last().copied(), Some((&b"mdat"[..], 68)));

    assert!(
        inspect(&output.path, &tools, &cancel)
            .await
            .unwrap()
            .already_patched
    );
    let duplicate = convert(job(&output.path, base, &tools), cancel.clone(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(duplicate.code, "alreadyPatched");
    assert!(
        !inspect(&source, &tools, &cancel)
            .await
            .unwrap()
            .already_patched
    );
}

#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe 7.1+ with libx264 on PATH or set via environment"]
async fn reports_why_audio_patch_was_skipped() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path();
    let tools = tools();
    for (audio_rate, expected) in [
        (None, AudioPatch::NoAudio),
        (Some("44100"), AudioPatch::Applied),
    ] {
        let source = base.join(format!("source-{}.mp4", audio_rate.unwrap_or("silent")));
        fixture(
            &tools.ffmpeg,
            &source,
            "testsrc2=s=320x180:r=30:d=1",
            "libx264",
            audio_rate,
        );
        let output = convert(job(&source, base, &tools), CancellationToken::new(), |_| {})
            .await
            .unwrap();
        assert_eq!(output.audio_patch, expected);
    }
}
