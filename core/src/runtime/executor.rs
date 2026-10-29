use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::RuntimeError;

static BINDING_RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

fn binding_runtime() -> Result<&'static Runtime, String> {
    match BINDING_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("mist-core")
            .build()
            .map_err(|error| error.to_string())
    }) {
        Ok(runtime) => Ok(runtime),
        Err(reason) => Err(reason.clone()),
    }
}

pub(super) async fn run<F>(future: F) -> Result<F::Output, RuntimeError>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    binding_runtime()
        .map_err(|reason| RuntimeError::Executor { reason })?
        .spawn(future)
        .await
        .map_err(|error| RuntimeError::Executor {
            reason: error.to_string(),
        })
}

/// Admit the operation before checking shutdown, so shutdown cannot miss an
/// operation that passed its cancellation check but has not been spawned yet.
pub(crate) async fn run_owned<F, T>(
    tasks: &TaskTracker,
    shutdown: &CancellationToken,
    future: F,
) -> Result<T, RuntimeError>
where
    F: Future<Output = Result<T, RuntimeError>> + Send + 'static,
    T: Send + 'static,
{
    let shutdown = shutdown.clone();
    let operation_shutdown = shutdown.clone();
    let future = tasks.track_future(async move {
        tokio::select! {
            biased;
            () = operation_shutdown.cancelled() => Err(RuntimeError::Closed),
            result = future => result,
        }
    });
    if shutdown.is_cancelled() {
        return Err(RuntimeError::Closed);
    }
    run(future).await?
}

// Drop cannot await. Construction has already initialized this executor, so
// the last public handle can schedule cleanup without a host Tokio context.
pub(super) fn close_dropped_instance(endpoint: iroh::Endpoint, tasks: TaskTracker) {
    if let Ok(runtime) = binding_runtime() {
        drop(runtime.spawn(async move {
            endpoint.close().await;
            tasks.wait().await;
        }));
    }
}
