use std::{sync::Mutex, time::Duration};

/// Global memory ballast that accumulates across tool calls.
/// Each call to [`bloat`] grows memory by writing non-zero bytes in chunks,
/// forcing the kernel to back real pages.
static BALLAST: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

const CHUNK_COUNT: usize = 20;

/// Read peak RSS via `getrusage(2)` (works on Linux and macOS).
pub fn peak_rss_mb() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid, zeroed struct passed by mutable pointer.
    let ok = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0;
    if !ok {
        return 0.0;
    }
    // ru_maxrss is in KB on Linux, bytes on macOS.
    if cfg!(target_os = "macos") {
        usage.ru_maxrss as f64 / (1024.0 * 1024.0)
    } else {
        usage.ru_maxrss as f64 / 1024.0
    }
}

/// Gradually allocate `total_mb` of memory over `duration`, writing `0xAA` to
/// every byte to prevent lazy page allocation. Returns the peak RSS in MB.
pub async fn bloat(total_mb: usize, duration: Duration) -> f64 {
    let chunk_size = (total_mb * 1024 * 1024) / CHUNK_COUNT;
    let sleep_per_chunk = duration / CHUNK_COUNT as u32;

    tracing::info!(
        total_mb,
        ?duration,
        chunk_count = CHUNK_COUNT,
        chunk_size_bytes = chunk_size,
        rss_mb = format!("{:.1}", peak_rss_mb()),
        "Starting gradual memory allocation (simulating agent context growth)"
    );

    for step in 1..=CHUNK_COUNT {
        let mut chunk = vec![0xAAu8; chunk_size];
        // Touch every page to ensure the kernel backs it.
        for page in chunk.chunks_mut(4096) {
            page[0] = 0xBB;
        }

        BALLAST.lock().expect("ballast poisoned").push(chunk);

        tracing::info!(
            step,
            of = CHUNK_COUNT,
            rss_mb = format!("{:.1}", peak_rss_mb()),
            "Memory allocation step"
        );

        tokio::time::sleep(sleep_per_chunk).await;
    }

    let rss = peak_rss_mb();
    tracing::info!(rss_mb = format!("{rss:.1}"), "Memory allocation complete");
    rss
}
