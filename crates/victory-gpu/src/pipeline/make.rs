//! Host pipelines are made by workers, not by the title thread that first
//! records a draw with them. Translating a shader and the host driver's
//! compile take from milliseconds to seconds each (structurisation of the
//! biggest compute programs ~0.4 s, some driver compiles 3.4 s), and
//! the title's first world frame wants over 1,200 new pipelines: made
//! on its recording threads, that frame took more than 30 s and the
//! title's GPU watchdog ended the run (decision 0007, "Pipelines made off
//! the title's threads").
//!
//! A first use queues the pipeline and waits for it within a budget per
//! frame, so the cheap ones (menus, one-off passes such as a colour-grading
//! bake) are there at once as before; past the budget a draw or dispatch
//! whose pipeline is not ready is dropped, as an untranslated one is.
//!
//! The default budget is half a second per title frame. A pass the title
//! runs only once and whose pipeline takes longer is lost.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::Shader;
use super::host::{GraphicsState, HostPipeline};

/// What a host pipeline is made from, and the host pipeline once made.
pub(crate) struct Pending {
    shaders: Vec<Arc<Shader>>,
    pub(crate) graphics: Option<Box<GraphicsState>>,
    // UMBRA_BELOBUG_V02: preserve the existing UMBRA worker queue.
    screen: bool,
    queue_state: AtomicU8,
    made: OnceLock<Option<HostPipeline>>,
    /// What identifies it across runs (shader containers and state), and
    /// whether a draw or dispatch has requested it yet.
    key: String,
    used: AtomicBool,
    /// Why it is not made, if it is not ([`left_out`]).
    left_out: Option<&'static str>,
    /// When a worker began it ([`slow`]).
    begun: OnceLock<Instant>,
}

/// Why a pipeline with these shaders is not made, if it is not.
fn left_out(shaders: &[Arc<Shader>]) -> Option<&'static str> {
    if without_structures(shaders) {
        Some("shades ray-tracing hits, left out")
    } else if too_dear(shaders) {
        Some("too dear to compile, left out")
    } else {
        None
    }
}

/// Whether some compute shader of `shaders` holds `name`.
fn named(shaders: &[Arc<Shader>], name: &[u8]) -> bool {
    shaders
        .iter()
        .any(|s| s.stage == "compute" && s.container.windows(name.len()).any(|w| w == name))
}

/// Whether a shader is left out for what it costs the host's driver:
/// `CS_ScreenProbes_ActivateRays` of the title
/// (39f0e9aeb156a61c, 66,501 instructions, 9,948 branches). As translated
/// before the clean-up passes (`emit::shader_opt`), AMD's took a core and
/// 0.8 GB more every 10 s for it and did not finish (RX 7800 XT: 47 GB after
/// 8 minutes); cleaned up, it takes 85 s and 1.3 GB, once.
/// `VICTORY_ACTIVATE_RAYS=0` leaves it out.
fn too_dear(shaders: &[Arc<Shader>]) -> bool {
    static OUT: OnceLock<bool> = OnceLock::new();
    let out = *OUT.get_or_init(|| std::env::var_os("VICTORY_ACTIVATE_RAYS").is_some_and(|v| v == "0"));
    out && named(shaders, b"CS_ScreenProbes_ActivateRays")
}

/// Whether a shader only shades what the console's ray-tracing structures
/// were hit by: about 1,000 compute programs of the title, all bound
/// at the world's first frame, each 5 to 9 s of AMD's compiler. They stood in
/// the queue before the passes that make the picture, and with the structure
/// builder off there is no hit to shade.
/// `VICTORY_RT_RESOLVE=1` makes them, as does the builder being on.
fn without_structures(shaders: &[Arc<Shader>]) -> bool {
    static MADE: OnceLock<bool> = OnceLock::new();
    let on = |name| std::env::var_os(name).is_some_and(|v| v == "1");
    let made = *MADE.get_or_init(|| on("VICTORY_RT_RESOLVE") || on("VICTORY_BVH_BUILDER"));
    !made && named(shaders, b"CS_RaytracingResolve")
}

// SAFETY: D3D12 objects are free-threaded; the graphics state (whose input
// elements point into its own names) is only read after creation.
unsafe impl Send for Pending {}
// SAFETY: as above.
unsafe impl Sync for Pending {}

