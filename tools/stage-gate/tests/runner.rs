#![cfg(unix)]

use std::{
    fs,
    os::unix::process::CommandExt as _,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use stage_gate::{
    process::{
        CommandSpec, OutputPolicy, ProcessErrorCode, ProcessObservation, ProcessPlatform,
        ensure_platform_support, run_command, run_command_observed,
    },
    runner::{
        DesignExpectation, RepositoryErrorCode, RepositorySnapshot, RunnerErrorCode,
        run_guarded_checks,
    },
};
use tempfile::TempDir;

const REDACTED: &str = "[REDACTED]";

#[test]
fn dirty_repository_is_rejected() {
    let repository = TestRepository::new();
    fs::write(repository.path().join("dirty.txt"), "not committed\n").unwrap();

    let error = RepositorySnapshot::capture(repository.path(), &repository.design())
        .expect_err("dirty repository must fail closed");

    assert_eq!(error.code(), RepositoryErrorCode::DirtyTree);
}

#[test]
fn head_drift_is_detected_after_each_command_and_stops_the_run() {
    let repository = TestRepository::new();
    let original_head = repository.head();
    let replacement_head = repository.add_commit("replacement.txt", "replacement\n");
    repository.checkout(&original_head);
    let snapshot = RepositorySnapshot::capture(repository.path(), &repository.design()).unwrap();
    let marker = repository.temp.path().join("second-command-ran");

    let drift = helper_spec(
        repository.path(),
        "drift-head",
        vec![
            (
                "STAGE_GATE_HELPER_REPO".to_owned(),
                repository.path().to_string_lossy().into_owned(),
            ),
            ("STAGE_GATE_HELPER_TARGET".to_owned(), replacement_head),
        ],
    );
    let second = helper_spec(
        repository.path(),
        "write-marker",
        vec![(
            "STAGE_GATE_HELPER_MARKER".to_owned(),
            marker.to_string_lossy().into_owned(),
        )],
    );

    let error = run_guarded_checks(
        repository.path(),
        &snapshot,
        &[drift, second],
        &output_policy(),
        Duration::from_secs(10),
    )
    .expect_err("HEAD drift must stop the run immediately");

    assert_eq!(error.code(), RunnerErrorCode::RepositoryChanged);
    assert_eq!(
        error.repository_code(),
        Some(RepositoryErrorCode::HeadChanged)
    );
    assert!(!marker.exists(), "the next command must not be started");
}

#[test]
fn design_tag_object_mismatch_is_rejected() {
    let repository = TestRepository::new();
    let mut design = repository.design();
    design.tag_object = "0000000000000000000000000000000000000000".to_owned();

    let error = RepositorySnapshot::capture(repository.path(), &design)
        .expect_err("the annotated tag object must match exactly");

    assert_eq!(error.code(), RepositoryErrorCode::DesignTagObjectMismatch);
}

#[test]
fn design_tag_peel_mismatch_is_rejected() {
    let repository = TestRepository::new();
    let mut design = repository.design();
    design.commit = "0000000000000000000000000000000000000000".to_owned();

    let error = RepositorySnapshot::capture(repository.path(), &design)
        .expect_err("the tag must peel to the pinned commit");

    assert_eq!(error.code(), RepositoryErrorCode::DesignCommitMismatch);
}

#[test]
fn non_zero_exit_is_reported_without_losing_output() {
    let command = helper_spec(Path::new("."), "non-zero", Vec::new());

    let outcome = run_command(&command, &output_policy()).unwrap();

    assert_eq!(outcome.exit_code, Some(23));
    assert!(!outcome.success);
    assert!(outcome.stdout.text.contains("before failure"));
    assert!(outcome.stderr.text.contains("failure detail"));
}

#[test]
fn child_environment_is_cleared_before_explicit_values_are_added() {
    let command = helper_spec(Path::new("."), "inspect-environment", Vec::new());

    let outcome = run_command(&command, &output_policy()).unwrap();

    assert!(outcome.success);
    assert!(outcome.stdout.text.contains("HOME=false PATH=false"));
}

#[test]
fn large_stdout_and_stderr_are_drained_concurrently() {
    let command = helper_spec(Path::new("."), "large-dual-output", Vec::new());
    let policy = OutputPolicy {
        max_bytes_per_stream: 2 * 1024 * 1024,
        redactions: Vec::new(),
    };

    let outcome = run_command(&command, &policy).unwrap();

    assert!(outcome.success);
    assert!(outcome.stdout.total_bytes >= 1024 * 1024);
    assert!(outcome.stderr.total_bytes >= 1024 * 1024);
    assert!(!outcome.stdout.truncated);
    assert!(!outcome.stderr.truncated);
}

#[test]
fn retained_output_is_bounded_and_redacted() {
    let secret = "stage-gate-secret";
    let command = helper_spec(
        Path::new("."),
        "secret-output",
        vec![("STAGE_GATE_HELPER_SECRET".to_owned(), secret.to_owned())],
    );
    let policy = OutputPolicy {
        max_bytes_per_stream: 64,
        redactions: vec![secret.to_owned()],
    };

    let outcome = run_command(&command, &policy).unwrap();

    assert!(outcome.stdout.truncated);
    assert!(outcome.stdout.total_bytes > outcome.stdout.text.len());
    assert!(outcome.stdout.text.len() <= 64);
    assert!(!outcome.stdout.text.contains(secret));
    assert!(outcome.stdout.text.contains(REDACTED));
    assert!(outcome.stdout.text.contains("[... output truncated ...]"));
}

#[test]
fn timeout_terminates_and_reaps_the_descendant_process_group() {
    let temp = TempDir::new().unwrap();
    let pid_path = temp.path().join("descendant.pid");
    let mut command = helper_spec(
        Path::new("."),
        "spawn-descendant",
        vec![(
            "STAGE_GATE_HELPER_PID_PATH".to_owned(),
            pid_path.to_string_lossy().into_owned(),
        )],
    );
    command.timeout = Duration::from_millis(250);
    command.termination_grace = Duration::from_millis(100);

    let error = run_command(&command, &output_policy())
        .expect_err("the command and its descendants must time out");

    assert_eq!(error.code(), ProcessErrorCode::TimedOut);
    let pid = wait_for_pid(&pid_path);
    wait_until_process_is_gone(pid);
}

#[test]
fn unsupported_platform_fails_closed() {
    let error = ensure_platform_support(ProcessPlatform::Unsupported)
        .expect_err("unsupported process-group platforms must fail closed");

    assert_eq!(error.code(), ProcessErrorCode::UnsupportedPlatform);
}

#[test]
fn dirty_submodule_is_not_ignored() {
    let repository = TestRepository::new();
    let submodule = TestRepository::new();
    let status = Command::new(git_program())
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("-C")
        .arg(repository.path())
        .args(["submodule", "add", "-q"])
        .arg(submodule.path())
        .arg("vendor/submodule")
        .status()
        .unwrap();
    assert!(status.success());
    git(repository.path(), ["commit", "-q", "-am", "add submodule"]);
    fs::write(
        repository.path().join("vendor/submodule/tracked.txt"),
        "dirty submodule\n",
    )
    .unwrap();

    let error = RepositorySnapshot::capture(repository.path(), &repository.design())
        .expect_err("dirty submodules must be included in the exact clean check");

    assert_eq!(error.code(), RepositoryErrorCode::DirtyTree);
}

#[test]
fn captured_head_is_the_verified_peeled_commit() {
    let repository = TestRepository::new();
    let expected = git_output(
        repository.path(),
        ["rev-parse", "--verify", "HEAD^{commit}"],
    );

    let snapshot = RepositorySnapshot::capture(repository.path(), &repository.design()).unwrap();

    assert_eq!(snapshot.head(), expected);
}

#[test]
fn guarded_check_rejects_canonical_working_directory_outside_repository() {
    let repository = TestRepository::new();
    let snapshot = RepositorySnapshot::capture(repository.path(), &repository.design()).unwrap();
    let outside = TempDir::new().unwrap();
    let marker = outside.path().join("must-not-run");
    let command = helper_spec(
        outside.path(),
        "write-marker",
        vec![(
            "STAGE_GATE_HELPER_MARKER".to_owned(),
            marker.to_string_lossy().into_owned(),
        )],
    );

    let error = run_guarded_checks(
        repository.path(),
        &snapshot,
        &[command],
        &output_policy(),
        Duration::from_secs(5),
    )
    .expect_err("cwd outside the repository must fail before spawn");

    assert_eq!(error.code(), RunnerErrorCode::UnsafeWorkingDirectory);
    assert!(!marker.exists());
}

#[test]
fn whole_gate_deadline_is_enforced_independently_of_check_timeout() {
    let repository = TestRepository::new();
    let snapshot = RepositorySnapshot::capture(repository.path(), &repository.design()).unwrap();
    let mut command = helper_spec(repository.path(), "sleep", Vec::new());
    command.timeout = Duration::from_secs(5);

    let error = run_guarded_checks(
        repository.path(),
        &snapshot,
        &[command],
        &output_policy(),
        Duration::from_millis(100),
    )
    .expect_err("the whole-gate deadline must bound individual checks");

    assert_eq!(error.code(), RunnerErrorCode::GateDeadlineExceeded);
}

#[test]
fn non_zero_check_fails_the_guarded_run() {
    let repository = TestRepository::new();
    let snapshot = RepositorySnapshot::capture(repository.path(), &repository.design()).unwrap();
    let command = helper_spec(repository.path(), "non-zero", Vec::new());

    let error = run_guarded_checks(
        repository.path(),
        &snapshot,
        &[command],
        &output_policy(),
        Duration::from_secs(5),
    )
    .expect_err("non-zero checks must fail the overall run");

    assert_eq!(error.code(), RunnerErrorCode::NonZeroExit);
}

#[test]
fn leader_exit_after_timeout_detection_still_kills_descendants_and_reaps() {
    let temp = TempDir::new().unwrap();
    let pid_path = temp.path().join("race-descendant.pid");
    let mut command = helper_spec(
        Path::new("."),
        "spawn-descendant",
        vec![(
            "STAGE_GATE_HELPER_PID_PATH".to_owned(),
            pid_path.to_string_lossy().into_owned(),
        )],
    );
    command.timeout = Duration::from_millis(100);
    command.termination_grace = Duration::from_millis(50);

    let error = run_command_observed(&command, &output_policy(), |leader, observation| {
        if observation == ProcessObservation::TimeoutDetected {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &leader.to_string()])
                .status();
        }
    })
    .expect_err("the timeout race must remain a timeout");

    assert_eq!(error.code(), ProcessErrorCode::TimedOut);
    let descendant = wait_for_pid(&pid_path);
    wait_until_process_is_gone(descendant);
}

