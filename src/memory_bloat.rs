use std::sync::Mutex;

/// Global memory ballast that accumulates across tool calls.
/// Each call to `bloat()` grows memory over ~10-15 seconds by writing
/// non-zero bytes in chunks, forcing the kernel to back real pages.
static BALLAST: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// Read current RSS from /proc/self/status (Linux only).
pub fn current_rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|line| {
                    line.split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<f64>().ok())
                })
        })
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

/// Gradually allocate `total_mb` of memory over `duration_secs` seconds.
/// Writes 0xAA to every byte to prevent lazy page allocation.
/// Returns the final RSS in MB.
pub async fn bloat(total_mb: usize, duration_secs: u64) -> f64 {
    let chunk_count = 20usize; // allocate in 20 steps
    let chunk_size = (total_mb * 1024 * 1024) / chunk_count;
    let sleep_per_chunk =
        tokio::time::Duration::from_millis((duration_secs * 1000) / chunk_count as u64);

    tracing::info!(
        total_mb,
        duration_secs,
        chunk_count,
        chunk_size_bytes = chunk_size,
        rss_mb = format!("{:.1}", current_rss_mb()),
        "Starting gradual memory allocation (simulating agent context growth)"
    );

    for i in 0..chunk_count {
        // Allocate a chunk and fill it with non-zero data
        let mut chunk = vec![0xAAu8; chunk_size];
        // Touch every page to ensure the kernel backs it
        for page in chunk.chunks_mut(4096) {
            page[0] = 0xBB;
        }

        {
            let mut ballast = BALLAST.lock().unwrap();
            ballast.push(chunk);
        }

        let rss = current_rss_mb();
        tracing::info!(
            step = i + 1,
            of = chunk_count,
            rss_mb = format!("{:.1}", rss),
            "Memory allocation step"
        );

        tokio::time::sleep(sleep_per_chunk).await;
    }

    let final_rss = current_rss_mb();
    tracing::info!(
        rss_mb = format!("{:.1}", final_rss),
        "Memory allocation complete"
    );
    final_rss
}
