//! Rollback guard. Two high-water marks are kept: the largest entitlement `iat` seen, and the
//! largest wall-clock `now` observed. A system clock more than the tolerance behind either
//! means it was moved back to stretch an expiry. Only a successful online refresh, which
//! needs the real server, may lower the wall-clock mark.

use std::sync::Mutex;

use crate::error::AppError;

pub const ROLLBACK_TOLERANCE_SECONDS: i64 = 10 * 60;
/// Wall-clock progress is persisted at most this often; the in-memory mark is always current.
const PERSIST_INTERVAL_SECONDS: i64 = 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClockMarks {
    pub max_iat: Option<i64>,
    pub max_now: Option<i64>,
}

/// Persistence for the marks. The production store is the Keychain through the native layer;
/// tests use memory.
pub trait ClockStore: Send + Sync {
    fn load(&self) -> Result<ClockMarks, AppError>;
    fn store(&self, marks: ClockMarks) -> Result<(), AppError>;
}

struct State {
    marks: ClockMarks,
    persisted: ClockMarks,
}

pub struct ClockGuard {
    store: Box<dyn ClockStore>,
    state: Mutex<State>,
}

fn high_water(marks: ClockMarks) -> Option<i64> {
    marks.max_iat.max(marks.max_now)
}

impl ClockGuard {
    /// An unreadable store is an error, not "nothing seen": treating it as empty would let
    /// deleting the store defeat the guard.
    pub fn open(store: Box<dyn ClockStore>) -> Result<Self, AppError> {
        let marks = store.load()?;
        Ok(Self {
            store,
            state: Mutex::new(State {
                marks,
                persisted: marks,
            }),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("clock lock must not be poisoned")
    }

    pub fn is_rolled_back(&self, now: i64) -> bool {
        high_water(self.state().marks).is_some_and(|mark| now < mark - ROLLBACK_TOLERANCE_SECONDS)
    }

    /// The time to judge expiry against: a set-back inside the tolerance cannot extend a grant.
    pub fn effective(&self, now: i64) -> i64 {
        high_water(self.state().marks).map_or(now, |mark| now.max(mark))
    }

    /// Records the observed wall clock. Only moves forward.
    pub fn note_now(&self, now: i64) -> Result<(), AppError> {
        let mut state = self.state();
        if state.marks.max_now.is_some_and(|seen| now <= seen) {
            return Ok(());
        }
        state.marks.max_now = Some(now);
        let due = state
            .persisted
            .max_now
            .is_none_or(|saved| now - saved >= PERSIST_INTERVAL_SECONDS);
        if due {
            self.persist(&mut state)?;
        }
        Ok(())
    }

    /// Records a verified `iat`. Only moves forward.
    pub fn observe(&self, issued_at: i64) -> Result<(), AppError> {
        let mut state = self.state();
        if state.marks.max_iat.is_some_and(|seen| issued_at <= seen) {
            return Ok(());
        }
        state.marks.max_iat = Some(issued_at);
        self.persist(&mut state)
    }

    /// A fresh entitlement just verified against a server challenge: the server's time and
    /// this clock agree, so the wall-clock mark is re-anchored to `now`, even backwards.
    pub fn reset_after_online(&self, now: i64, issued_at: i64) -> Result<(), AppError> {
        let mut state = self.state();
        state.marks.max_now = Some(now);
        state.marks.max_iat = state.marks.max_iat.max(Some(issued_at));
        self.persist(&mut state)
    }

    fn persist(&self, state: &mut State) -> Result<(), AppError> {
        self.store.store(state.marks)?;
        state.persisted = state.marks;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Arc;

    #[derive(Default, Clone)]
    pub struct MemoryClockStore(pub Arc<Mutex<ClockMarks>>);

    impl ClockStore for MemoryClockStore {
        fn load(&self) -> Result<ClockMarks, AppError> {
            Ok(*self.0.lock().unwrap())
        }
        fn store(&self, marks: ClockMarks) -> Result<(), AppError> {
            *self.0.lock().unwrap() = marks;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryClockStore;
    use super::*;

    const T: i64 = 1_000_000;

    #[test]
    fn rollback_boundary_is_ten_minutes_for_wall_clock_and_iat() {
        for seed in ["now", "iat"] {
            let guard = ClockGuard::open(Box::new(MemoryClockStore::default())).unwrap();
            assert!(!guard.is_rolled_back(0), "nothing seen yet");
            match seed {
                "now" => guard.note_now(T).unwrap(),
                _ => guard.observe(T).unwrap(),
            }
            let limit = T - ROLLBACK_TOLERANCE_SECONDS;
            assert!(!guard.is_rolled_back(limit + 1), "{seed}");
            assert!(!guard.is_rolled_back(limit), "{seed}");
            assert!(guard.is_rolled_back(limit - 1), "{seed}");
        }
    }

    #[test]
    fn marks_survive_reopen_and_only_move_forward() {
        let store = MemoryClockStore::default();
        let guard = ClockGuard::open(Box::new(store.clone())).unwrap();
        guard.note_now(T).unwrap();
        guard.note_now(T - 5_000).unwrap();
        assert_eq!(store.0.lock().unwrap().max_now, Some(T));
        assert!(ClockGuard::open(Box::new(store))
            .unwrap()
            .is_rolled_back(T - 3_600));
    }

    #[test]
    fn normal_forward_time_never_trips_the_guard() {
        let guard = ClockGuard::open(Box::new(MemoryClockStore::default())).unwrap();
        for step in 0..100 {
            let now = T + step * 30;
            assert!(!guard.is_rolled_back(now));
            guard.note_now(now).unwrap();
        }
    }

    #[test]
    fn a_setback_inside_tolerance_cannot_extend_effective_time() {
        let guard = ClockGuard::open(Box::new(MemoryClockStore::default())).unwrap();
        guard.note_now(T).unwrap();
        assert_eq!(guard.effective(T - 300), T);
        assert_eq!(guard.effective(T + 5), T + 5);
    }

    #[test]
    fn only_an_online_reset_lowers_the_wall_clock_mark() {
        let guard = ClockGuard::open(Box::new(MemoryClockStore::default())).unwrap();
        guard.note_now(T + 100_000).unwrap();
        assert!(guard.is_rolled_back(T));
        guard.reset_after_online(T, T - 30).unwrap();
        assert!(!guard.is_rolled_back(T));
    }
}