#[test]
fn early_zero_leader_with_detached_inherited_pipe_is_bounded_by_command_deadline() {
    let temp = TempDir::new().unwrap();
    let pid_path = temp.path().join("detached-descendant.pid");
    let mut command = helper_spec(
        Path::new("."),
        "exit-zero-with-detached-inherited-pipe",
        vec![(
            "STAGE_GATE_HELPER_PID_PATH".to_owned(),
            pid_path.to_string_lossy().into_owned(),
        )],
    );
    command.timeout = Duration::from_millis(100);
    command.termination_grace = Duration::from_millis(100);

    let started = Instant::now();
    let result = run_command(&command, &output_policy());
    let elapsed = started.elapsed();
    let descendant = wait_for_pid(&pid_path);
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &descendant.to_string()])
        .status();

    let error = result.expect_err("inherited output pipes must not outlive the command deadline");
    assert_eq!(error.code(), ProcessErrorCode::TimedOut);
    assert!(
        elapsed < Duration::from_secs(1),
        "command deadline plus bounded grace took {elapsed:?}"
    );
}

#[test]
fn inherited_pipe_drain_cannot_overrun_the_whole_gate_deadline() {
    let repository = TestRepository::new();
    let snapshot = RepositorySnapshot::capture(repository.path(), &repository.design()).unwrap();
    let pid_path = repository.path().join(".git/detached-descendant.pid");
    let mut command = helper_spec(
        repository.path(),
        "exit-zero-with-detached-inherited-pipe",
        vec![(
            "STAGE_GATE_HELPER_PID_PATH".to_owned(),
            pid_path.to_string_lossy().into_owned(),
        )],
    );
    command.timeout = Duration::from_secs(5);
    command.termination_grace = Duration::from_millis(100);

    let started = Instant::now();
    let result = run_guarded_checks(
        repository.path(),
        &snapshot,
        &[command],
        &output_policy(),
        Duration::from_millis(100),
    );
    let elapsed = started.elapsed();
    let descendant = wait_for_pid(&pid_path);
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &descendant.to_string()])
        .status();

    let error = result.expect_err("the whole gate deadline must include output draining");
    assert_eq!(error.code(), RunnerErrorCode::GateDeadlineExceeded);
    assert!(
        elapsed < Duration::from_secs(1),
        "whole-gate deadline plus bounded grace took {elapsed:?}"
    );
}