impl Pending {
    /// Whether it is a compute pipeline.
    fn computes(&self) -> bool {
        matches!(self.shaders.as_slice(), [only] if only.stage == "compute")
    }

    pub(crate) fn new(shaders: Vec<Arc<Shader>>, graphics: Option<Box<GraphicsState>>) -> Pending {
        let mut key = blake3::Hasher::new();
        for s in &shaders {
            key.update(s.stage.as_bytes());
            key.update(s.hash.as_bytes());
        }
        key.update(graphics.as_ref().map(|g| g.describe()).unwrap_or_default().as_bytes());
        let left_out = left_out(&shaders);
        Pending {
            screen: graphics.as_ref().is_some_and(|g| g.screen_pass())
                && shaders.iter().all(|s| s.stage != "mesh"),
            shaders,
            graphics,
            left_out,
            begun: OnceLock::new(),
            queue_state: AtomicU8::new(NOT_QUEUED),
            made: OnceLock::new(),
            key: key.finalize().to_hex()[..32].to_owned(),
            used: AtomicBool::new(false),
        }
    }

    fn make(&self) {
        let _ = self.begun.set(Instant::now());
        self.made
            .get_or_init(|| HostPipeline::new(&self.shaders, self.graphics.as_deref()));
        let (lock, done) = &DONE;
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        done.notify_all();
    }
}

/// Why a pipeline has no host pipeline (diagnostics): its shaders that do
/// not translate, else "refused" or "being made"; `None` if it has one.
#[cfg(feature = "diagnostics")]
pub(crate) fn missing(p: &Pending) -> Option<String> {
    match p.made.get() {
        Some(Some(_)) => None,
        None => Some(p.left_out.map_or_else(
            || format!("being made {:?}", p.shaders),
            str::to_owned,
        )),
        Some(None) => {
            let failed: Vec<String> = p
                .shaders
                .iter()
                .filter(|s| matches!(s.translated.get(), Some(None)))
                .map(|s| format!("{} {}", s.stage, s.hash))
                .collect();
            Some(if failed.is_empty() {
                format!("refused {:?}", p.shaders)
            } else {
                failed.join(" + ")
            })
        }
    }
}

/// Signalled whenever a worker has made a pipeline.
static DONE: (Mutex<()>, Condvar) = (Mutex::new(()), Condvar::new());

