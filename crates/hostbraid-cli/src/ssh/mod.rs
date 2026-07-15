//! Safe OpenSSH process orchestration.
//!
//! This module deliberately delegates the SSH protocol, authentication, host-key verification,
//! and user configuration to OpenSSH. It only constructs argument arrays and coordinates local
//! child processes; it never invokes a local shell.

use command_group::{CommandGroup, GroupChild};
use hostbraid_core::{AppError, EnvironmentRef, ErrorCode, Result, SshTarget};
use serde::Serialize;
use signal_hook::SigId;
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Default number of concurrent SSH children used for fan-out execution.
pub(crate) const DEFAULT_JOBS: usize = 8;

/// Maximum bytes retained from each stream for ordinary-sized batches.
pub(crate) const MAX_STREAM_CAPTURE_BYTES: usize = 1024 * 1024;

/// Maximum raw stdout and stderr bytes retained across one batch.
pub(crate) const MAX_BATCH_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CAPTURE_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// One environment paired with provider-validated SSH coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionTarget {
    pub environment: EnvironmentRef,
    pub ssh: SshTarget,
}

/// How a child process is connected to local standard streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoMode {
    /// An interactive login session with inherited standard streams.
    Interactive,
    /// A one-shot human command with inherited standard streams.
    Inherited,
    /// A non-interactive command with bounded stdout and stderr capture.
    Captured,
}

/// A cloneable cancellation signal for queued and running local SSH children.
#[derive(Debug, Clone, Default)]
pub(crate) struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    signal_exit_code: Arc<AtomicUsize>,
}

impl CancellationToken {
    /// Request cancellation. Running SSH children are killed at their next poll.
    #[cfg(test)]
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    #[must_use]
    fn signal_exit_code(&self) -> Option<u8> {
        u8::try_from(self.signal_exit_code.load(Ordering::Acquire))
            .ok()
            .filter(|code| *code != 0)
    }
}

/// Installs graceful process-signal cancellation for a captured SSH batch.
///
/// The first termination signal asks every worker to kill and reap its SSH process group. A
/// repeated signal uses the conventional immediate-exit behavior so cleanup cannot trap the user
/// indefinitely. Dropping the guard restores the handlers that were active before the batch.
pub(crate) struct ProcessSignalGuard {
    cancellation: CancellationToken,
    registrations: Vec<SigId>,
}

impl ProcessSignalGuard {
    pub(crate) fn install() -> Result<Self> {
        let mut guard = Self {
            cancellation: CancellationToken::default(),
            registrations: Vec::with_capacity(6),
        };
        guard.register(SIGINT, 130)?;
        guard.register(SIGTERM, 143)?;
        #[cfg(windows)]
        guard.register(signal_hook::consts::signal::SIGBREAK, 149)?;
        Ok(guard)
    }

    fn register(&mut self, signal: i32, exit_code: u8) -> Result<()> {
        let immediate = signal_hook::flag::register_conditional_shutdown(
            signal,
            i32::from(exit_code),
            Arc::clone(&self.cancellation.cancelled),
        )
        .map_err(map_signal_error)?;
        self.registrations.push(immediate);

        let cancel = signal_hook::flag::register(signal, Arc::clone(&self.cancellation.cancelled))
            .map_err(map_signal_error)?;
        self.registrations.push(cancel);

        let remember = signal_hook::flag::register_usize(
            signal,
            Arc::clone(&self.cancellation.signal_exit_code),
            usize::from(exit_code),
        )
        .map_err(map_signal_error)?;
        self.registrations.push(remember);
        Ok(())
    }

    #[must_use]
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    #[must_use]
    pub(crate) fn exit_code(&self) -> Option<u8> {
        self.cancellation.signal_exit_code()
    }
}

impl Drop for ProcessSignalGuard {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            signal_hook::low_level::unregister(registration);
        }
    }
}

fn map_signal_error(error: io::Error) -> AppError {
    AppError::io("failed to install SSH cancellation signal handlers", &error)
}

/// Execution controls shared by one-shot inherited-stdio commands.
#[derive(Debug, Clone)]
pub(crate) struct RunOptions {
    pub timeout: Option<Duration>,
    pub pooling: bool,
    pub cancellation: CancellationToken,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            timeout: None,
            pooling: true,
            cancellation: CancellationToken::default(),
        }
    }
}

/// Fan-out execution controls.
#[derive(Debug, Clone)]
pub(crate) struct BatchOptions {
    pub jobs: usize,
    pub timeout: Option<Duration>,
    pub fail_fast: bool,
    pub pooling: bool,
    pub cancellation: CancellationToken,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            jobs: DEFAULT_JOBS,
            timeout: None,
            fail_fast: false,
            pooling: true,
            cancellation: CancellationToken::default(),
        }
    }
}

/// Stable state for an inherited or captured SSH process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionState {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Skipped,
}

/// Stable reason for a per-target execution failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionFailureCode {
    TargetUnavailable,
    InvalidTarget,
    SpawnFailed,
    WaitFailed,
    CaptureFailed,
    RemoteExit,
    TimedOut,
    Cancelled,
    FailFast,
}

/// Curated failure data. Raw process errors and remote output never enter this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExecutionFailure {
    pub code: ExecutionFailureCode,
    pub message: String,
}

impl ExecutionFailure {
    fn new(code: ExecutionFailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Result of a one-shot command whose standard streams were inherited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InheritedOutcome {
    pub state: ExecutionState,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub failure: Option<ExecutionFailure>,
}

/// Encoding used by one captured stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CaptureEncoding {
    Text,
    Base64,
}

/// JSON-safe bounded process output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CapturedStream {
    pub encoding: CaptureEncoding,
    pub data: String,
    pub truncated: bool,
    pub captured_bytes: usize,
}

impl CapturedStream {
    fn empty() -> Self {
        Self {
            encoding: CaptureEncoding::Text,
            data: String::new(),
            truncated: false,
            captured_bytes: 0,
        }
    }