#[test]
#[allow(clippy::zombie_processes)] // Detached-pipe fixture exits without owning the child handle.
fn stage_gate_process_helper() {
    let Ok(mode) = std::env::var("STAGE_GATE_HELPER_MODE") else {
        return;
    };

    match mode.as_str() {
        "drift-head" => {
            let status = Command::new(git_program())
                .arg("-C")
                .arg(required_env("STAGE_GATE_HELPER_REPO"))
                .args(["update-ref", "HEAD"])
                .arg(required_env("STAGE_GATE_HELPER_TARGET"))
                .status()
                .unwrap();
            std::process::exit(if status.success() { 0 } else { 91 });
        }
        "write-marker" => {
            fs::write(required_env("STAGE_GATE_HELPER_MARKER"), b"ran\n").unwrap();
        }
        "non-zero" => {
            println!("before failure");
            eprintln!("failure detail");
            std::process::exit(23);
        }
        "inspect-environment" => {
            println!(
                "HOME={} PATH={}",
                std::env::var_os("HOME").is_some(),
                std::env::var_os("PATH").is_some()
            );
        }
        "large-dual-output" => {
            let stdout = std::io::stdout();
            let stderr = std::io::stderr();
            write_repeated(stdout.lock(), b'o', 1024 * 1024);
            write_repeated(stderr.lock(), b'e', 1024 * 1024);
        }
        "secret-output" => {
            let secret = required_env("STAGE_GATE_HELPER_SECRET");
            print!("{secret}:");
            print!("{}", "x".repeat(4096));
        }
        "spawn-descendant" => {
            let mut child = Command::new("/bin/sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            fs::write(
                required_env("STAGE_GATE_HELPER_PID_PATH"),
                format!("{}\n", child.id()),
            )
            .unwrap();
            let _ = child.wait();
        }
        "exit-zero-with-detached-inherited-pipe" => {
            let child = Command::new("/bin/sleep")
                .arg("3")
                .process_group(0)
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            fs::write(
                required_env("STAGE_GATE_HELPER_PID_PATH"),
                format!("{}\n", child.id()),
            )
            .unwrap();
        }
        "sleep" => thread::sleep(Duration::from_secs(30)),
        other => panic!("unknown helper mode {other}"),
    }
}

