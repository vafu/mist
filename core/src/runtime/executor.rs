use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

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

pub(super) async fn run<F>(future: F) -> Result<F::Output, String>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    binding_runtime()?
        .spawn(future)
        .await
        .map_err(|error| error.to_string())
}
