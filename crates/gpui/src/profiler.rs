use scheduler::Instant;
use std::{
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    sync::LazyLock,
    thread::{self, ThreadId},
};

use serde::{Deserialize, Serialize};

use crate::SharedString;

#[doc(hidden)]
#[derive(Debug, Copy, Clone)]
pub struct TaskTiming {
    pub location: &'static core::panic::Location<'static>,
    pub start: Instant,
    pub end: Option<Instant>,
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ThreadTaskTimings {
    pub thread_name: Option<String>,
    pub thread_id: ThreadId,
    pub timings: Vec<TaskTiming>,
    pub total_pushed: u64,
}

impl ThreadTaskTimings {
    /// Collect a per-thread view of the current task timing buffer.
    pub fn collect_all() -> Vec<Self> {
        let store = PROFILER_STORE.lock();
        let mut by_thread: HashMap<ThreadId, ThreadTaskTimings> = HashMap::new();
        for entry in store.timings.iter() {
            let bucket = by_thread
                .entry(entry.thread_id)
                .or_insert_with(|| ThreadTaskTimings {
                    thread_name: store.names.get(&entry.thread_id).cloned(),
                    thread_id: entry.thread_id,
                    timings: Vec::new(),
                    total_pushed: store
                        .total_pushed_by_thread
                        .get(&entry.thread_id)
                        .copied()
                        .unwrap_or_default(),
                });
            bucket.timings.push(entry.timing);
        }
        by_thread.into_values().collect()
    }

    /// Collect a view of the timings that originated on the current thread.
    pub fn collect_current() -> Self {
        let thread_id = thread::current().id();
        let store = PROFILER_STORE.lock();
        let timings = store
            .timings
            .iter()
            .filter(|entry| entry.thread_id == thread_id)
            .map(|entry| entry.timing)
            .collect::<Vec<_>>();
        ThreadTaskTimings {
            thread_name: store.names.get(&thread_id).cloned(),
            thread_id,
            timings,
            total_pushed: store
                .total_pushed_by_thread
                .get(&thread_id)
                .copied()
                .unwrap_or_default(),
        }
    }
}

/// Serializable variant of [`core::panic::Location`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedLocation {
    /// Name of the source file
    pub file: SharedString,
    /// Line in the source file
    pub line: u32,
    /// Column in the source file
    pub column: u32,
}

impl From<&core::panic::Location<'static>> for SerializedLocation {
    fn from(value: &core::panic::Location<'static>) -> Self {
        SerializedLocation {
            file: value.file().into(),
            line: value.line(),
            column: value.column(),
        }
    }
}

/// Serializable variant of [`TaskTiming`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedTaskTiming {
    /// Location of the timing
    pub location: SerializedLocation,
    /// Time at which the measurement was reported in nanoseconds
    pub start: u128,
    /// Duration of the measurement in nanoseconds
    pub duration: u128,
}

impl SerializedTaskTiming {
    /// Convert an array of [`TaskTiming`] into their serializable format
    ///
    /// # Params
    ///
    /// `anchor` - [`Instant`] that should be earlier than all timings to use as base anchor
    pub fn convert(anchor: Instant, timings: &[TaskTiming]) -> Vec<SerializedTaskTiming> {
        let serialized = timings
            .iter()
            .map(|timing| {
                let start = timing.start.duration_since(anchor).as_nanos();
                let duration = timing
                    .end
                    .unwrap_or_else(|| Instant::now())
                    .duration_since(timing.start)
                    .as_nanos();
                SerializedTaskTiming {
                    location: timing.location.into(),
                    start,
                    duration,
                }
            })
            .collect::<Vec<_>>();

        serialized
    }
}

/// Serializable variant of [`ThreadTaskTimings`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedThreadTaskTimings {
    /// Thread name
    pub thread_name: Option<String>,
    /// Hash of the thread id
    pub thread_id: u64,
    /// Timing records for this thread
    pub timings: Vec<SerializedTaskTiming>,
}