    fn from_raw(raw: RawCapture) -> Self {
        let captured_bytes = raw.bytes.len();
        match String::from_utf8(raw.bytes) {
            Ok(data) => Self {
                encoding: CaptureEncoding::Text,
                data,
                truncated: raw.truncated,
                captured_bytes,
            },
            Err(error) => Self {
                encoding: CaptureEncoding::Base64,
                data: encode_base64(error.as_bytes()),
                truncated: raw.truncated,
                captured_bytes,
            },
        }
    }
}

/// Ordered result for one target in a fan-out operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TargetExecution {
    pub environment: EnvironmentRef,
    pub state: ExecutionState,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<ExecutionFailure>,
}

impl TargetExecution {
    pub(crate) fn target_unavailable(environment: EnvironmentRef) -> Self {
        Self {
            environment,
            state: ExecutionState::Failed,
            exit_code: None,
            duration_ms: 0,
            stdout: CapturedStream::empty(),
            stderr: CapturedStream::empty(),
            failure: Some(ExecutionFailure::new(
                ExecutionFailureCode::TargetUnavailable,
                "SSH access is unavailable for this environment",
            )),
        }
    }

    pub(crate) fn fail_fast_skipped(environment: EnvironmentRef) -> Self {
        Self::skipped(environment, false)
    }

    fn skipped(environment: EnvironmentRef, cancelled: bool) -> Self {
        let (state, code, message) = if cancelled {
            (
                ExecutionState::Cancelled,
                ExecutionFailureCode::Cancelled,
                "SSH command was cancelled before it started",
            )
        } else {
            (
                ExecutionState::Skipped,
                ExecutionFailureCode::FailFast,
                "SSH command was not started after an earlier target failed",
            )
        };
        Self {
            environment,
            state,
            exit_code: None,
            duration_ms: 0,
            stdout: CapturedStream::empty(),
            stderr: CapturedStream::empty(),
            failure: Some(ExecutionFailure::new(code, message)),
        }
    }

    #[must_use]
    pub(crate) fn succeeded(&self) -> bool {
        self.state == ExecutionState::Succeeded
    }
}

/// Complete ordered fan-out report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BatchReport {
    pub results: Vec<TargetExecution>,
    pub stream_capture_limit_bytes: usize,
}

impl BatchReport {
    pub(crate) fn from_results(results: Vec<TargetExecution>) -> Self {
        Self {
            stream_capture_limit_bytes: effective_stream_capture_limit(results.len()),
            results,
        }
    }

    #[must_use]
    pub(crate) fn succeeded(&self) -> bool {
        self.results.iter().all(TargetExecution::succeeded)
    }
}

/// Warning emitted when safe OpenSSH multiplexing cannot be configured locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PoolWarning {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone)]
struct ControlPool {
    control_path: Option<PathBuf>,
    warning: Option<PoolWarning>,
}

impl ControlPool {
    fn discover() -> Self {
        match discover_control_path() {
            Ok(control_path) => Self {
                control_path: Some(control_path),
                warning: None,
            },
            Err(()) => Self {
                control_path: None,
                warning: Some(PoolWarning {
                    code: "ssh_pool_unavailable",
                    message: "secure SSH multiplexing is unavailable; continuing without pooling",
                }),
            },
        }
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self {
            control_path: None,
            warning: None,
        }
    }

    #[cfg(test)]
    fn explicit(directory: &Path) -> Result<Self> {
        let control_path = secure_control_path(directory).map_err(|_| {
            AppError::new(
                ErrorCode::Io,
                "could not create a secure SSH multiplexing directory",
            )
        })?;
        Ok(Self {
            control_path: Some(control_path),
            warning: None,
        })
    }
}

/// OpenSSH process transport.
#[derive(Debug, Clone)]
pub(crate) struct OpenSsh {
    binary: OsString,
    pool: ControlPool,
}

impl OpenSsh {
    /// Use the `ssh` executable resolved through the ordinary process PATH.
    #[must_use]
    pub(crate) fn system() -> Self {
        Self::new("ssh")
    }

    /// Use an injectable SSH executable while discovering a secure control socket directory.
    #[must_use]
    pub(crate) fn new(binary: impl Into<OsString>) -> Self {
        Self {
            binary: binary.into(),
            pool: ControlPool::discover(),
        }
    }