fn helper_spec(cwd: &Path, mode: &str, env: Vec<(String, String)>) -> CommandSpec {
    let mut explicit_env = vec![("STAGE_GATE_HELPER_MODE".to_owned(), mode.to_owned())];
    explicit_env.extend(env);
    CommandSpec {
        program: std::env::current_exe().unwrap(),
        arg0: None,
        args: vec![
            "--exact".into(),
            "stage_gate_process_helper".into(),
            "--nocapture".into(),
        ],
        cwd: cwd.to_path_buf(),
        env: explicit_env,
        timeout: Duration::from_secs(5),
        termination_grace: Duration::from_millis(250),
    }
}

fn output_policy() -> OutputPolicy {
    OutputPolicy {
        max_bytes_per_stream: 16 * 1024,
        redactions: Vec::new(),
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn write_repeated(mut writer: impl std::io::Write, byte: u8, count: usize) {
    let chunk = vec![byte; 8192];
    for _ in 0..(count / chunk.len()) {
        writer.write_all(&chunk).unwrap();
    }
    writer.flush().unwrap();
}

fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(source) = fs::read_to_string(path)
            && let Ok(pid) = source.trim().parse()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "descendant PID was not recorded");
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until_process_is_gone(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .unwrap();
        if !status.success() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "descendant process {pid} survived timeout"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn git_program() -> &'static str {
    "/usr/bin/git"
}

struct TestRepository {
    temp: TempDir,
    tag: String,
    tag_object: String,
    design_commit: String,
}

impl TestRepository {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        git(temp.path(), ["init", "-q"]);
        git(temp.path(), ["config", "user.name", "Stage Gate Test"]);
        git(
            temp.path(),
            ["config", "user.email", "stage-gate@example.invalid"],
        );
        fs::write(temp.path().join("tracked.txt"), "initial\n").unwrap();
        git(temp.path(), ["add", "tracked.txt"]);
        git(temp.path(), ["commit", "-q", "-m", "initial"]);
        let design_commit = git_output(temp.path(), ["rev-parse", "HEAD"]);
        let tag = "design/v1".to_owned();
        git(temp.path(), ["tag", "-a", &tag, "-m", "pinned design"]);
        let tag_object = git_output(temp.path(), ["rev-parse", &format!("{tag}^{{tag}}")]);
        Self {
            temp,
            tag,
            tag_object,
            design_commit,
        }
    }

    fn path(&self) -> &Path {
        self.temp.path()
    }

    fn head(&self) -> String {
        git_output(self.path(), ["rev-parse", "HEAD"])
    }

    fn design(&self) -> DesignExpectation {
        DesignExpectation {
            tag: self.tag.clone(),
            tag_object: self.tag_object.clone(),
            commit: self.design_commit.clone(),
        }
    }

    fn add_commit(&self, file: &str, contents: &str) -> String {
        fs::write(self.path().join(file), contents).unwrap();
        git(self.path(), ["add", file]);
        git(self.path(), ["commit", "-q", "-m", file]);
        self.head()
    }

    fn checkout(&self, commit: &str) {
        git(self.path(), ["checkout", "-q", commit]);
    }
}

fn git<const N: usize>(repository: &Path, args: [&str; N]) {
    let status = Command::new(git_program())
        .arg("-C")
        .arg(repository)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}

fn git_output<const N: usize>(repository: &Path, args: [&str; N]) -> String {
    let output = Command::new(git_program())
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
