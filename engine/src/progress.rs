use crate::{media::ffmpeg_error, process, Error};
use serde::Serialize;
use std::{collections::VecDeque, fs::File, io::Write, process::Stdio, time::Instant};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    Progress {
        pass: u8,
        percent: f64,
        pass_percent: f64,
        elapsed: f64,
        speed: Option<f64>,
        passes: u8,
        encoder: &'static str,
    },
    Log {
        line: String,
    },
    Finalizing,
}

type Reporter<'a> = dyn Fn(Event) + Send + Sync + 'a;

pub(crate) struct Log<'a> {
    pub(crate) report: &'a Reporter<'a>,
    file: Option<File>,
    tail: VecDeque<String>,
}

impl<'a> Log<'a> {
    pub(crate) fn new(report: &'a Reporter<'a>, file: Option<File>) -> Self {
        Self {
            report,
            file,
            tail: VecDeque::new(),
        }
    }

    pub(crate) fn line(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        if let Some(file) = &mut self.file {
            if writeln!(file, "{line}").is_err() {
                self.file = None;
                (self.report)(Event::Log {
                    line: "Could not continue writing the saved log.".into(),
                });
            }
        }
        if self.tail.len() == 20 {
            self.tail.pop_front();
        }
        self.tail.push_back(line.clone());
        (self.report)(Event::Log { line });
    }
}

pub(crate) async fn run_pass(
    command: &mut Command,
    progress: &mut Progress,
    log: &mut Log<'_>,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    log.tail.clear();
    let (mut child, tree) = process::spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()))?;
    let mut stdout = BufReader::new(child.stdout.take().expect("piped")).lines();
    let mut stderr = BufReader::new(child.stderr.take().expect("piped"));
    let mut err_line = Vec::new();
    let (mut stdout_open, mut stderr_open) = (true, true);
    let consume = async {
        while stdout_open || stderr_open {
            tokio::select! {
                result = stdout.next_line(), if stdout_open => {
                    match result {
                        Ok(Some(line)) => progress.read(&line, log.report),
                        Ok(None) => stdout_open = false,
                        Err(error) => return Err(Error::io(error, "process", "Could not read encoding progress.")),
                    }
                }
                result = stderr.read_until(b'\n', &mut err_line), if stderr_open => {
                    match result {
                        Ok(0) => stderr_open = false,
                        Ok(_) => {
                            log.line(String::from_utf8_lossy(&err_line).trim_end().to_owned());
                            err_line.clear();
                        }
                        Err(error) => return Err(Error::io(error, "process", "Could not read encoding logs.")),
                    }
                }
            }
        }
        child
            .wait()
            .await
            .map_err(|e| Error::io(e, "process", "The video process ended unexpectedly."))
    };
    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(Error::cancelled()),
        result = consume => result,
    };
    tree.close();
    if outcome.is_err() {
        let _ = child.kill().await;
    }
    let status = outcome?;
    if !status.success() {
        let detail = format!("{status}\n{}", self_contained_tail(&log.tail));
        let crashed = status.code().is_none_or(|code| code < 0);
        return Err(if crashed {
            Error::new("process", "The video process ended unexpectedly.", detail)
        } else {
            ffmpeg_error(
                &detail,
                "encoder",
                "FFmpeg could not finish encoding this video.",
            )
        });
    }
    Ok(())
}

fn self_contained_tail(tail: &VecDeque<String>) -> String {
    tail.iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) struct Progress {
    pass: u8,
    passes: u8,
    encoder: &'static str,
    duration: f64,
    started: Instant,
    pass_started: Instant,
    position: f64,
    pub(crate) frame: u64,
    pub(crate) speed: Option<f64>,
}

impl Progress {
    pub(crate) fn new(
        pass: u8,
        passes: u8,
        encoder: &'static str,
        duration: f64,
        started: Instant,
    ) -> Self {
        Self {
            pass,
            passes,
            encoder,
            duration,
            started,
            pass_started: Instant::now(),
            position: 0.0,
            frame: 0,
            speed: None,
        }
    }

    fn read(&mut self, line: &str, report: &Reporter<'_>) {
        let Some((key, value)) = line.split_once('=') else {
            return;
        };
        match key {
            "frame" => self.frame = value.trim().parse().unwrap_or(self.frame),
            "out_time_us" => {
                if let Ok(us) = value.parse::<i64>() {
                    self.position = self
                        .position
                        .max((us as f64 / 1_000_000.0).min(self.duration));
                }
            }
            "progress" => {
                let elapsed = self.pass_started.elapsed().as_secs_f64();
                if elapsed >= 1.5 && self.position / self.duration >= 0.03 {
                    let measured = self.position / elapsed;
                    self.speed = Some(
                        self.speed
                            .map_or(measured, |speed| speed * 0.75 + measured * 0.25),
                    );
                }
                self.emit(report, false);
            }
            _ => {}
        }
    }

    pub(crate) fn emit(&self, report: &Reporter<'_>, done: bool) {
        let part = if done {
            1.0
        } else {
            (self.position / self.duration).clamp(0.0, 0.998)
        };
        report(Event::Progress {
            pass: self.pass,
            passes: self.passes,
            encoder: self.encoder,
            percent: ((f64::from(self.pass - 1) + part) * 100.0 / f64::from(self.passes)).min(99.9),
            pass_percent: part * 100.0,
            elapsed: self.started.elapsed().as_secs_f64(),
            speed: self.speed,
        });
    }

    pub(crate) fn finish(&mut self, report: &Reporter<'_>) {
        let elapsed = self.pass_started.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.speed = Some(self.duration / elapsed);
        }
        self.emit(report, true);
    }

    pub(crate) fn resume(started: Instant, speed: Option<f64>, report: &Reporter<'_>) {
        report(Event::Progress {
            pass: 1,
            passes: 2,
            encoder: "libx264",
            percent: 50.0,
            pass_percent: 100.0,
            elapsed: started.elapsed().as_secs_f64(),
            speed,
        });
    }
}