    /// Use an injectable SSH executable with multiplexing disabled.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn without_pooling(binary: impl Into<OsString>) -> Self {
        Self {
            binary: binary.into(),
            pool: ControlPool::disabled(),
        }
    }

    /// Use an explicit directory after enforcing owner-only Unix permissions.
    #[cfg(test)]
    pub(crate) fn with_pool_directory(
        binary: impl Into<OsString>,
        directory: impl AsRef<Path>,
    ) -> Result<Self> {
        Ok(Self {
            binary: binary.into(),
            pool: ControlPool::explicit(directory.as_ref())?,
        })
    }

    #[must_use]
    pub(crate) fn pool_warning(&self) -> Option<&PoolWarning> {
        self.pool.warning.as_ref()
    }

    /// Check that an executable compatible with the configured command can be spawned locally.
    pub(crate) fn check_available(&self) -> Result<()> {
        let status = Command::new(&self.binary)
            .arg("-V")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(map_spawn_error)?;
        if status.success() {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::DependencyMissing,
                "the local SSH client did not pass its availability check",
            ))
        }
    }

    /// Construct the exact OpenSSH argument array for inspection or execution.
    pub(crate) fn build_arguments(
        &self,
        target: &SshTarget,
        remote_command: &[OsString],
        mode: IoMode,
        pooling: bool,
    ) -> Result<Vec<OsString>> {
        if mode != IoMode::Interactive && remote_command.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidArguments,
                "an SSH remote command is required",
            ));
        }
        if remote_command
            .first()
            .map(OsString::as_os_str)
            .is_some_and(OsStr::is_empty)
        {
            return Err(AppError::new(
                ErrorCode::InvalidArguments,
                "the SSH remote command name cannot be empty",
            ));
        }
        if target.working_directory().is_some() {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "SSH working-directory changes are not supported by the process transport",
            )
            .with_hint("Run an explicit, safely quoted remote shell command if needed."));
        }

        let mut arguments = Vec::with_capacity(remote_command.len() + 12);
        if mode == IoMode::Captured {
            arguments.push(OsString::from("-T"));
            push_option(&mut arguments, "BatchMode=yes");
        }
        if pooling {
            if let Some(control_path) = &self.pool.control_path {
                push_option(&mut arguments, "ControlMaster=auto");
                push_option(&mut arguments, "ControlPersist=60s");
                let mut value = OsString::from("ControlPath=");
                value.push(control_path);
                push_os_option(&mut arguments, value);
            }
        }
        arguments.push(OsString::from("-p"));
        arguments.push(OsString::from(target.port().to_string()));
        arguments.push(OsString::from(format!(
            "{}@{}",
            target.user(),
            target.host()
        )));
        arguments.extend(remote_command.iter().cloned());
        Ok(arguments)
    }

    /// Open an interactive SSH session with inherited standard streams.
    pub(crate) fn open_interactive(&self, target: &SshTarget, pooling: bool) -> Result<ExitStatus> {
        let arguments = self.build_arguments(target, &[], IoMode::Interactive, pooling)?;
        Command::new(&self.binary)
            .args(arguments)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(map_spawn_error)
    }

    /// Run one command with inherited standard streams.
    pub(crate) fn run_inherited(
        &self,
        target: &SshTarget,
        remote_command: &[OsString],
        options: &RunOptions,
    ) -> Result<InheritedOutcome> {
        let arguments =
            self.build_arguments(target, remote_command, IoMode::Inherited, options.pooling)?;
        let started = Instant::now();
        let child = Command::new(&self.binary)
            .args(arguments)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(map_spawn_error)?;
        // Keep inherited SSH in HostBraid's foreground process group. Creating a new process group
        // here would make terminal reads (host-key, password, or sudo prompts) stop with SIGTTIN.
        let mut child = ManagedChild::new(child);
        let outcome = wait_for_inherited_child(&mut child, options.timeout, &options.cancellation)
            .map_err(|error| AppError::io("failed while waiting for the SSH client", &error))?;
        if matches!(outcome, ProcessOutcome::Exited(_)) {
            child.release();
        }
        Ok(inherited_outcome(outcome, started.elapsed()))
    }

    /// Run one remote command against many targets with bounded parallelism and capture.
    pub(crate) fn run_batch(
        &self,
        targets: &[ExecutionTarget],
        remote_command: &[OsString],
        options: &BatchOptions,
    ) -> Result<BatchReport> {
        if targets.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "at least one SSH target is required",
            ));
        }
        if options.jobs == 0 {
            return Err(AppError::new(
                ErrorCode::InvalidArguments,
                "SSH concurrency must be greater than zero",
            ));
        }
        if remote_command.is_empty()
            || remote_command
                .first()
                .map(OsString::as_os_str)
                .is_some_and(OsStr::is_empty)
        {
            return Err(AppError::new(
                ErrorCode::InvalidArguments,
                "an SSH remote command is required",
            ));
        }

        let stream_limit = effective_stream_capture_limit(targets.len());
        let next = AtomicUsize::new(0);
        let stop_scheduling = AtomicBool::new(false);
        let result_slots: Mutex<Vec<Option<TargetExecution>>> =
            Mutex::new((0..targets.len()).map(|_| None).collect());
        let workers = options.jobs.min(targets.len());

        thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        if options.cancellation.is_cancelled()
                            || (options.fail_fast && stop_scheduling.load(Ordering::Acquire))
                        {
                            break;
                        }
                        let index = next.fetch_add(1, Ordering::AcqRel);
                        if index >= targets.len() {
                            break;
                        }
                        if options.cancellation.is_cancelled()
                            || (options.fail_fast && stop_scheduling.load(Ordering::Acquire))
                        {
                            break;
                        }

                        let result = self.execute_captured(
                            &targets[index],
                            remote_command,
                            options.pooling,
                            options.timeout,
                            &options.cancellation,
                            stream_limit,
                        );
                        if options.fail_fast && !result.succeeded() {
                            stop_scheduling.store(true, Ordering::Release);
                        }
                        result_slots
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)[index] =
                            Some(result);
                    }
                });
            }
        });

        let cancelled = options.cancellation.is_cancelled();
        let slots = result_slots
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let results = slots
            .into_iter()
            .zip(targets)
            .map(|(result, target)| {
                result.unwrap_or_else(|| {
                    if cancelled {
                        TargetExecution::skipped(target.environment.clone(), true)
                    } else {
                        TargetExecution::fail_fast_skipped(target.environment.clone())
                    }
                })
            })
            .collect();
        Ok(BatchReport::from_results(results))
    }

    fn execute_captured(
        &self,
        target: &ExecutionTarget,
        remote_command: &[OsString],
        pooling: bool,
        timeout: Option<Duration>,
        cancellation: &CancellationToken,
        stream_limit: usize,
    ) -> TargetExecution {
        let started = Instant::now();
        let arguments =
            match self.build_arguments(&target.ssh, remote_command, IoMode::Captured, pooling) {
                Ok(arguments) => arguments,
                Err(error) => {
                    return failed_execution(
                        target.environment.clone(),
                        ExecutionState::Failed,
                        ExecutionFailureCode::InvalidTarget,
                        error.message(),
                        started.elapsed(),
                    );
                }
            };
        let spawn = Command::new(&self.binary)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .group_spawn();
        let mut child = match spawn {
            Ok(child) => ManagedChildGroup::new(child),
            Err(_) => {
                return failed_execution(
                    target.environment.clone(),
                    ExecutionState::Failed,
                    ExecutionFailureCode::SpawnFailed,
                    "the local SSH client could not be started",
                    started.elapsed(),
                );
            }
        };

        let capture_failed = Arc::new(AtomicBool::new(false));
        let stdout_reader =
            child.inner().stdout.take().map(|reader| {
                capture_in_background(reader, stream_limit, Arc::clone(&capture_failed))
            });
        if stdout_reader.is_none() {
            capture_failed.store(true, Ordering::Release);
        }
        let stderr_reader =
            child.inner().stderr.take().map(|reader| {
                capture_in_background(reader, stream_limit, Arc::clone(&capture_failed))
            });
        if stderr_reader.is_none() {
            capture_failed.store(true, Ordering::Release);
        }
        let process_outcome = wait_for_child(
            &mut child,
            timeout,
            cancellation,
            Some(capture_failed.as_ref()),
        );
        let capture_deadline = Instant::now() + CAPTURE_DRAIN_TIMEOUT;
        let stdout = finish_capture(stdout_reader, capture_deadline);
        let stderr = finish_capture(stderr_reader, capture_deadline);
        let elapsed = started.elapsed();

        let Ok(process_outcome) = process_outcome else {
            return execution_with_capture(
                target.environment.clone(),
                ExecutionState::Failed,
                None,
                elapsed,
                stdout,
                stderr,
                Some(ExecutionFailure::new(
                    ExecutionFailureCode::WaitFailed,
                    "failed while waiting for the local SSH client",
                )),
            );
        };
        if matches!(process_outcome, ProcessOutcome::CaptureFailed)
            || stdout.read_failed
            || stderr.read_failed
        {
            let _cleanup = child.terminate_and_reap();
            return execution_with_capture(
                target.environment.clone(),
                ExecutionState::Failed,
                process_exit_code(&process_outcome),
                elapsed,
                stdout,
                stderr,
                Some(ExecutionFailure::new(
                    ExecutionFailureCode::CaptureFailed,
                    "failed while capturing SSH command output",
                )),
            );
        }

        if matches!(process_outcome, ProcessOutcome::Exited(_)) {
            child.release();
        }

        let (state, exit_code, failure) = match process_outcome {
            ProcessOutcome::Exited(status) if status.success() => {
                (ExecutionState::Succeeded, status.code(), None)
            }
            ProcessOutcome::Exited(status) => (
                ExecutionState::Failed,
                status.code(),
                Some(ExecutionFailure::new(
                    ExecutionFailureCode::RemoteExit,
                    "the remote command exited unsuccessfully",
                )),
            ),
            ProcessOutcome::TimedOut => (
                ExecutionState::TimedOut,
                None,
                Some(ExecutionFailure::new(
                    ExecutionFailureCode::TimedOut,
                    "the SSH command exceeded its timeout",
                )),
            ),
            ProcessOutcome::Cancelled => (
                ExecutionState::Cancelled,
                None,
                Some(ExecutionFailure::new(
                    ExecutionFailureCode::Cancelled,
                    "the SSH command was cancelled",
                )),
            ),
            ProcessOutcome::CaptureFailed => (
                ExecutionState::Failed,
                None,
                Some(ExecutionFailure::new(
                    ExecutionFailureCode::CaptureFailed,
                    "failed while capturing SSH command output",
                )),
            ),
        };
        execution_with_capture(
            target.environment.clone(),
            state,
            exit_code,
            elapsed,
            stdout,
            stderr,
            failure,
        )
    }
}