/// Waiting allowed per title frame, over all threads.
fn wait_budget() -> Duration {
    static BUDGET: OnceLock<Duration> = OnceLock::new();
    *BUDGET.get_or_init(|| {
        let millis = std::env::var("VICTORY_PIPELINE_WAIT_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(500);
        tracing::info!(millis, "host pipeline wait budget per frame");
        Duration::from_millis(millis)
    })
}

/// Whether the host's driver has been at `p` for longer than most take
/// (one of the title's takes 85 s): no frame waits for it.
fn slow(p: &Pending) -> bool {
    // A pass a title runs once (it unpacks a model, say) is lost if its
    // pipeline is not there; a draw comes again in the next frame.
    let most = if p.computes() { 10 } else { 3 };
    p.begun.get().is_some_and(|at| at.elapsed() > Duration::from_secs(most))
}

/// What a frame may wait while a world loads: pipelines are asked for by
/// the hundred then, and a pass the title runs once is lost if its
/// pipeline is not there. All such waits of a run together are bounded;
/// the title's GPU watchdog bounds one frame's (30 s).
const LOADING_FRAME: Duration = Duration::from_secs(4);
/// The same for compute pipelines, waited for apart: the passes that unpack
/// a world's models are dispatched once, and in a run whose pipelines were
/// all still to be loaded 130 of them were dropped, and with them walls,
/// floor, ceiling and faces (they never came back).
const LOADING_FRAME_COMPUTE: Duration = Duration::from_secs(12);

/// `VICTORY_PIPELINE_GATE_S`: the bound on all of them (0: none of them).
fn loading_total() -> Duration {
    static TOTAL: OnceLock<Duration> = OnceLock::new();
    *TOTAL.get_or_init(|| {
        let seconds = std::env::var("VICTORY_PIPELINE_GATE_S")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(120);
        Duration::from_secs(seconds)
    })
}

/// Separate budget for one-off screen passes. Zero restores the old shared budget.
fn screen_budget() -> Duration {
    static BUDGET: OnceLock<Duration> = OnceLock::new();
    *BUDGET.get_or_init(|| {
        let ms = std::env::var("UMBRA_SCREEN_WAIT_MS").ok()
            .and_then(|v| v.parse::<u64>().ok()).unwrap_or(5000).min(20000);
        Duration::from_millis(ms)
    })
}

/// Pipelines queued and not made yet.
static WAITING: AtomicU64 = AtomicU64::new(0);

/// Pipelines queued and not made yet (for the frame line).
pub(crate) fn waiting() -> u64 {
    WAITING.load(Ordering::Relaxed)
}

/// The host pipeline if it is made (or is made within the frame's budget);
/// queues it on first use.
pub(crate) fn ready(p: &Arc<Pending>) -> Option<&HostPipeline> {
    if p.left_out.is_some() {
        return None;
    }
    // A still-pending pipeline was requested by the title too. Prewarm it
    // on the next launch, even if this run never got to use it.
    if !p.used.load(Ordering::Relaxed) && !p.used.swap(true, Ordering::Relaxed) {
        super::library::note_used(&p.key);
    }
    if p.made.get().is_none() {
        start(p, REQUESTED);
        wait(p, crate::frame::current());
    }
    p.made.get()?.as_ref()
}

/// Makes `p` on this thread if it is not made yet (waiting for a worker
/// that is making it); whether it has a host pipeline.
pub(crate) fn now(p: &Arc<Pending>) -> bool {
    if p.left_out.is_some() {
        return false;
    }
    p.make();
    p.made.get().is_some_and(|made| made.is_some())
}

// UMBRA_V025_BVH_PREWARM: compile-only BVH audit. This queues a low-priority
// pipeline translation on normal workers; it NEVER records/dispatches a
// BVH command, switches VICTORY_BVH_BUILDER on, or copies data into the title.
pub(crate) fn prewarm_bvh_for_audit(p: &Arc<Pending>) {
    start(p, EARLY);
}

/// Translates the containers of compute programs and makes their host
/// pipelines, for the caches on disk; whether it has one (`None`: left out).
pub(crate) fn warm(shaders: Vec<Arc<Shader>>) -> Option<bool> {
    let p = Pending::new(shaders, None);
    if p.left_out.is_some() {
        return None;
    }
    Some(HostPipeline::new(&p.shaders, None).is_some())
}

/// At the title's creation of a pipeline: starts it at once if an earlier
/// run drew with it (during loading, so the world's first frames find it).
pub(crate) fn early(p: &Arc<Pending>) {
    if p.left_out.is_none() && !p.shaders.is_empty() && super::library::wanted_early(&p.key) {
        super::library::EARLY.fetch_add(1, Ordering::Relaxed);
        start(p, EARLY);
    }
}

/// Whether the frame's budget is one for all threads together: a thread
/// that waits takes what is left of it and gives back what it did not
/// use. Counted only when a wait had ended, the budget was one per thread:
/// each of the title's five recording threads began its first wait of a
/// frame with nothing counted yet and waited the whole half second, 2.5 s
/// of waiting in every frame in which pipelines were being made (runs
/// 34-49: "2504 ms waiting for pipelines" in each such frame).
/// On by default; `VICTORY_PIPELINE_WAIT_SHARED=0` counts as before.
fn shared_budget() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_PIPELINE_WAIT_SHARED").is_none_or(|v| v != "0"))
}

struct WaitBudget {
    frame: u64,
    /// Completed waits plus current reservations when sharing is enabled.
    occupied: Duration,
    completed: Duration,
    loading_used: Duration,
    loading_reserved: Duration,
}

struct WaitReservation {
    base: Duration,
    loading: Duration,
}

impl WaitBudget {
    const fn new() -> Self {
        Self {
            frame: 0,
            occupied: Duration::ZERO,
            completed: Duration::ZERO,
            loading_used: Duration::ZERO,
            loading_reserved: Duration::ZERO,
        }
    }

