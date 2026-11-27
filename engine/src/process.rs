use crate::Error;
use std::{
    io,
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

pub fn command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    command
}

pub fn spawn(command: &mut Command) -> Result<(Child, ProcessTree), Error> {
    let mut child = command
        .spawn()
        .map_err(|e| Error::io(e, "missingTool", "Could not start FFmpeg or FFprobe."))?;
    let tree = ProcessTree::attach(&child).map_err(|e| {
        let _ = child.start_kill();
        Error::io(e, "process", "Could not manage the video process.")
    })?;
    Ok((child, tree))
}

pub async fn capture(command: &mut Command, cancel: &CancellationToken) -> Result<Output, Error> {
    let (mut child, tree) = spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let read_output = async {
        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let (status, _, _) = tokio::try_join!(
            child.wait(),
            stdout.read_to_end(&mut stdout_bytes),
            stderr.read_to_end(&mut stderr_bytes),
        )?;
        Ok::<_, io::Error>(Output {
            status,
            stdout: stdout_bytes,
            stderr: stderr_bytes,
        })
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(Error::cancelled()),
        result = tokio::time::timeout(Duration::from_secs(30), read_output) => {
            match result {
                Ok(result) => result.map_err(|e| Error::io(e, "process", "Could not read the video process.")),
                Err(e) => Err(Error::new("probeTimeout", "The video took too long to inspect.", e)),
            }
        }
    };
    tree.close();
    if result.is_err() {
        let _ = child.kill().await;
    }
    result
}

#[cfg(not(windows))]
pub struct ProcessTree;

#[cfg(not(windows))]
impl ProcessTree {
    fn attach(_: &Child) -> io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(windows)]
pub struct ProcessTree(windows_sys::Win32::Foundation::HANDLE);

impl ProcessTree {
    pub fn close(self) {}
}

// The job handle is owned by this object and can be closed from any thread.
#[cfg(windows)]
unsafe impl Send for ProcessTree {}

#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &Child) -> io::Result<Self> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            let tree = Self(raw);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(
                    raw,
                    child
                        .raw_handle()
                        .ok_or_else(|| io::Error::other("process handle unavailable"))?,
                ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(tree)
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