fn push_option(arguments: &mut Vec<OsString>, value: &str) {
    push_os_option(arguments, OsString::from(value));
}

fn push_os_option(arguments: &mut Vec<OsString>, value: OsString) {
    arguments.push(OsString::from("-o"));
    arguments.push(value);
}

fn map_spawn_error(error: io::Error) -> AppError {
    if error.kind() == io::ErrorKind::NotFound {
        AppError::new(
            ErrorCode::DependencyMissing,
            "a local OpenSSH client could not be found",
        )
        .with_hint("Install OpenSSH and ensure `ssh` is available on PATH.")
    } else {
        AppError::io("could not start the local SSH client", &error)
    }
}

enum ProcessOutcome {
    Exited(ExitStatus),
    TimedOut,
    Cancelled,
    CaptureFailed,
}

/// A foreground-compatible child that is killed and reaped on every abnormal return path.
struct ManagedChild {
    child: Child,
    cleanup_required: bool,
}

impl ManagedChild {
    const fn new(child: Child) -> Self {
        Self {
            child,
            cleanup_required: true,
        }
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    const fn release(&mut self) {
        self.cleanup_required = false;
    }

    fn terminate_and_reap(&mut self) -> io::Result<()> {
        if !self.cleanup_required {
            return Ok(());
        }
        let _kill_result = self.child.kill();
        let wait_result = self.child.wait();
        if wait_result.is_ok() {
            self.cleanup_required = false;
        }
        wait_result.map(drop)
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if self.cleanup_required {
            let _kill_result = self.child.kill();
            let _wait_result = self.child.wait();
        }
    }
}

/// A process-group child that kills and reaps the group unless explicitly released.
///
/// Successful SSH clients are released so OpenSSH control-master persistence keeps working.
/// Every abnormal local execution path leaves the guard armed.
struct ManagedChildGroup {
    child: GroupChild,
    cleanup_required: bool,
}

impl ManagedChildGroup {
    fn new(child: GroupChild) -> Self {
        Self {
            child,
            cleanup_required: true,
        }
    }

    fn inner(&mut self) -> &mut Child {
        self.child.inner()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // Poll the leader directly instead of GroupChild::try_wait. Keeping the group wrapper's
        // status uncached lets terminate_and_reap wait for the Windows job after capture failure.
        self.inner().try_wait()
    }

    fn release(&mut self) {
        self.cleanup_required = false;
    }

    fn terminate_and_reap(&mut self) -> io::Result<()> {
        if !self.cleanup_required {
            return Ok(());
        }

        // A failed kill can mean the group exited between the last poll and this call. Waiting is
        // still required to reap the leader, and a successful wait proves no further cleanup of
        // the owned child handle is necessary.
        let _kill_result = self.child.kill();
        let wait_result = self.child.wait();
        if wait_result.is_ok() {
            self.cleanup_required = false;
        }
        wait_result.map(drop)
    }
}

impl Drop for ManagedChildGroup {
    fn drop(&mut self) {
        if self.cleanup_required {
            let _kill_result = self.child.kill();
            let _wait_result = self.child.wait();
        }
    }
}

fn wait_for_inherited_child(
    child: &mut ManagedChild,
    timeout: Option<Duration>,
    cancellation: &CancellationToken,
) -> io::Result<ProcessOutcome> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(ProcessOutcome::Exited(status)),
            Ok(None) => {}
            Err(error) => {
                let _cleanup = child.terminate_and_reap();
                return Err(error);
            }
        }
        if cancellation.is_cancelled() {
            child.terminate_and_reap()?;
            return Ok(ProcessOutcome::Cancelled);
        }
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            child.terminate_and_reap()?;
            return Ok(ProcessOutcome::TimedOut);
        }
        thread::sleep(CHILD_POLL_INTERVAL);
    }
}