    fn reserve(
        &mut self,
        frame: u64,
        base: Duration,
        frame_limit: Duration,
        total: Duration,
        shared: bool,
    ) -> WaitReservation {
        if frame > self.frame {
            self.frame = frame;
            self.occupied = Duration::ZERO;
            self.completed = Duration::ZERO;
        }
        if frame != self.frame {
            return WaitReservation { base: Duration::ZERO, loading: Duration::ZERO };
        }
        let remaining = total.saturating_sub(self.loading_used + self.loading_reserved);
        let baseline = base.saturating_sub(self.occupied);
        let loading = frame_limit
            .saturating_sub(self.occupied.max(base))
            .min(remaining);
        self.loading_reserved += loading;
        if shared {
            self.occupied += baseline + loading;
        }
        WaitReservation { base: baseline, loading }
    }

    fn finish(&mut self, frame: u64, base: Duration, reservation: WaitReservation,
              waited: Duration, shared: bool) -> Duration {
        self.loading_reserved -= reservation.loading;
        let extra = if frame == self.frame {
            // Charge only the incremental time above the baseline for this
            // frame. Many short waits can use the whole extended allowance.
            let previous = self.completed.saturating_sub(base);
            self.completed += waited;
            if shared {
                self.occupied -= reservation.base + reservation.loading - waited;
            } else {
                self.occupied += waited;
            }
            self.completed.saturating_sub(base) - previous
        } else {
            // The prior frame's other completed waits are no longer retained.
            waited.saturating_sub(reservation.base)
        };
        self.loading_used += extra;
        extra
    }
}

// Graphics, compute and one-off screen passes each have their own frame
// budget. Keep separate screen totals so its longer baseline does not
// repeatedly consume the ordinary graphics loading allowance.
static BUDGETS: [Mutex<WaitBudget>; 3] = [const { Mutex::new(WaitBudget::new()) }; 3];
static WAIT_US: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static LOADING_US: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

pub(crate) fn report_waits(frames: f64) {
    let waited = WAIT_US.each_ref().map(|time| time.swap(0, Ordering::Relaxed));
    let loading = LOADING_US.each_ref().map(|time| time.swap(0, Ordering::Relaxed));
    if waited.iter().any(|&time| time != 0) {
        let ms = |time| time as f64 / (1_000.0 * frames);
        tracing::info!(
            graphics_ms = ms(waited[0]),
            compute_ms = ms(waited[1]),
            screen_ms = ms(waited[2]),
            extra_loading_ms = ms(loading.iter().sum()),
            "pipeline waits per frame"
        );
    }
}

/// Waits for p while frame has budget left.
fn wait(p: &Pending, frame: u64) {
    let normal = wait_budget();
    let screen = p.screen && !screen_budget().is_zero() && !normal.is_zero();
    let base = if screen { screen_budget() } else { normal };
    if base.is_zero() || slow(p) {
        return;
    }
    let compute = p.computes();
    let kind = if screen { 2 } else { usize::from(compute) };
    let frame_limit = if compute { LOADING_FRAME_COMPUTE } else { LOADING_FRAME };
    let shared = shared_budget();
    let reservation = BUDGETS[kind].lock().unwrap_or_else(|e| e.into_inner())
        .reserve(frame, base, frame_limit, loading_total(), shared);
    let left = reservation.base + reservation.loading;
    if left.is_zero() {
        return;
    }
    let started = Instant::now();
    let (lock, done) = &DONE;
    let mut held = lock.lock().unwrap_or_else(|e| e.into_inner());
    while p.made.get().is_none() && !slow(p) {
        let rest = left.saturating_sub(started.elapsed());
        if rest.is_zero() {
            break;
        }
        // Woken now and then to see whether it has become a slow one.
        let rest = rest.min(Duration::from_millis(250));
        held = done.wait_timeout(held, rest).unwrap_or_else(|e| e.into_inner()).0;
    }
    drop(held);
    let waited = started.elapsed().min(left);
    let extra = BUDGETS[kind].lock().unwrap_or_else(|e| e.into_inner())
        .finish(frame, base, reservation, waited, shared);
    WAIT_US[kind].fetch_add(waited.as_micros() as u64, Ordering::Relaxed);
    LOADING_US[kind].fetch_add(extra.as_micros() as u64, Ordering::Relaxed);
    #[cfg(feature = "diagnostics")]
    crate::copystat::add(
        crate::copystat::C::PipelineWaitUs,
        started.elapsed().as_micros() as u64,
    );
}

