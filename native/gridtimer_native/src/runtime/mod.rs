// v0.0.1 - Share ownership-aware supervisor mutex handling across Windows entry points.
// v2.22.25 - Export the managed desktop task runtime.

pub mod bounded_io;
pub mod cancellation;
pub mod latest_worker;
#[cfg(windows)]
pub mod named_mutex;
pub mod task_supervisor;