fn wait_for_child(
    child: &mut ManagedChildGroup,
    timeout: Option<Duration>,
    cancellation: &CancellationToken,
    capture_failed: Option<&AtomicBool>,
) -> io::Result<ProcessOutcome> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(ProcessOutcome::Exited(status)),
            Ok(None) => {}
            Err(error) => {
                let _cleanup = child.terminate_and_reap();
                return Err(error);
            }
        }
        if cancellation.is_cancelled() {
            child.terminate_and_reap()?;
            return Ok(ProcessOutcome::Cancelled);
        }
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            child.terminate_and_reap()?;
            return Ok(ProcessOutcome::TimedOut);
        }
        if capture_failed.is_some_and(|failed| failed.load(Ordering::Acquire)) {
            child.terminate_and_reap()?;
            return Ok(ProcessOutcome::CaptureFailed);
        }
        thread::sleep(CHILD_POLL_INTERVAL);
    }
}

fn inherited_outcome(outcome: ProcessOutcome, elapsed: Duration) -> InheritedOutcome {
    let duration_ms = duration_millis(elapsed);
    match outcome {
        ProcessOutcome::Exited(status) if status.success() => InheritedOutcome {
            state: ExecutionState::Succeeded,
            exit_code: status.code(),
            duration_ms,
            failure: None,
        },
        ProcessOutcome::Exited(status) => InheritedOutcome {
            state: ExecutionState::Failed,
            exit_code: status.code(),
            duration_ms,
            failure: Some(ExecutionFailure::new(
                ExecutionFailureCode::RemoteExit,
                "the remote command exited unsuccessfully",
            )),
        },
        ProcessOutcome::TimedOut => InheritedOutcome {
            state: ExecutionState::TimedOut,
            exit_code: None,
            duration_ms,
            failure: Some(ExecutionFailure::new(
                ExecutionFailureCode::TimedOut,
                "the SSH command exceeded its timeout",
            )),
        },
        ProcessOutcome::Cancelled => InheritedOutcome {
            state: ExecutionState::Cancelled,
            exit_code: None,
            duration_ms,
            failure: Some(ExecutionFailure::new(
                ExecutionFailureCode::Cancelled,
                "the SSH command was cancelled",
            )),
        },
        ProcessOutcome::CaptureFailed => InheritedOutcome {
            state: ExecutionState::Failed,
            exit_code: None,
            duration_ms,
            failure: Some(ExecutionFailure::new(
                ExecutionFailureCode::CaptureFailed,
                "failed while capturing SSH command output",
            )),
        },
    }
}

fn process_exit_code(outcome: &ProcessOutcome) -> Option<i32> {
    match outcome {
        ProcessOutcome::Exited(status) => status.code(),
        ProcessOutcome::TimedOut | ProcessOutcome::Cancelled | ProcessOutcome::CaptureFailed => {
            None
        }
    }
}

#[derive(Default)]
struct RawCapture {
    bytes: Vec<u8>,
    truncated: bool,
    read_failed: bool,
}

struct CaptureWorker {
    receiver: mpsc::Receiver<RawCapture>,
    _thread: JoinHandle<()>,
}

struct CaptureFailureGuard {
    failed: Arc<AtomicBool>,
    completed: bool,
}

impl Drop for CaptureFailureGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.failed.store(true, Ordering::Release);
        }
    }
}

fn read_bounded(mut reader: impl Read, limit: usize) -> RawCapture {
    let mut capture = RawCapture {
        bytes: Vec::with_capacity(limit),
        truncated: false,
        read_failed: false,
    };
    let mut buffer = [0_u8; 8192];
    loop {
        let bytes_read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes_read) => bytes_read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                capture.read_failed = true;
                break;
            }
        };
        let remaining = limit.saturating_sub(capture.bytes.len());
        let kept = remaining.min(bytes_read);
        capture.bytes.extend_from_slice(&buffer[..kept]);
        capture.truncated |= kept < bytes_read;
    }
    capture
}

fn capture_in_background(
    reader: impl Read + Send + 'static,
    limit: usize,
    failed: Arc<AtomicBool>,
) -> CaptureWorker {
    let (sender, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        let mut guard = CaptureFailureGuard {
            failed,
            completed: false,
        };
        let capture = read_bounded(reader, limit);
        if capture.read_failed {
            guard.failed.store(true, Ordering::Release);
        }
        let _send_result = sender.send(capture);
        guard.completed = true;
    });
    CaptureWorker {
        receiver,
        _thread: thread,
    }
}