const NOT_QUEUED: u8 = 0;
const EARLY: u8 = 1;
const REQUESTED: u8 = 2;
const MAKING: u8 = 3;

struct Queue {
    early: VecDeque<Arc<Pending>>,
    requested: VecDeque<Arc<Pending>>,
}

impl Queue {
    const fn new() -> Self {
        Self {
            early: VecDeque::new(),
            requested: VecDeque::new(),
        }
    }

    /// Queues an unrequested pipeline or promotes one queued at creation.
    /// Returns whether it is new to the queue, or `None` if nothing changed.
    fn push(&mut self, p: &Arc<Pending>, priority: u8) -> Option<bool> {
        let prev = p.queue_state.load(Ordering::Relaxed);
        if prev >= priority {
            return None;
        }
        p.queue_state.store(priority, Ordering::Relaxed);
        if priority == REQUESTED {
            if p.screen && !screen_budget().is_zero() {
                self.requested.push_front(p.clone());
            } else {
                self.requested.push_back(p.clone());
            }
        } else {
            self.early.push_back(p.clone());
        }
        Some(prev == NOT_QUEUED)
    }

    fn pop(&mut self) -> Option<Arc<Pending>> {
        let p = self.requested.pop_front().or_else(|| {
            // An early entry can have been promoted and claimed already.
            std::iter::from_fn(|| self.early.pop_front())
                .find(|p| p.queue_state.load(Ordering::Relaxed) == EARLY)
        })?;
        p.queue_state.store(MAKING, Ordering::Relaxed);
        Some(p)
    }
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue::new());
static AVAILABLE: Condvar = Condvar::new();

/// Start or promote work for a draw without waiting behind the entire early
/// list. Queue changes and worker claims happen under the same lock.
fn start(p: &Arc<Pending>, priority: u8) {
    if p.queue_state.load(Ordering::Relaxed) >= priority {
        return;
    }
    if !workers() {
        if p.queue_state.swap(MAKING, Ordering::Relaxed) == NOT_QUEUED {
            p.make();
        }
        return;
    }
    let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(new) = queue.push(p, priority) {
        if new {
            PEAK.fetch_max(WAITING.fetch_add(1, Ordering::Relaxed) + 1, Ordering::Relaxed);
        }
        AVAILABLE.notify_one();
    }
}

fn next() -> Arc<Pending> {
    let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if let Some(p) = queue.pop() {
            return p;
        }
        queue = AVAILABLE.wait(queue).unwrap_or_else(|e| e.into_inner());
    }
}

/// Starts the workers (half the logical processors, 2 to 8) on first use.
fn workers() -> bool {
    static RUNNING: OnceLock<bool> = OnceLock::new();
    *RUNNING.get_or_init(|| {
        let mut running = false;
        // UMBRA_V025_PIPELINE_FEED: optionally reserve CPU capacity for the
        // renderer and memory transfers; all queued shaders still compile.
        let default_workers = std::thread::available_parallelism()
            .map_or(2, |n| (n.get() / 2).clamp(2, 8));
        let workers = std::env::var("UMBRA_V025_PIPELINE_WORKERS")
            .ok().and_then(|v| v.parse::<usize>().ok())
            .filter(|v| (1..=8).contains(v))
            .unwrap_or(default_workers)
            .min(default_workers).max(1);
        tracing::info!(default_workers, workers, "UMBRA v0.25: pipeline worker limit");
        for i in 0..workers {
            let spawned = std::thread::Builder::new()
                .name(format!("victory-pipelines-{i}"))
                .spawn(|| {
                    // Below the title's own threads (loading, recording).
                    // SAFETY: a plain call on our own thread.
                    unsafe {
                        use windows::Win32::System::Threading::*;
                        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                    }
                    loop {
                        next().make();
                        if WAITING.fetch_sub(1, Ordering::Relaxed) == 1 {
                            drained();
                        } else {
                            super::library::save(false);
                        }
                    }
                });
            if let Err(e) = spawned {
                tracing::warn!(error = %e, "no pipeline worker");
            } else {
                running = true;
            }
        }
        running
    })
}

