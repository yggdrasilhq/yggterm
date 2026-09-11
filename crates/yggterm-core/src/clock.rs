//! Amortized wall clock for the trace plane's per-event stamps.
//!
//! A production host marked its TSC unstable on 2026-09-11 (a
//! 15.6-billion-cycle warp between cores; the kernel fell back to hpet),
//! which turned every `SystemTime::now()` from a ~25ns vDSO read into a ~1µs
//! syscall — and the per-event trace stamp was the trace plane's hottest
//! clock consumer. This module keeps the stamp truthful while paying the
//! syscall for a small fraction of events: a real read re-anchors the clock,
//! `rdtsc` deltas project the stamps in between.
//!
//! ⛔ The TSCs on hosts in this state are UNSYNCHRONIZED across cores — a
//! thread that migrates between anchors can see the raw counter jump by
//! seconds. The
//! projection therefore refuses backward counters, re-anchors at a fixed
//! measured window, and needs two real reads to learn the TSC rate, so a
//! stamp is never more than a few milliseconds from a real clock read.

/// Re-anchor at least this often, in projected milliseconds since the real
/// read. This bounds the stamp's drift to the TSC rate error over one window
/// — microseconds once the rate is learned from a wide-enough pair.
const REANCHOR_AFTER_MS: u64 = 50;

/// A rate learned from a pair of real reads closer than this is refused: the
/// pair's width is measured at integer-millisecond grain, so a 0-1ms pair
/// carries ratio error large enough to drift a whole re-anchor window.
const MIN_RATE_SAMPLE_MS: u64 = 5;

/// One real clock read paired with the raw TSC counter at that moment.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    unix_ms: u64,
    ticks: u64,
}

/// Projects wall-clock milliseconds from TSC deltas between two real reads.
#[derive(Debug, Clone, Copy)]
struct Amortized {
    anchor: Anchor,
    ticks_per_ms: f64,
}

impl Amortized {
    /// Projects `ticks` (read after `self.anchor`) to unix ms. `None` means
    /// "do not trust this projection — take a real read": the counter moved
    /// backwards (a migration to a lower-offset core) or no usable rate has
    /// been learned yet.
    fn project(&self, ticks: u64) -> Option<u64> {
        if !self.ticks_per_ms.is_finite() || self.ticks_per_ms <= 0.0 {
            return None;
        }
        let delta = ticks.checked_sub(self.anchor.ticks)?;
        Some(self.anchor.unix_ms + (delta as f64 / self.ticks_per_ms) as u64)
    }

    /// True when a projection from this anchor to `ticks` must not be used
    /// even though the counter moved forward: too much measured time has
    /// passed since the real read, or the counter cannot be projected at all.
    fn expired(&self, ticks: u64) -> bool {
        match self.project(ticks) {
            Some(ms) => ms.saturating_sub(self.anchor.unix_ms) > REANCHOR_AFTER_MS,
            None => true,
        }
    }
}

#[cfg(target_arch = "x86_64")]
thread_local! {
    static STATE: std::cell::RefCell<ThreadState> = std::cell::RefCell::new(ThreadState {
        prev: None,
        cur: None,
    });
}

#[cfg(target_arch = "x86_64")]
struct ThreadState {
    /// The previous real read, kept so the next one can relearn the TSC rate.
    prev: Option<Anchor>,
    /// The live projection, `None` until a rate has been learned.
    cur: Option<Amortized>,
}

