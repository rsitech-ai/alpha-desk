use std::{
    ffi::OsString,
    io::{self, Read, Write as _},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
    io::Errno,
    process::{Pid, Signal, kill_process_group},
};
#[cfg(unix)]
use std::os::fd::AsFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
pub const TRUNCATION_MARKER: &str = "[... output truncated ...]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub evidence_program: Option<PathBuf>,
    pub arg0: Option<OsString>,
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
    #[cfg(unix)]
    if let Some(arg0) = &spec.arg0 {
        command.arg0(arg0);
    }
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
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_process_group(&mut child, spec.termination_grace)?;
            return Err(ProcessError::TerminationFailed(
                "stdout pipe was not created".to_owned(),
            ));
        }
    };
    let mut stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_process_group(&mut child, spec.termination_grace)?;
            return Err(ProcessError::TerminationFailed(
                "stderr pipe was not created".to_owned(),
            ));
        }
    };
    let deadline = started + spec.timeout;
    let (mut deadline_signal, mut deadline_waker) = match io::pipe() {
        Ok(pipe) => pipe,
        Err(source) => {
            terminate_process_group(&mut child, spec.termination_grace)?;
            return Err(ProcessError::OutputReadFailed {
                stream: "deadline",
                source,
            });
        }
    };
    if let Err(error) = set_nonblocking(&stdout)
        .and_then(|()| set_nonblocking(&stderr))
        .and_then(|()| set_nonblocking(&deadline_signal))
    {
        terminate_process_group(&mut child, spec.termination_grace)?;
        return Err(error);
    }
    thread::spawn(move || {
        thread::sleep(deadline.saturating_duration_since(Instant::now()));
        let _ = deadline_waker.write_all(&[1]);
    });
    let mut stdout_output = RawOutput::new(output_policy.max_bytes_per_stream);
    let mut stderr_output = RawOutput::new(output_policy.max_bytes_per_stream);
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut deadline_fired = false;
    let mut status = None;
    loop {
        match poll_command_fds(
            &stdout,
            &stderr,
            &deadline_signal,
            stdout_eof,
            stderr_eof,
            deadline,
        ) {
            Ok(fired) => deadline_fired |= fired,
            Err(error) => {
                terminate_process_group(&mut child, spec.termination_grace)?;
                return Err(error);
            }
        }
        deadline_fired |= deadline_signal_ready(&mut deadline_signal);
        if let Err(error) = drain_once(&mut stdout, &mut stdout_output, "stdout", &mut stdout_eof) {
            terminate_process_group(&mut child, spec.termination_grace)?;
            return Err(error);
        }
        if let Err(error) = drain_once(&mut stderr, &mut stderr_output, "stderr", &mut stderr_eof) {
            terminate_process_group(&mut child, spec.termination_grace)?;
            return Err(error);
        }
        if status.is_none() {
            status = match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    let original = ProcessError::WaitFailed(error);
                    terminate_process_group(&mut child, spec.termination_grace)?;
                    return Err(original);
                }
            };
        }
        if let Some(exit_status) = status
            && stdout_eof
            && stderr_eof
        {
            let stdout = protect_retained_output(stdout_output, output_policy);
            let stderr = protect_retained_output(stderr_output, output_policy);
            return Ok(CommandOutcome {
                success: exit_status.success(),
                exit_code: exit_status.code(),
                stdout,
                stderr,
                elapsed: started.elapsed(),
            });
        }
        if Instant::now() >= deadline || deadline_fired {
            observer(child.id(), ProcessObservation::TimeoutDetected);
            let _ = terminate_process_group(&mut child, spec.termination_grace);
            drop(stdout);
            drop(stderr);
            return Err(ProcessError::TimedOut {
                timeout: spec.timeout,
            });
        }
    }
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
    wait_until_deadline(child, Instant::now() + timeout)
}