/// The most pipelines waiting at once since the workers last ran out.
static PEAK: AtomicU64 = AtomicU64::new(0);

/// The workers ran out of work: logged after a batch of 10 or more, and
/// the cache saved (at most every 20 s).
fn drained() {
    use super::library::{EARLY, LOADED, MADE};
    if PEAK.swap(0, Ordering::Relaxed) >= 10 {
        tracing::info!(
            made = MADE.load(Ordering::Relaxed),
            loaded = LOADED.load(Ordering::Relaxed),
            started_early = EARLY.load(Ordering::Relaxed),
            "host pipelines all made"
        );
    }
    super::library::save(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_pipeline_waits_use_up_the_loading_budget() {
        let ms = Duration::from_millis;
        let mut budget = WaitBudget::new();
        for _ in 0..4 {
            let reservation = budget.reserve(1, ms(500), ms(2_000), ms(700), true);
            budget.finish(1, ms(500), reservation, ms(300), true);
        }
        assert_eq!(budget.loading_used, ms(700));
        let just_baseline = budget.reserve(2, ms(500), ms(2_000), ms(700), true);
        assert_eq!(just_baseline.base, ms(500));
        assert_eq!(just_baseline.loading, Duration::ZERO);
    }

    #[test]
    fn an_outstanding_wait_counts_against_the_loading_allowance() {
        let ms = Duration::from_millis;
        let mut budget = WaitBudget::new();
        let first = budget.reserve(1, ms(500), ms(2_000), ms(750), true);
        assert_eq!(first.loading, ms(750));
        let next = budget.reserve(2, ms(500), ms(2_000), ms(750), true);
        assert_eq!(next.loading, Duration::ZERO);
        assert_eq!(budget.finish(1, ms(500), first, ms(700), true), ms(200));
        assert_eq!(budget.finish(2, ms(500), next, ms(10), true), Duration::ZERO);
        let after = budget.reserve(3, ms(500), ms(2_000), ms(750), true);
        assert_eq!(after.loading, ms(550));
    }

    /// A first use queues the pipeline and, within the budget, gets the
    /// worker's answer (here: no host pipeline for no shaders).
    #[test]
    fn first_use_is_made_by_a_worker() {
        let p = Arc::new(Pending::new(Vec::new(), None));
        assert!(ready(&p).is_none());
        assert_ne!(p.queue_state.load(Ordering::Relaxed), NOT_QUEUED);
        let started = Instant::now();
        while p.made.get().is_none() && started.elapsed() < Duration::from_secs(10) {
            std::thread::yield_now();
        }
        assert!(matches!(p.made.get(), Some(None)));
    }

    #[test]
    fn pending_pipeline_is_recorded_for_the_next_launch() {
        let mut p = Pending::new(Vec::new(), None);
        p.key = "pending-pipeline-test".to_owned();
        // Simulate a worker that has taken the job but cannot finish it yet.
        p.queue_state.store(MAKING, Ordering::Relaxed);
        let p = Arc::new(p);

        assert!(!super::super::library::wanted_early(&p.key));
        assert!(ready(&p).is_none());
        assert!(p.made.get().is_none());
        assert!(super::super::library::wanted_early(&p.key));
    }

    #[test]
    fn draws_promote_early_pipelines_without_claiming_them_twice() {
        let earlier = Arc::new(Pending::new(Vec::new(), None));
        let promoted = Arc::new(Pending::new(Vec::new(), None));
        let requested = Arc::new(Pending::new(Vec::new(), None));
        let mut queue = Queue::new();

        assert_eq!(queue.push(&earlier, EARLY), Some(true));
        assert_eq!(queue.push(&promoted, EARLY), Some(true));
        assert_eq!(queue.push(&requested, REQUESTED), Some(true));
        assert_eq!(queue.push(&promoted, REQUESTED), Some(false));
        assert_eq!(queue.push(&promoted, REQUESTED), None);

        assert!(Arc::ptr_eq(&queue.pop().unwrap(), &requested));
        assert!(Arc::ptr_eq(&queue.pop().unwrap(), &promoted));
        assert_eq!(queue.push(&promoted, REQUESTED), None);
        assert!(Arc::ptr_eq(&queue.pop().unwrap(), &earlier));
        assert!(queue.pop().is_none());
    }
}
