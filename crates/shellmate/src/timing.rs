//! When the process started, the zero of every `--timings` figure.

/// When the process started running our code: the zero of every `--timings` figure.
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

pub(crate) fn since_start_us() -> u64 {
    START.get().map_or(0, |t| t.elapsed().as_micros() as u64)
}

/// Mark the start. Call first thing in `main`.
pub(crate) fn start() {
    START.get_or_init(std::time::Instant::now);
}