fn wait_until_deadline(
    child: &mut Child,
    deadline: Instant,
) -> Result<Option<ExitStatus>, ProcessError> {
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
    let process_group = child.id();
    let _ = signal_process_group(process_group, Signal::TERM);
    let leader_exited = wait_until(child, termination_grace)?.is_some();
    let kill_result = signal_process_group(process_group, Signal::KILL);
    if leader_exited {
        return Ok(());
    }
    let _ = child.kill();
    if wait_until(child, termination_grace)?.is_some() {
        return Ok(());
    }
    match kill_result {
        Ok(()) => Err(ProcessError::TerminationFailed(
            "process group leader survived SIGKILL".to_owned(),
        )),
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn terminate_process_group(
    _child: &mut Child,
    _termination_grace: Duration,
) -> Result<(), ProcessError> {
    Err(ProcessError::UnsupportedPlatform)
}

#[cfg(unix)]
fn signal_process_group(process_group: u32, signal: Signal) -> Result<(), ProcessError> {
    let raw = i32::try_from(process_group).map_err(|_| {
        ProcessError::TerminationFailed(format!(
            "process group {process_group} is not a valid pid"
        ))
    })?;
    let pid = Pid::from_raw(raw).ok_or_else(|| {
        ProcessError::TerminationFailed(format!(
            "process group {process_group} is not a valid pid"
        ))
    })?;
    kill_process_group(pid, signal).map_err(|error| {
        ProcessError::TerminationFailed(format!(
            "{signal:?} for process group {process_group}: {error}"
        ))
    })
}

#[cfg(unix)]
fn poll_command_fds(
    stdout: impl AsFd,
    stderr: impl AsFd,
    deadline_signal: impl AsFd,
    stdout_eof: bool,
    stderr_eof: bool,
    deadline: Instant,
) -> Result<bool, ProcessError> {
    if Instant::now() >= deadline {
        return Ok(true);
    }
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .min(WAIT_POLL_INTERVAL);
    let Some(timeout) = Timespec::try_from(remaining).ok() else {
        return Ok(true);
    };
    if stdout_eof && stderr_eof {
        let mut fds = [PollFd::new(&deadline_signal, PollFlags::IN | PollFlags::HUP)];
        return poll_deadline(&mut fds, Some(&timeout), deadline);
    }
    let mut fds = [
        PollFd::new(&stdout, PollFlags::IN | PollFlags::HUP),
        PollFd::new(&stderr, PollFlags::IN | PollFlags::HUP),
        PollFd::new(&deadline_signal, PollFlags::IN | PollFlags::HUP),
    ];
    if stdout_eof {
        fds[0] = PollFd::new(&deadline_signal, PollFlags::IN | PollFlags::HUP);
    }
    if stderr_eof {
        fds[1] = PollFd::new(&deadline_signal, PollFlags::IN | PollFlags::HUP);
    }
    poll_deadline(&mut fds, Some(&timeout), deadline)
}

#[cfg(not(unix))]
fn poll_command_fds(
    _stdout: &impl Sized,
    _stderr: &impl Sized,
    _deadline_signal: &impl Sized,
    _stdout_eof: bool,
    _stderr_eof: bool,
    _deadline: Instant,
) -> Result<bool, ProcessError> {
    Err(ProcessError::UnsupportedPlatform)
}

#[cfg(not(unix))]
fn deadline_signal_ready(_deadline_signal: &mut impl Read) -> bool {
    false
}

#[cfg(unix)]
fn poll_deadline(
    fds: &mut [PollFd<'_>],
    timeout: Option<&Timespec>,
    deadline: Instant,
) -> Result<bool, ProcessError> {
    match poll(fds, timeout) {
        Ok(_) => Ok(Instant::now() >= deadline),
        Err(error) if error == Errno::INTR => Ok(Instant::now() >= deadline),
        Err(error) => Err(ProcessError::OutputReadFailed {
            stream: "stdio",
            source: io::Error::from_raw_os_error(error.raw_os_error()),
        }),
    }
}

#[cfg(unix)]
fn deadline_signal_ready(deadline_signal: &mut impl Read) -> bool {
    let mut byte = [0_u8; 1];
    matches!(deadline_signal.read(&mut byte), Ok(1..))
}

#[cfg(unix)]
fn set_nonblocking(fd: impl AsFd) -> Result<(), ProcessError> {
    let flags = fcntl_getfl(&fd).map_err(|source| ProcessError::OutputReadFailed {
        stream: "stdio",
        source: io::Error::from(source),
    })?;
    fcntl_setfl(&fd, flags | OFlags::NONBLOCK).map_err(|source| ProcessError::OutputReadFailed {
        stream: "stdio",
        source: io::Error::from(source),
    })
}

#[cfg(not(unix))]
fn set_nonblocking(_fd: &impl Sized) -> Result<(), ProcessError> {
    Err(ProcessError::UnsupportedPlatform)
}

fn drain_once(
    stream: &mut impl Read,
    output: &mut RawOutput,
    name: &'static str,
    eof: &mut bool,
) -> Result<bool, ProcessError> {
    if *eof {
        return Ok(false);
    }
    let mut buffer = [0_u8; 8192];
    match stream.read(&mut buffer) {
        Ok(0) => {
            *eof = true;
            Ok(true)
        }
        Ok(count) => {
            output.total_bytes = output.total_bytes.saturating_add(count);
            let remaining = output.limit.saturating_sub(output.retained.len());
            output
                .retained
                .extend_from_slice(&buffer[..count.min(remaining)]);
            Ok(true)
        }
        Err(source)
            if source.kind() == io::ErrorKind::WouldBlock
                || source.kind() == io::ErrorKind::Interrupted =>
        {
            Ok(false)
        }
        Err(source) => Err(ProcessError::OutputReadFailed {
            stream: name,
            source,
        }),
    }
}

struct RawOutput {
    retained: Vec<u8>,
    total_bytes: usize,
    limit: usize,
}

impl RawOutput {
    fn new(limit: usize) -> Self {
        Self {
            retained: Vec::with_capacity(limit.min(8192)),
            total_bytes: 0,
            limit,
        }
    }
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