fn finish_capture(reader: Option<CaptureWorker>, deadline: Instant) -> RawCapture {
    let Some(reader) = reader else {
        return failed_capture();
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    reader
        .receiver
        .recv_timeout(remaining)
        .unwrap_or_else(|_| failed_capture())
}

fn failed_capture() -> RawCapture {
    RawCapture {
        read_failed: true,
        ..RawCapture::default()
    }
}

fn execution_with_capture(
    environment: EnvironmentRef,
    state: ExecutionState,
    exit_code: Option<i32>,
    elapsed: Duration,
    stdout: RawCapture,
    stderr: RawCapture,
    failure: Option<ExecutionFailure>,
) -> TargetExecution {
    TargetExecution {
        environment,
        state,
        exit_code,
        duration_ms: duration_millis(elapsed),
        stdout: CapturedStream::from_raw(stdout),
        stderr: CapturedStream::from_raw(stderr),
        failure,
    }
}

fn failed_execution(
    environment: EnvironmentRef,
    state: ExecutionState,
    code: ExecutionFailureCode,
    message: impl Into<String>,
    elapsed: Duration,
) -> TargetExecution {
    execution_with_capture(
        environment,
        state,
        None,
        elapsed,
        RawCapture::default(),
        RawCapture::default(),
        Some(ExecutionFailure::new(code, message)),
    )
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn effective_stream_capture_limit(target_count: usize) -> usize {
    let stream_count = target_count.saturating_mul(2).max(1);
    MAX_STREAM_CAPTURE_BYTES.min(MAX_BATCH_CAPTURE_BYTES / stream_count)
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(TABLE[usize::from(first >> 2)]));
        encoded.push(char::from(
            TABLE[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        if chunk.len() > 1 {
            encoded.push(char::from(
                TABLE[usize::from(((second & 0x0f) << 2) | (third >> 6))],
            ));
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(char::from(TABLE[usize::from(third & 0x3f)]));
        } else {
            encoded.push('=');
        }
    }
    encoded
}

#[cfg(unix)]
fn discover_control_path() -> std::result::Result<PathBuf, ()> {
    use std::os::unix::fs::MetadataExt;

    let uid = current_uid().map_err(|_| ())?;
    let mut candidates = Vec::with_capacity(2);
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        candidates.push(PathBuf::from(runtime).join("hostbraid-ssh"));
    }
    candidates.push(std::env::temp_dir().join(format!("hostbraid-ssh-{uid}")));
    for directory in candidates {
        if let Ok(path) = secure_control_path(&directory) {
            let metadata = std::fs::symlink_metadata(&directory).map_err(|_| ())?;
            if metadata.uid() == uid {
                return Ok(path);
            }
        }
    }
    Err(())
}

#[cfg(not(unix))]
fn discover_control_path() -> std::result::Result<PathBuf, ()> {
    Err(())
}

#[cfg(unix)]
fn secure_control_path(directory: &Path) -> io::Result<PathBuf> {
    use std::fs::{DirBuilder, OpenOptions, Permissions};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::sync::atomic::AtomicU64;

    static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let expected_uid = current_uid()?;
    validate_control_parent(directory, expected_uid)?;
    match DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH pool path is not a directory",
        ));
    }
    // An existing writable directory may already contain a socket planted by another local user.
    // Tightening its mode after inspecting the contents cannot make such a socket trustworthy.
    if metadata.uid() != expected_uid || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH pool directory was not already private",
        ));
    }

    let probe_name = format!(
        ".owner-probe-{}-{}",
        std::process::id(),
        PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let probe_path = directory.join(probe_name);
    let probe = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe_path)?;
    let probe_uid = probe.metadata().map(|probe| probe.uid());
    drop(probe);
    let _ = std::fs::remove_file(&probe_path);
    let probe_uid = probe_uid?;
    if probe_uid != expected_uid || metadata.uid() != expected_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH pool directory has the wrong owner",
        ));
    }

    std::fs::set_permissions(directory, Permissions::from_mode(0o700))?;
    let secured = std::fs::symlink_metadata(directory)?;
    if !secured.file_type().is_dir()
        || secured.file_type().is_symlink()
        || secured.uid() != probe_uid
        || secured.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH pool directory is not private",
        ));
    }

    let control_path = directory.join("%C");
    if expanded_control_path_len(&control_path) > 96 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SSH control path is too long",
        ));
    }
    Ok(control_path)
}

#[cfg(unix)]
fn validate_control_parent(directory: &Path, uid: u32) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let parent = directory.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "SSH pool directory has no parent",
        )
    })?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH pool parent is not a directory",
        ));
    }

    let mode = metadata.mode();
    let writable_by_others = mode & 0o022 != 0;
    let sticky = mode & 0o1000 != 0;
    let trusted_owner = metadata.uid() == uid || metadata.uid() == 0;
    if !trusted_owner || (writable_by_others && !sticky) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SSH pool parent is not protected from replacement",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_control_path(_directory: &Path) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure SSH control paths require Unix permissions",
    ))
}

#[cfg(unix)]
fn current_uid() -> io::Result<u32> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::sync::atomic::AtomicU64;

    static UID_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let probe_path = std::env::temp_dir().join(format!(
        ".hostbraid-uid-{}-{}",
        std::process::id(),
        UID_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let probe = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe_path)?;
    let uid = probe.metadata().map(|probe| probe.uid());
    drop(probe);
    let _ = std::fs::remove_file(probe_path);
    uid
}