impl SerializedThreadTaskTimings {
    /// Convert [`ThreadTaskTimings`] into their serializable format
    ///
    /// # Params
    ///
    /// `anchor` - [`Instant`] that should be earlier than all timings to use as base anchor
    pub fn convert(anchor: Instant, timings: ThreadTaskTimings) -> SerializedThreadTaskTimings {
        let serialized_timings = SerializedTaskTiming::convert(anchor, &timings.timings);

        let mut hasher = DefaultHasher::new();
        timings.thread_id.hash(&mut hasher);
        let thread_id = hasher.finish();

        SerializedThreadTaskTimings {
            thread_name: timings.thread_name,
            thread_id,
            timings: serialized_timings,
        }
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ThreadTimingsDelta {
    /// Hashed thread id
    pub thread_id: u64,
    /// Thread name, if known
    pub thread_name: Option<String>,
    /// New timings since the last call. If the circular buffer wrapped around
    /// since the previous poll, some entries may have been lost.
    pub new_timings: Vec<SerializedTaskTiming>,
}

/// Tracks which timing events have already been seen so that callers can request only unseen events.
#[doc(hidden)]
pub struct ProfilingCollector {
    startup_time: Instant,
    cursors: HashMap<ThreadId, u64>,
}

impl ProfilingCollector {
    pub fn new(startup_time: Instant) -> Self {
        Self {
            startup_time,
            cursors: HashMap::default(),
        }
    }

    pub fn startup_time(&self) -> Instant {
        self.startup_time
    }

    pub fn collect_unseen(
        &mut self,
        all_timings: Vec<ThreadTaskTimings>,
    ) -> Vec<ThreadTimingsDelta> {
        let mut deltas = Vec::with_capacity(all_timings.len());

        for thread in all_timings {
            let mut hasher = DefaultHasher::new();
            thread.thread_id.hash(&mut hasher);
            let hashed_id = hasher.finish();

            let prev_cursor = self.cursors.get(&thread.thread_id).copied().unwrap_or(0);
            let buffer_len = thread.timings.len() as u64;
            let buffer_start = thread.total_pushed.saturating_sub(buffer_len);

            let mut slice = if prev_cursor < buffer_start {
                // Cursor fell behind the buffer — some entries were evicted.
                // Return everything still in the buffer.
                thread.timings.as_slice()
            } else {
                let skip = (prev_cursor - buffer_start) as usize;
                &thread.timings[skip.min(thread.timings.len())..]
            };

            // Don't emit the last entry if it's still in-progress (end: None).
            let incomplete_at_end = slice.last().is_some_and(|t| t.end.is_none());
            if incomplete_at_end {
                slice = &slice[..slice.len() - 1];
            }

            let cursor_advance = if incomplete_at_end {
                thread.total_pushed.saturating_sub(1)
            } else {
                thread.total_pushed
            };

            self.cursors.insert(thread.thread_id, cursor_advance);

            if slice.is_empty() {
                continue;
            }

            let new_timings = SerializedTaskTiming::convert(self.startup_time, slice);

            deltas.push(ThreadTimingsDelta {
                thread_id: hashed_id,
                thread_name: thread.thread_name,
                new_timings,
            });
        }

        deltas
    }

    pub fn reset(&mut self) {
        self.cursors.clear();
    }
}

/// Total cap on retained task timings. This is shared across all threads;
/// when the buffer fills up, the oldest entry is evicted regardless of
/// which thread produced it.
///
/// Keeping a single bounded buffer — rather than one per thread — avoids
/// leaking memory when worker threads exit without running TLS destructors
/// (which happens routinely for Apple's libdispatch and the Windows thread
/// pool): otherwise every such exited worker would leak its own ring buffer.
const MAX_TASK_TIMINGS: usize = (16 * 1024 * 1024) / core::mem::size_of::<StoredTiming>();

static PROFILER_STORE: LazyLock<spin::Mutex<ProfilerStore>> = LazyLock::new(|| {
    spin::Mutex::new(ProfilerStore {
        timings: VecDeque::with_capacity(MAX_TASK_TIMINGS),
        total_pushed_by_thread: HashMap::new(),
        names: HashMap::new(),
    })
});

#[doc(hidden)]
pub fn add_task_timing(timing: TaskTiming) {
    PROFILER_STORE.lock().push(timing);
}

#[derive(Copy, Clone)]
struct StoredTiming {
    thread_id: ThreadId,
    timing: TaskTiming,
}

struct ProfilerStore {
    timings: VecDeque<StoredTiming>,
    /// `thread_id -> total number of timings ever pushed from this thread`.
    /// Needed so `ProfilingCollector` can compute how far a thread's cursor
    /// has advanced even after old entries have been evicted.
    total_pushed_by_thread: HashMap<ThreadId, u64>,
    /// `thread_id -> thread name` captured on each thread's first push.
    names: HashMap<ThreadId, String>,
}

impl ProfilerStore {
    fn push(&mut self, timing: TaskTiming) {
        let current_thread = thread::current();
        let thread_id = current_thread.id();

        // Coalesce the pre-run and post-run pair emitted by the dispatcher
        // trampoline for the same task into a single entry.
        if let Some(last) = self.timings.back_mut()
            && last.thread_id == thread_id
            && last.timing.location == timing.location
            && last.timing.start == timing.start
        {
            last.timing.end = timing.end;
            return;
        }

        if self.timings.len() == MAX_TASK_TIMINGS {
            self.timings.pop_front();
        }

        if let Some(name) = current_thread.name() {
            self.names
                .entry(thread_id)
                .or_insert_with(|| name.to_owned());
        }
        *self.total_pushed_by_thread.entry(thread_id).or_insert(0) += 1;
        self.timings.push_back(StoredTiming { thread_id, timing });
    }
}
