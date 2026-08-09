//! Background decoding.
//!
//! The UI thread never touches a JPEG. A small pool of workers decodes, rotates,
//! downscales, reads EXIF and resolves the nearest city, then hands the finished
//! bundle over a channel; stepping to the next photo is then just a texture
//! upload. Jobs carry a priority — distance from the picture you are looking at
//! — and workers always take the lowest one available, so changing your mind
//! and jumping across the folder does not queue behind ten stale decodes.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use sort4print_core::geo::{CityDb, Place};
use sort4print_core::loader::{self, Preview};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Editor-sized image for the picture in view and its neighbours.
    Preview,
    /// Small image for a filmstrip row.
    Thumb,
}

#[derive(Debug)]
pub struct LoadedImage {
    pub preview: Preview,
    /// Nearest city to the EXIF GPS fix, if there was one.
    pub place: Option<Place>,
}

struct Job {
    priority: u32,
    path: PathBuf,
    kind: JobKind,
    max_px: u32,
}

struct Queue {
    jobs: Vec<Job>,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

struct Done {
    path: PathBuf,
    kind: JobKind,
    result: Result<Arc<LoadedImage>, String>,
}

/// Most-recently-used-last order, so eviction takes from the front.
struct Cache {
    map: HashMap<PathBuf, Arc<LoadedImage>>,
    order: VecDeque<PathBuf>,
    limit: usize,
}

impl Cache {
    fn new(limit: usize) -> Cache {
        Cache {
            map: HashMap::new(),
            order: VecDeque::new(),
            limit: limit.max(1),
        }
    }

    fn get(&mut self, path: &Path) -> Option<Arc<LoadedImage>> {
        let hit = self.map.get(path)?.clone();
        self.touch(path);
        Some(hit)
    }

    fn peek(&self, path: &Path) -> Option<&Arc<LoadedImage>> {
        self.map.get(path)
    }

    fn touch(&mut self, path: &Path) {
        if let Some(i) = self.order.iter().position(|p| p == path) {
            let p = self.order.remove(i).expect("index came from the deque");
            self.order.push_back(p);
        }
    }

    fn insert(&mut self, path: PathBuf, value: Arc<LoadedImage>) {
        if self.map.insert(path.clone(), value).is_none() {
            self.order.push_back(path);
        } else {
            self.touch(&path);
        }
        while self.order.len() > self.limit {
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }
    }

    fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
        while self.order.len() > self.limit {
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

pub struct Prefetcher {
    shared: Arc<Shared>,
    results: Receiver<Done>,
    workers: Vec<std::thread::JoinHandle<()>>,
    previews: Cache,
    thumbs: Cache,
    /// Queued or being worked on; stops the same file being asked for twice.
    inflight: std::collections::HashSet<(PathBuf, JobKind)>,
    failed: HashMap<PathBuf, String>,
}

impl Prefetcher {
    pub fn new(workers: usize, cache_limit: usize, repaint: egui::Context) -> Prefetcher {
        let worker_count = if workers == 0 {
            // Leave a core for the UI and one for the OS; more threads than
            // that just compete for memory bandwidth while decoding.
            std::thread::available_parallelism()
                .map(|n| n.get().saturating_sub(1).clamp(1, 8))
                .unwrap_or(2)
        } else {
            workers.clamp(1, 16)
        };

        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                jobs: Vec::new(),
                stop: false,
            }),
            wake: Condvar::new(),
        });
        let (tx, results) = channel();

        let handles = (0..worker_count)
            .filter_map(|i| {
                let shared = Arc::clone(&shared);
                let tx: Sender<Done> = tx.clone();
                let repaint = repaint.clone();
                // Named so the panic hook can tell a decode crash, which is
                // handled, from a real one, which should still be reported.
                std::thread::Builder::new()
                    .name(format!("sort4print-decode-{i}"))
                    .spawn(move || worker_loop(shared, tx, repaint))
                    .ok()
            })
            .collect();

