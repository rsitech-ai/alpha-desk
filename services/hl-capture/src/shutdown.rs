use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::task::{Id, JoinSet};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

type TaskFuture = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + 'static>>;
type TaskOutcome = (&'static str, Result<(), AppError>);
type TaskJoinResult = Option<Result<TaskOutcome, tokio::task::JoinError>>;

pub struct OwnedTask {
    name: &'static str,
    future: TaskFuture,
}

impl OwnedTask {
    pub fn new<F>(name: &'static str, future: F) -> Self
    where
        F: Future<Output = Result<(), AppError>> + Send + 'static,
    {
        Self {
            name,
            future: Box::pin(future),
        }
    }

    pub fn from_join_handle(
        name: &'static str,
        handle: tokio::task::JoinHandle<Result<(), AppError>>,
    ) -> Self {
        let abort_on_drop = AbortOnDrop(handle.abort_handle());
        Self::new(name, async move {
            let _abort_on_drop = abort_on_drop;
            handle
                .await
                .unwrap_or(Err(AppError::TaskPanicked { task: name }))
        })
    }
}

#[derive(Debug)]
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl std::fmt::Debug for OwnedTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedTask")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AppError {
    #[error("capture task {task} failed with reason code {reason_code}")]
    TaskFailed {
        task: &'static str,
        reason_code: &'static str,
    },
    #[error("capture task {task} exited before shutdown")]
    TaskExited { task: &'static str },
    #[error("capture task {task} panicked")]
    TaskPanicked { task: &'static str },
    #[error("capture task names must be nonempty and unique")]
    InvalidTaskSet,
    #[error("capture shutdown exceeded its configured grace period")]
    ShutdownTimeout,
}

impl AppError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::TaskFailed { .. } => "capture_app.task_failed",
            Self::TaskExited { .. } => "capture_app.task_exited",
            Self::TaskPanicked { .. } => "capture_app.task_panicked",
            Self::InvalidTaskSet => "capture_app.invalid_task_set",
            Self::ShutdownTimeout => "capture_app.shutdown_timeout",
        }
    }
}

pub async fn run_owned_tasks(
    cancellation: CancellationToken,
    shutdown_grace: Duration,
    tasks: Vec<OwnedTask>,
) -> Result<(), AppError> {
    validate_task_set(shutdown_grace, &tasks)?;

    let mut task_names = HashMap::<Id, &'static str>::with_capacity(tasks.len());
    let mut task_set = JoinSet::new();
    for task in tasks {
        let name = task.name;
        let abort_handle = task_set.spawn(async move { (name, task.future.await) });
        task_names.insert(abort_handle.id(), name);
    }

    let primary_error = tokio::select! {
        () = cancellation.cancelled() => None,
        result = task_set.join_next() => {
            Some(classify_join(
                result,
                &mut task_names,
                cancellation.is_cancelled(),
            ))
        },
    };
    cancellation.cancel();

    let drain_result = timeout(shutdown_grace, async {
        let mut shutdown_error = None;
        while let Some(result) = task_set.join_next().await {
            let result = classify_join(Some(result), &mut task_names, true);
            if shutdown_error.is_none() && result.is_err() {
                shutdown_error = result.err();
            }
        }
        shutdown_error
    })
    .await;

    let shutdown_error = match drain_result {
        Ok(error) => error,
        Err(_) => {
            task_set.abort_all();
            while task_set.join_next().await.is_some() {}
            Some(AppError::ShutdownTimeout)
        }
    };

    match primary_error {
        Some(Err(error)) => Err(error),
        Some(Ok(())) | None => shutdown_error.map_or(Ok(()), Err),
    }
}

fn validate_task_set(shutdown_grace: Duration, tasks: &[OwnedTask]) -> Result<(), AppError> {
    if tasks.is_empty() || shutdown_grace.is_zero() {
        return Err(AppError::InvalidTaskSet);
    }
    let mut names = BTreeSet::new();
    if tasks
        .iter()
        .any(|task| task.name.is_empty() || !names.insert(task.name))
    {
        return Err(AppError::InvalidTaskSet);
    }
    Ok(())
}

fn classify_join(
    result: TaskJoinResult,
    task_names: &mut HashMap<Id, &'static str>,
    shutdown_started: bool,
) -> Result<(), AppError> {
    match result {
        Some(Ok((name, Ok(())))) => {
            task_names.retain(|_, stored_name| *stored_name != name);
            if shutdown_started {
                Ok(())
            } else {
                Err(AppError::TaskExited { task: name })
            }
        }
        Some(Ok((name, Err(error)))) => {
            task_names.retain(|_, stored_name| *stored_name != name);
            Err(error)
        }
        Some(Err(error)) => {
            let name = task_names.remove(&error.id()).unwrap_or("unknown");
            Err(AppError::TaskPanicked { task: name })
        }
        None => Err(AppError::InvalidTaskSet),
    }
}