fn real_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// Wall-clock unix milliseconds for trace stamps: one real read per anchor
/// window, TSC-projected in between. Falls back to a real read on every call
/// off x86_64, before the first two anchors, and whenever the projection
/// refuses the counter. The output carries the same semantics
/// `SystemTime::now()` did — wall time, millisecond grain, no monotonicity
/// guarantee across anchors — so every existing reader of `ts_ms` reads it
/// exactly as before.
pub fn amortized_unix_ms() -> u128 {
    #[cfg(not(target_arch = "x86_64"))]
    {
        real_unix_ms()
    }

    #[cfg(target_arch = "x86_64")]
    {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let ticks = unsafe { std::arch::x86_64::_rdtsc() };
            if let Some(cur) = state.cur {
                if !cur.expired(ticks) {
                    if let Some(ms) = cur.project(ticks) {
                        return u128::from(ms);
                    }
                }
            }
            // Re-anchor: a real read, and (when a previous real read exists)
            // a fresh measurement of the TSC rate smoothed into the old one.
            let now_ms = u64::try_from(real_unix_ms()).unwrap_or(u64::MAX);
            let now = Anchor {
                unix_ms: now_ms,
                ticks,
            };
            let learned = state.prev.and_then(|prev| {
                let dt = now_ms.checked_sub(prev.unix_ms)?;
                let dticks = ticks.checked_sub(prev.ticks)?;
                if dt < MIN_RATE_SAMPLE_MS {
                    return None;
                }
                let rate = dticks as f64 / dt as f64;
                // TSC rates on this fleet are 3-5 GHz; a rate outside the
                // band is a broken pair (an NTP step, a counter artifact),
                // not a measurement — refuse it and keep the old rate.
                (1_000.0..=10_000.0).contains(&rate).then_some(rate)
            });
            // Never average a live rate with a dead one (0.0 = not learned
            // yet): a halved rate makes the projection fall behind reality
            // by half a window every window.
            let base = state
                .cur
                .map(|old| old.ticks_per_ms)
                .filter(|rate| *rate > 0.0);
            state.cur = Some(Amortized {
                anchor: now,
                ticks_per_ms: match (base, learned) {
                    (Some(old_rate), Some(rate)) => (old_rate + rate) / 2.0,
                    (Some(old_rate), None) => old_rate,
                    (None, Some(rate)) => rate,
                    (None, None) => 0.0,
                },
            });
            state.prev = Some(now);
            u128::from(now_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_refuses_backward_counter_and_zero_rate() {
        let good = Amortized {
            anchor: Anchor {
                unix_ms: 1_000,
                ticks: 10_000,
            },
            ticks_per_ms: 3_300.0,
        };
        let expected = 1_000u64 + (3_000 as f64 / 3_300.0) as u64;
        assert_eq!(good.project(13_000), Some(expected));
        assert_eq!(good.project(9_999), None);
        let rateless = Amortized {
            anchor: Anchor {
                unix_ms: 5,
                ticks: 1,
            },
            ticks_per_ms: 0.0,
        };
        assert_eq!(rateless.project(2), None);
        assert!(rateless.expired(2), "a zero rate must force re-anchoring");
    }

    #[test]
    fn projection_expires_after_the_window() {
        let a = Amortized {
            anchor: Anchor {
                unix_ms: 0,
                ticks: 0,
            },
            ticks_per_ms: 3_300.0,
        };
        assert!(!a.expired(3_300 * u64::from(REANCHOR_AFTER_MS - 1)));
        assert!(a.expired(3_300 * u64::from(REANCHOR_AFTER_MS + 1)));
    }

    #[test]
    fn amortized_stays_near_the_real_clock() {
        for _ in 0..1_000 {
            let amortized = amortized_unix_ms();
            let real = real_unix_ms();
            assert!(
                real.abs_diff(amortized) <= 25,
                "amortized {amortized} vs real {real}"
            );
        }
    }

    #[test]
    fn amortized_survives_anchor_windows_and_stays_plausible() {
        let first = amortized_unix_ms();
        std::thread::sleep(std::time::Duration::from_millis(
            3 * u64::from(REANCHOR_AFTER_MS),
        ));
        let second = amortized_unix_ms();
        assert!(second > first, "time must move: {first} -> {second}");
        let real = real_unix_ms();
        assert!(
            real.abs_diff(second) <= 25,
            "after window expiry {second} vs real {real}"
        );
    }
}
