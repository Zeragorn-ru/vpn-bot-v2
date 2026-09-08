use tracing_subscriber::EnvFilter;
use uuid::Uuid;

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(false)
        .try_init()
        .ok();
}

#[must_use]
pub fn correlation_id() -> Uuid {
    Uuid::now_v7()
}
