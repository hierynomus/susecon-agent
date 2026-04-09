use std::{sync::Mutex, time::Duration};

/// Cached session index that accumulates across recommendation calls.
/// Each call to [`load_and_index`] grows cached data by parsing and indexing
/// session content, forcing the kernel to back real pages.
static SESSION_CACHE: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

const INDEX_PASSES: usize = 20;

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

/// Load and index session catalog data, building an in-memory search index.
/// Allocates `total_mb` of index data over `duration`. Returns peak RSS in MB.
pub async fn load_and_index(total_mb: usize, duration: Duration) -> f64 {
    let chunk_size = (total_mb * 1024 * 1024) / INDEX_PASSES;
    let sleep_per_pass = duration / INDEX_PASSES as u32;

    tracing::info!(
        total_mb,
        ?duration,
        index_passes = INDEX_PASSES,
        chunk_size_bytes = chunk_size,
        rss_mb = format!("{:.1}", peak_rss_mb()),
        "Loading session catalog and building search index"
    );

    for pass in 1..=INDEX_PASSES {
        let mut chunk = vec![0xAAu8; chunk_size];
        // Touch every page to ensure the kernel backs it.
        for page in chunk.chunks_mut(4096) {
            page[0] = 0xBB;
        }

        SESSION_CACHE.lock().expect("session cache poisoned").push(chunk);

        tracing::info!(
            pass,
            of = INDEX_PASSES,
            rss_mb = format!("{:.1}", peak_rss_mb()),
            "Indexing session catalog"
        );

        tokio::time::sleep(sleep_per_pass).await;
    }

    let rss = peak_rss_mb();
    tracing::info!(rss_mb = format!("{rss:.1}"), "Session catalog index ready");
    rss
}
