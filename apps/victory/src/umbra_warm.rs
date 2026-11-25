//! UMBRA local shader cache warm-up. Does not execute the title.
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

pub fn shader_dir(exe: &Path) -> Result<PathBuf> {
    let exe = exe.canonicalize().with_context(|| format!("resolving {}", exe.display()))?;
    let dir = exe.parent().context("title executable has no parent")?.join("Umbra").join("shader");
    victory_core::ensure_outside_repo(&dir)?;
    victory_core::ensure_outside_repo(&dir.with_file_name("pipelines"))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(not(windows))]
pub fn run(_exe: &Path, _jobs: Option<usize>) -> Result<()> {
    anyhow::bail!("UMBRA shader warm-up requires Windows D3D12")
}

#[cfg(windows)]
pub fn run(exe: &Path, jobs: Option<usize>) -> Result<()> {
    use std::collections::HashSet;
    use std::sync::{Mutex, atomic::{AtomicUsize, Ordering::Relaxed}};
    use std::time::Duration;
    use crate::shader::{MAX_FILE, sources};
    use victory_gpu::shader_cache as cache;

    anyhow::ensure!(std::env::var_os("UMBRA_SHADER_CACHE").is_none_or(|v| v != "0"), "UMBRA shader cache is disabled");
    let exe = exe.canonicalize()?;
    let parent = exe.parent().context("missing title directory")?;
    let pack = parent.join("shaders.rpf");
    anyhow::ensure!(pack.is_file(), "missing shader archive: {}", pack.display());
    let dir = shader_dir(&exe)?;
    cache::umbra_prepare(&dir).map_err(anyhow::Error::msg)?;
    let mut paths = vec![pack, exe.clone()];
    for name in ["XboxBvhBuilder.dll", "amd_fidelityfx_upscaler_xbox.dll"] {
        let p = parent.join(name);
        if p.is_file() { paths.push(p); }
    }
    let workers = jobs.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get())).clamp(1, 64);
    println!("UMBRA_WARM BEGIN {} {}", paths.len(), workers);
    println!("UMBRA_WARM CACHE {}", dir.display());
    let seen = Mutex::new(HashSet::new());
    let refused = AtomicUsize::new(0);
    let invalid = AtomicUsize::new(0);
    let duplicate = AtomicUsize::new(0);
    let mut found = 0usize;
    for (i, path) in paths.iter().enumerate() {
        anyhow::ensure!(std::fs::metadata(path)?.len() <= MAX_FILE, "shader source exceeds supported scan size: {}", path.display());
        println!("UMBRA_WARM SCAN {} {} {}", i + 1, paths.len(), path.file_name().unwrap_or_default().to_string_lossy());
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let entries = sources(&data);
        if i == 0 { anyhow::ensure!(!entries.is_empty(), "no discoverable shader containers in shaders.rpf"); }
        found += entries.len();
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let report = || {
            let (hits, misses) = cache::umbra_counts();
            println!("UMBRA_WARM PROGRESS {} {} {} {} {} {} {} {} {}", i+1, paths.len(), done.load(Relaxed), entries.len(), hits, misses, refused.load(Relaxed), duplicate.load(Relaxed), invalid.load(Relaxed));
        };
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..workers.min(entries.len())).map(|_| s.spawn(|| {
                while let Some(entry) = entries.get(next.fetch_add(1, Relaxed)) {
                    let result = (|| {
                        let bytes = entry.bytes()?;
                        let size = u32::from_le_bytes(bytes.get(0x18..0x1c)?.try_into().ok()?) as usize;
                        let container = bytes.get(..size)?;
                        if size < 32 || victory_shader::Container::parse(container).is_err() { return None; }
                        let hash = blake3::hash(container);
                        if !seen.lock().unwrap_or_else(|e| e.into_inner()).insert(hash) {
                            duplicate.fetch_add(1, Relaxed);
                            return Some(());
                        }
                        if let Err(error) = cache::umbra_warm_one(container) {
                            let n = refused.fetch_add(1, Relaxed);
                            if n < 40 { eprintln!("UMBRA shader refused {}: {}", &hash.to_hex()[..16], error); }
                        }
                        Some(())
                    })();
                    if result.is_none() { invalid.fetch_add(1, Relaxed); }
                    done.fetch_add(1, Relaxed);
                }
            })).collect();
            report();
            while !handles.iter().all(|h| h.is_finished()) {
                std::thread::sleep(Duration::from_millis(500));
                report();
            }
        });
        report();
    }
    let (hits, misses) = cache::umbra_counts();
    anyhow::ensure!(hits + misses > 0, "no usable shaders cached");
    println!("UMBRA_WARM END {} {} {} {} {} {}", found, hits, misses, refused.load(Relaxed), duplicate.load(Relaxed), invalid.load(Relaxed));
    Ok(())
}
