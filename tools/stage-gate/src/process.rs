use std::{
    ffi::OsString,
    io::{self, Read},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
pub const TRUNCATION_MARKER: &str = "[... output truncated ...]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub termination_grace: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputPolicy {
    pub max_bytes_per_stream: usize,
    pub redactions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedOutput {
    pub text: String,
    pub total_bytes: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct CommandOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
    pub elapsed: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessPlatform {
    UnixProcessGroups,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessObservation {
    TimeoutDetected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessErrorCode {
    UnsupportedPlatform,
    SpawnFailed,
    OutputReadFailed,
    WaitFailed,
    TerminationFailed,
    TimedOut,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("the platform does not provide the required process-group lifecycle")]
    UnsupportedPlatform,
    #[error("failed to spawn {program}: {source}")]
    SpawnFailed {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read child {stream}: {source}")]
    OutputReadFailed {
        stream: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("failed to wait for child process: {0}")]
    WaitFailed(#[source] io::Error),
    #[error("failed to terminate child process group: {0}")]
    TerminationFailed(String),
    #[error("command timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
}

impl ProcessError {
    #[must_use]
    pub const fn code(&self) -> ProcessErrorCode {
        match self {
            Self::UnsupportedPlatform => ProcessErrorCode::UnsupportedPlatform,
            Self::SpawnFailed { .. } => ProcessErrorCode::SpawnFailed,
            Self::OutputReadFailed { .. } => ProcessErrorCode::OutputReadFailed,
            Self::WaitFailed(_) => ProcessErrorCode::WaitFailed,
            Self::TerminationFailed(_) => ProcessErrorCode::TerminationFailed,
            Self::TimedOut { .. } => ProcessErrorCode::TimedOut,
        }
    }
}

pub fn ensure_platform_support(platform: ProcessPlatform) -> Result<(), ProcessError> {
    match platform {
        ProcessPlatform::UnixProcessGroups => Ok(()),
        ProcessPlatform::Unsupported => Err(ProcessError::UnsupportedPlatform),
    }
}

pub fn run_command(
    spec: &CommandSpec,
    output_policy: &OutputPolicy,
) -> Result<CommandOutcome, ProcessError> {
    run_command_observed(spec, output_policy, |_, _| {})
}

#[doc(hidden)]
pub fn run_command_observed<F>(
    spec: &CommandSpec,
    output_policy: &OutputPolicy,
    mut observer: F,
) -> Result<CommandOutcome, ProcessError>
where
    F: FnMut(u32, ProcessObservation),
{
    ensure_platform_support(current_platform())?;

    let started = Instant::now();
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command)?;

    let mut child = command
        .spawn()
        .map_err(|source| ProcessError::SpawnFailed {
            program: spec.program.clone(),
            source,
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessError::TerminationFailed("stdout pipe was not created".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessError::TerminationFailed("stderr pipe was not created".to_owned()))?;
    let stdout_limit = output_policy.max_bytes_per_stream;
    let stderr_limit = output_policy.max_bytes_per_stream;
    let stdout_reader = thread::spawn(move || drain_stream(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || drain_stream(stderr, stderr_limit));

    let status = match wait_until(&mut child, spec.timeout)? {
        Some(status) => status,
        None => {
            observer(child.id(), ProcessObservation::TimeoutDetected);
            terminate_process_group(&mut child, spec.termination_grace)?;
            let _ = join_reader(stdout_reader, "stdout")?;
            let _ = join_reader(stderr_reader, "stderr")?;
            return Err(ProcessError::TimedOut {
                timeout: spec.timeout,
            });
        }
    };

    let stdout = protect_retained_output(join_reader(stdout_reader, "stdout")?, output_policy);
    let stderr = protect_retained_output(join_reader(stderr_reader, "stderr")?, output_policy);
    Ok(CommandOutcome {
        success: status.success(),
        exit_code: status.code(),
        stdout,
        stderr,
        elapsed: started.elapsed(),
    })
}

fn current_platform() -> ProcessPlatform {
    #[cfg(unix)]
    {
        ProcessPlatform::UnixProcessGroups
    }
    #[cfg(not(unix))]
    {
        ProcessPlatform::Unsupported
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) -> Result<(), ProcessError> {
    command.process_group(0);
    Ok(())
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) -> Result<(), ProcessError> {
    Err(ProcessError::UnsupportedPlatform)
}

fn wait_until(child: &mut Child, timeout: Duration) -> Result<Option<ExitStatus>, ProcessError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(ProcessError::WaitFailed)? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(WAIT_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

#[cfg(unix)]
fn terminate_process_group(
    child: &mut Child,
    termination_grace: Duration,
) -> Result<(), ProcessError> {
    let _term_result = signal_process_group(child.id(), "TERM");
    let grace_started = Instant::now();
    let leader_exited = wait_until(child, termination_grace)?.is_some();
    thread::sleep(termination_grace.saturating_sub(grace_started.elapsed()));
    let kill_result = signal_process_group(child.id(), "KILL");
    if let Err(error) = kill_result
        && !leader_exited
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    if !leader_exited {
        child.wait().map_err(ProcessError::WaitFailed)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate_process_group(
    _child: &mut Child,
    _termination_grace: Duration,
) -> Result<(), ProcessError> {
    Err(ProcessError::UnsupportedPlatform)
}

#[cfg(unix)]
fn signal_process_group(process_group: u32, signal: &str) -> Result<(), ProcessError> {
    let status = Command::new("/bin/kill")
        .env_clear()
        .args([format!("-{signal}"), format!("-{process_group}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| ProcessError::TerminationFailed(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(ProcessError::TerminationFailed(format!(
            "{signal} for process group {process_group} exited {status}"
        )))
    }
}

fn drain_stream(mut stream: impl Read, limit: usize) -> io::Result<RawOutput> {
    let mut retained = Vec::with_capacity(limit.min(8192));
    let mut total_bytes = 0usize;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(count);
        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..count.min(remaining)]);
    }
    Ok(RawOutput {
        retained,
        total_bytes,
    })
}

fn join_reader(
    reader: thread::JoinHandle<io::Result<RawOutput>>,
    stream: &'static str,
) -> Result<RawOutput, ProcessError> {
    reader
        .join()
        .map_err(|_| ProcessError::TerminationFailed(format!("{stream} reader panicked")))?
        .map_err(|source| ProcessError::OutputReadFailed { stream, source })
}

struct RawOutput {
    retained: Vec<u8>,
    total_bytes: usize,
}

fn protect_retained_output(raw: RawOutput, policy: &OutputPolicy) -> CapturedOutput {
    let mut text = String::from_utf8_lossy(&raw.retained).into_owned();
    for secret in policy.redactions.iter().filter(|secret| !secret.is_empty()) {
        text = text.replace(secret, "[REDACTED]");
    }
    let truncated = raw.total_bytes > raw.retained.len();
    if truncated {
        let content_limit = policy
            .max_bytes_per_stream
            .saturating_sub(TRUNCATION_MARKER.len());
        truncate_string(&mut text, content_limit);
        if policy.max_bytes_per_stream >= TRUNCATION_MARKER.len() {
            text.push_str(TRUNCATION_MARKER);
        }
    } else {
        truncate_string(&mut text, policy.max_bytes_per_stream);
    }
    CapturedOutput {
        text,
        total_bytes: raw.total_bytes,
        truncated,
    }
}

fn truncate_string(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}
