/// Lock holder instrumentation for daemon observability.
///
/// **M1 — NAME THE LOCK HOLDER**: Track which request is holding the runtime
/// lock, so when another request waits, the trace can name what was holding it.
///
/// **M2 — DETECT BLOCKING CALLS INSIDE LOCK**: Warn if expensive remote calls
/// occur while the lock is held. The three known instances are: snapshot fan-out,
/// remote-cwd resolve, TerminalRead hoist.
///
/// When a trace shows `lock_wait_window` with high p50/p95, cross-reference the
/// timestamp with `lock_holder` spans to see what was executing.
use std::cell::RefCell;
use std::time::Instant;

/// Thread-local state tracking lock holder context.
///
/// Records which request is currently holding the runtime lock, when it started,
/// and whether expensive remote calls occur inside it.
thread_local! {
    static LOCK_HOLDER: RefCell<Option<LockHolderSpan>> = RefCell::new(None);
}

#[derive(Clone, Debug)]
pub struct LockHolderSpan {
    /// The name of the request that acquired the lock.
    pub request_name: &'static str,
    /// Wall-clock time when the lock was acquired.
    pub acquired_at: Instant,
    /// Count of blocking remote calls made while holding this lock.
    pub remote_call_count: u32,
    /// Total time spent in remote calls while holding this lock.
    pub remote_call_total_ms: u128,
}

impl LockHolderSpan {
    pub fn new(request_name: &'static str) -> Self {
        Self {
            request_name,
            acquired_at: Instant::now(),
            remote_call_count: 0,
            remote_call_total_ms: 0,
        }
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.acquired_at.elapsed().as_millis()
    }
}

/// Enter a lock holder context. Called when acquiring the runtime lock.
pub fn enter_lock_holder(request_name: &'static str) {
    LOCK_HOLDER.with(|holder| {
        *holder.borrow_mut() = Some(LockHolderSpan::new(request_name));
    });
}

/// Record a blocking remote call inside the lock.
///
/// **M2**: This detects expensive remote calls that occur while the lock is held.
/// If a call takes > threshold_ms, a trace warning is emitted.
pub fn record_blocking_call_inside_lock(
    home: &std::path::Path,
    call_name: &str,
    cost_ms: u128,
    peer_label: Option<&str>,
) {
    const BLOCKING_CALL_THRESHOLD_MS: u128 = 50; // Any call over 50ms while holding lock is suspicious.

    LOCK_HOLDER.with(|holder| {
        let mut state = holder.borrow_mut();
        if let Some(span) = &mut *state {
            span.remote_call_count += 1;
            span.remote_call_total_ms += cost_ms;

            if cost_ms >= BLOCKING_CALL_THRESHOLD_MS {
                // Emit a warning trace for blocking calls inside lock.
                yggterm_core::append_trace_event(
                    home,
                    "daemon",
                    "perf",
                    "blocking_call_inside_lock",
                    serde_json::json!({
                        "lock_held_by": span.request_name,
                        "lock_held_for_ms": span.elapsed_ms(),
                        "call_name": call_name,
                        "call_cost_ms": cost_ms,
                        "peer": peer_label,
                        "total_remote_calls_in_span": span.remote_call_count,
                        "total_remote_time_in_span_ms": span.remote_call_total_ms,
                    }),
                );
            }
        }
    });
}

/// Exit a lock holder context. Called when releasing the runtime lock.
///
/// Emits a trace span recording what work was done while holding the lock.
pub fn exit_lock_holder(home: &std::path::Path) {
    LOCK_HOLDER.with(|holder| {
        if let Some(span) = holder.borrow_mut().take() {
            let held_ms = span.elapsed_ms();
            let cpu_cost_estimate_ms = span.remote_call_total_ms;

            // Only emit if the lock was held for a notable duration.
            // This keeps the trace volume reasonable (most requests are fast).
            if held_ms > 10 {
                yggterm_core::append_trace_event(
                    home,
                    "daemon",
                    "request",
                    "lock_holder",
                    serde_json::json!({
                        "request": span.request_name,
                        "held_ms": held_ms,
                        "remote_calls": span.remote_call_count,
                        "remote_call_total_ms": span.remote_call_total_ms,
                    }),
                );
            }
        }
    });
}

/// Get the current lock holder, if any.
pub fn current_lock_holder() -> Option<LockHolderSpan> {
    LOCK_HOLDER.with(|holder| holder.borrow().clone())
}

/// Annotate a lock wait event with the lock holder that caused the wait.
///
/// When a request waits on the lock, this function can be called to record
/// what was holding it. The trace then has full context: the waiter, the holder,
/// and the duration.
pub fn append_lock_wait_with_holder(
    home: &std::path::Path,
    waiting_request: &str,
    wait_duration_us: u64,
) {
    LOCK_HOLDER.with(|holder| {
        if let Some(span) = &*holder.borrow() {
            yggterm_core::append_trace_event(
                home,
                "daemon",
                "request",
                "lock_wait_identified",
                serde_json::json!({
                    "waiting_request": waiting_request,
                    "wait_duration_us": wait_duration_us,
                    "lock_held_by": span.request_name,
                    "lock_held_for_ms": span.elapsed_ms(),
                    "holder_remote_calls": span.remote_call_count,
                }),
            );
        }
    });
}