        Prefetcher {
            shared,
            results,
            workers: handles,
            previews: Cache::new(cache_limit),
            thumbs: Cache::new(cache_limit.max(64)),
            inflight: Default::default(),
            failed: HashMap::new(),
        }
    }

    /// Queues a decode unless the result is already cached, already queued, or
    /// already known to be unreadable.
    pub fn request(&mut self, path: &Path, kind: JobKind, max_px: u32, priority: u32) {
        let key = (path.to_path_buf(), kind);
        if self.inflight.contains(&key) || self.failed.contains_key(path) {
            return;
        }
        let cached = match kind {
            JobKind::Preview => self.previews.peek(path).is_some(),
            JobKind::Thumb => self.thumbs.peek(path).is_some(),
        };
        if cached {
            return;
        }

        self.inflight.insert(key);
        let mut queue = self.shared.queue.lock().expect("prefetch queue poisoned");
        queue.jobs.push(Job {
            priority,
            path: path.to_path_buf(),
            kind,
            max_px,
        });
        drop(queue);
        self.shared.wake.notify_one();
    }

    /// Drops anything still queued. Used when the folder changes, so workers do
    /// not spend time on files nobody is looking at any more.
    pub fn cancel_pending(&mut self) {
        let mut queue = self.shared.queue.lock().expect("prefetch queue poisoned");
        for job in queue.jobs.drain(..) {
            self.inflight.remove(&(job.path, job.kind));
        }
    }

    /// Moves finished work into the caches. Call once per frame.
    pub fn poll(&mut self) -> usize {
        let mut received = 0;
        while let Ok(done) = self.results.try_recv() {
            self.inflight.remove(&(done.path.clone(), done.kind));
            received += 1;
            match done.result {
                Ok(image) => match done.kind {
                    JobKind::Preview => self.previews.insert(done.path, image),
                    JobKind::Thumb => self.thumbs.insert(done.path, image),
                },
                Err(message) => {
                    self.failed.insert(done.path, message);
                }
            }
        }
        received
    }

    pub fn preview(&mut self, path: &Path) -> Option<Arc<LoadedImage>> {
        self.previews.get(path)
    }

    pub fn thumb(&mut self, path: &Path) -> Option<Arc<LoadedImage>> {
        self.thumbs.get(path)
    }

    pub fn error(&self, path: &Path) -> Option<&str> {
        self.failed.get(path).map(String::as_str)
    }

    pub fn set_cache_limit(&mut self, limit: usize) {
        self.previews.set_limit(limit);
        self.thumbs.set_limit(limit.max(64));
    }

    /// Forgets everything, including past failures, so a re-scan retries files
    /// that were being written when they were first read.
    pub fn clear(&mut self) {
        self.cancel_pending();
        self.previews.clear();
        self.thumbs.clear();
        self.failed.clear();
    }

    pub fn cached_count(&self) -> usize {
        self.previews.map.len()
    }
}

impl Drop for Prefetcher {
    fn drop(&mut self) {
        {
            let mut queue = self.shared.queue.lock().expect("prefetch queue poisoned");
            queue.stop = true;
            queue.jobs.clear();
        }
        self.shared.wake.notify_all();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

fn worker_loop(shared: Arc<Shared>, tx: Sender<Done>, repaint: egui::Context) {
    loop {
        let job = {
            let mut queue = shared.queue.lock().expect("prefetch queue poisoned");
            loop {
                if queue.stop {
                    return;
                }
                if let Some(index) = lowest_priority(&queue.jobs) {
                    break queue.jobs.swap_remove(index);
                }
                queue = shared.wake.wait(queue).expect("prefetch queue poisoned");
            }
        };

        let result = decode_job(&job);
        if tx
            .send(Done {
                path: job.path,
                kind: job.kind,
                result,
            })
            .is_err()
        {
            return; // the app is gone
        }
        repaint.request_repaint();
    }
}

fn lowest_priority(jobs: &[Job]) -> Option<usize> {
    jobs.iter()
        .enumerate()
        .min_by_key(|(_, j)| j.priority)
        .map(|(i, _)| i)
}

/// A corrupt file must not take the process with it: image decoders are large
/// and a malformed JPEG occasionally panics rather than returning an error.
fn decode_job(job: &Job) -> Result<Arc<LoadedImage>, String> {
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        loader::load_preview(&job.path, job.max_px)
    }));

    match attempt {
        Ok(Ok(preview)) => {
            let place = preview
                .meta
                .gps
                .and_then(|(lat, lon)| CityDb::embedded().nearest(lat, lon));
            Ok(Arc::new(LoadedImage { preview, place }))
        }
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("the decoder crashed on this file".to_string()),
    }
}