#[cfg(unix)]
fn expanded_control_path_len(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt;

    // OpenSSH expands `%C` to a 40-character SHA-1 hex digest.
    path.as_os_str().as_bytes().len().saturating_add(38)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct SlowEofReader {
        delay: Duration,
    }

    impl Read for SlowEofReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            thread::sleep(self.delay);
            Ok(0)
        }
    }

    fn ssh_target(host: &str) -> SshTarget {
        SshTarget::try_new(host, 61_000, "site_user", None).expect("valid SSH target")
    }

    fn environment(index: usize) -> EnvironmentRef {
        EnvironmentRef::try_new(
            "kinsta",
            "agency",
            format!("site-{index}"),
            format!("env-{index}"),
        )
        .expect("valid environment reference")
    }

    fn execution_target(index: usize, host: &str) -> ExecutionTarget {
        ExecutionTarget {
            environment: environment(index),
            ssh: ssh_target(host),
        }
    }

    #[test]
    fn captured_arguments_are_separate_and_preserve_host_key_policy() {
        let transport = OpenSsh::without_pooling("ssh");
        let command = vec![OsString::from("printf"), OsString::from("hello; uname -a")];

        let arguments = transport
            .build_arguments(&ssh_target("ssh.example"), &command, IoMode::Captured, true)
            .expect("arguments build");

        assert_eq!(
            arguments,
            vec![
                "-T",
                "-o",
                "BatchMode=yes",
                "-p",
                "61000",
                "site_user@ssh.example",
                "printf",
                "hello; uname -a",
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
        assert!(
            !arguments
                .iter()
                .any(|argument| { argument.to_string_lossy().contains("StrictHostKeyChecking") })
        );
    }

    #[test]
    fn inherited_mode_does_not_force_batch_or_tty_options() {
        let transport = OpenSsh::without_pooling("ssh");
        let arguments = transport
            .build_arguments(
                &ssh_target("ssh.example"),
                &[OsString::from("wp"), OsString::from("core")],
                IoMode::Inherited,
                false,
            )
            .expect("arguments build");

        assert!(!arguments.iter().any(|argument| argument == "-T"));
        assert!(!arguments.iter().any(|argument| argument == "BatchMode=yes"));
    }

    #[test]
    fn working_directory_is_not_interpolated_into_a_remote_shell_string() {
        let transport = OpenSsh::without_pooling("ssh");
        let target = SshTarget::try_new(
            "ssh.example",
            22,
            "user",
            Some("/path/with spaces".to_owned()),
        )
        .expect("core accepts safe absolute path");

        let error = transport
            .build_arguments(&target, &[OsString::from("pwd")], IoMode::Captured, false)
            .expect_err("working directory must not be interpolated");

        assert_eq!(error.code(), ErrorCode::Unsupported);
    }

    #[test]
    fn invalid_utf8_is_base64_encoded() {
        let captured = CapturedStream::from_raw(RawCapture {
            bytes: vec![0xff, 0x00, 0x61],
            truncated: true,
            read_failed: false,
        });

        assert_eq!(captured.encoding, CaptureEncoding::Base64);
        assert_eq!(captured.data, "/wBh");
        assert_eq!(captured.captured_bytes, 3);
        assert!(captured.truncated);
    }

    #[test]
    fn bounded_reader_keeps_draining_after_truncation() {
        let raw = read_bounded(Cursor::new(vec![b'x'; 32]), 5);

        assert_eq!(raw.bytes, b"xxxxx");
        assert!(raw.truncated);
        assert!(!raw.read_failed);
    }

    #[test]
    fn capture_drain_stops_waiting_at_its_deadline() {
        let reader = capture_in_background(
            SlowEofReader {
                delay: Duration::from_millis(250),
            },
            5,
            Arc::new(AtomicBool::new(false)),
        );
        let started = Instant::now();
        let capture = finish_capture(Some(reader), Instant::now() + Duration::from_millis(20));

        assert!(capture.read_failed);
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn large_batches_share_the_total_capture_budget_fairly() {
        assert_eq!(effective_stream_capture_limit(1), 1024 * 1024);
        assert_eq!(effective_stream_capture_limit(32), 1024 * 1024);
        assert_eq!(
            effective_stream_capture_limit(33),
            MAX_BATCH_CAPTURE_BYTES / 66
        );
    }

    #[test]
    fn preflight_results_use_safe_failures_and_the_effective_capture_limit() {
        let unavailable = TargetExecution::target_unavailable(environment(0));
        let skipped = TargetExecution::fail_fast_skipped(environment(1));
        let report = BatchReport::from_results(
            (0..33)
                .map(|index| TargetExecution::target_unavailable(environment(index)))
                .collect(),
        );

        assert_eq!(unavailable.state, ExecutionState::Failed);
        assert_eq!(
            unavailable.failure.as_ref().map(|failure| failure.code),
            Some(ExecutionFailureCode::TargetUnavailable)
        );
        assert_eq!(skipped.state, ExecutionState::Skipped);
        assert_eq!(
            report.stream_capture_limit_bytes,
            effective_stream_capture_limit(33)
        );
    }

    #[cfg(unix)]
    fn fake_ssh(script: &str, label: &str) -> (OpenSsh, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "hostbraid-ssh-test-{label}-{}-{}",
            std::process::id(),
            UID_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).expect("test directory");
        let executable = directory.join("ssh");
        std::fs::write(&executable, script).expect("write fake SSH executable");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
            .expect("make fake SSH executable executable");
        (
            OpenSsh::without_pooling(executable.into_os_string()),
            directory,
        )
    }

    #[cfg(unix)]
    static UID_TEST_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    #[cfg(unix)]
    #[test]
    fn batch_capture_reports_nonzero_exit_and_binary_output() {
        let (transport, directory) = fake_ssh(
            "#!/bin/sh\nprintf 'hello'\nprintf '\\377' >&2\nexit 7\n",
            "capture",
        );
        let report = transport
            .run_batch(
                &[execution_target(1, "ssh.example")],
                &[OsString::from("wp"), OsString::from("plugin")],
                &BatchOptions {
                    jobs: 1,
                    pooling: false,
                    ..BatchOptions::default()
                },
            )
            .expect("batch executes");
        let result = &report.results[0];

        assert_eq!(result.state, ExecutionState::Failed);
        assert_eq!(result.exit_code, Some(7));
        assert_eq!(result.stdout.data, "hello");
        assert_eq!(result.stderr.encoding, CaptureEncoding::Base64);
        assert_eq!(result.stderr.data, "/w==");
        assert!(!report.succeeded());
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn fail_fast_leaves_queued_targets_unstarted() {
        let (transport, directory) = fake_ssh("#!/bin/sh\nexit 9\n", "fail-fast");
        let targets = vec![
            execution_target(1, "one.example"),
            execution_target(2, "two.example"),
            execution_target(3, "three.example"),
        ];
        let report = transport
            .run_batch(
                &targets,
                &[OsString::from("false")],
                &BatchOptions {
                    jobs: 1,
                    fail_fast: true,
                    pooling: false,
                    ..BatchOptions::default()
                },
            )
            .expect("batch executes");

        assert_eq!(report.results[0].state, ExecutionState::Failed);
        assert_eq!(report.results[1].state, ExecutionState::Skipped);
        assert_eq!(report.results[2].state, ExecutionState::Skipped);
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn batch_never_exceeds_the_requested_concurrency() {
        let script = r#"#!/bin/sh
state=${0%/*}
active="$state/active.$$"
: > "$active"
while :; do
  count=0
  for item in "$state"/active.*; do
    if [ -e "$item" ]; then count=$((count + 1)); fi
  done
  if [ "$count" -ge 3 ] || [ -e "$state/released" ]; then break; fi
  sleep 0.01
done
: > "$state/released"
printf '%s\n' "$count" >> "$state/counts"
sleep 0.03
rm -f "$active"
exit 0
"#;
        let (transport, directory) = fake_ssh(script, "concurrency");
        let targets: Vec<_> = (0..9)
            .map(|index| execution_target(index, "ssh.example"))
            .collect();
        let report = transport
            .run_batch(
                &targets,
                &[OsString::from("true")],
                &BatchOptions {
                    jobs: 3,
                    pooling: false,
                    ..BatchOptions::default()
                },
            )
            .expect("batch executes");
        let counts = std::fs::read_to_string(directory.join("counts")).expect("read counts");
        let maximum = counts
            .lines()
            .map(|line| line.parse::<usize>().expect("numeric concurrency"))
            .max()
            .expect("at least one observation");

        assert!(report.succeeded());
        assert_eq!(maximum, 3);
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_the_local_ssh_process_group() {
        let script = r#"#!/bin/sh
state=${0%/*}
(sleep 0.25; : > "$state/descendant-survived") &
while :; do sleep 1; done
"#;
        let (transport, directory) = fake_ssh(script, "timeout");
        let report = transport
            .run_batch(
                &[execution_target(1, "ssh.example")],
                &[OsString::from("slow-command")],
                &BatchOptions {
                    jobs: 1,
                    timeout: Some(Duration::from_millis(30)),
                    pooling: false,
                    ..BatchOptions::default()
                },
            )
            .expect("batch executes");

        assert_eq!(report.results[0].state, ExecutionState::TimedOut);
        assert!(report.results[0].duration_ms < 1_000);
        thread::sleep(Duration::from_millis(300));
        assert!(!directory.join("descendant-survived").exists());
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_stops_running_and_queued_children() {
        let (transport, directory) = fake_ssh("#!/bin/sh\nwhile :; do :; done\n", "cancel");
        let cancellation = CancellationToken::default();
        let cancel_from_thread = cancellation.clone();
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            cancel_from_thread.cancel();
        });
        let targets = vec![
            execution_target(1, "one.example"),
            execution_target(2, "two.example"),
        ];
        let report = transport
            .run_batch(
                &targets,
                &[OsString::from("slow-command")],
                &BatchOptions {
                    jobs: 1,
                    pooling: false,
                    cancellation,
                    ..BatchOptions::default()
                },
            )
            .expect("batch executes");
        trigger.join().expect("cancellation trigger joins");

        assert_eq!(report.results[0].state, ExecutionState::Cancelled);
        assert_eq!(report.results[1].state, ExecutionState::Cancelled);
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn interrupt_signal_cancels_and_reaps_captured_children() {
        let (transport, directory) = fake_ssh(
            "#!/bin/sh\nstate=${0%/*}\n(sleep 0.25; : > \"$state/descendant-survived\") &\nwhile :; do :; done\n",
            "signal-cancel",
        );
        let signals = ProcessSignalGuard::install().expect("install signal cancellation");
        let trigger = thread::spawn(|| {
            thread::sleep(Duration::from_millis(30));
            signal_hook::low_level::raise(SIGINT).expect("raise interrupt signal");
        });
        let report = transport
            .run_batch(
                &[execution_target(1, "one.example")],
                &[OsString::from("slow-command")],
                &BatchOptions {
                    jobs: 1,
                    pooling: false,
                    cancellation: signals.cancellation(),
                    ..BatchOptions::default()
                },
            )
            .expect("batch executes");
        trigger.join().expect("signal trigger joins");

        assert_eq!(signals.exit_code(), Some(130));
        assert_eq!(report.results[0].state, ExecutionState::Cancelled);
        thread::sleep(Duration::from_millis(300));
        assert!(!directory.join("descendant-survived").exists());
        drop(signals);
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_pool_uses_private_percent_c_control_path() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "hostbraid-pool-test-{}-{}",
            std::process::id(),
            UID_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let transport = OpenSsh::with_pool_directory("ssh", &directory).expect("secure pool");
        let arguments = transport
            .build_arguments(
                &ssh_target("ssh.example"),
                &[OsString::from("true")],
                IoMode::Captured,
                true,
            )
            .expect("arguments build");

        assert!(
            arguments
                .iter()
                .any(|argument| argument == "ControlMaster=auto")
        );
        assert!(
            arguments
                .iter()
                .any(|argument| argument == "ControlPersist=60s")
        );
        assert!(arguments.iter().any(|argument| {
            argument.to_string_lossy().starts_with("ControlPath=")
                && argument.to_string_lossy().ends_with("/%C")
        }));
        assert_eq!(
            std::fs::metadata(&directory)
                .expect("pool metadata")
                .permissions()
                .mode()
                & 0o077,
            0
        );
        std::fs::remove_dir_all(directory).expect("remove pool directory");
    }

    #[cfg(unix)]
    #[test]
    fn pool_rejects_a_replaceable_parent_directory() {
        use std::os::unix::fs::PermissionsExt;

        let parent = std::env::temp_dir().join(format!(
            "hostbraid-pool-parent-test-{}-{}",
            std::process::id(),
            UID_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&parent).expect("create test parent");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777))
            .expect("make parent replaceable");

        let result = OpenSsh::with_pool_directory("ssh", parent.join("pool"));

        assert!(result.is_err());
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
            .expect("restore private permissions");
        std::fs::remove_dir_all(parent).expect("remove test parent");
    }

    #[cfg(unix)]
    #[test]
    fn pool_rejects_an_existing_non_private_directory() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "hostbraid-open-pool-test-{}-{}",
            std::process::id(),
            UID_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).expect("create pool directory");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o770))
            .expect("make pool directory group-writable");
        std::fs::write(directory.join("%C"), b"untrusted socket placeholder")
            .expect("plant untrusted path");

        let result = OpenSsh::with_pool_directory("ssh", &directory);

        assert!(result.is_err());
        assert!(directory.join("%C").exists());
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .expect("restore private permissions");
        std::fs::remove_dir_all(directory).expect("remove pool directory");
    }
}
