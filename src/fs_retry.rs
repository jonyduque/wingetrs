//! Retries filesystem operations that fail transiently because another process (an antivirus
//! scanner, the indexer) briefly holds a handle on a file we just wrote.
use std::io;
use std::path::Path;
use std::time::Duration;

/// Waits between attempts: doubling from 10 ms, 1.27 s in total. Task 78 measured an external
/// holder releasing a fresh extraction folder after 16 ms and transient locks up to 228 ms under
/// load; the ladder covers more than twice that and stays under 2 s.
pub const DELAYS: &[Duration] = &[
    Duration::from_millis(10),
    Duration::from_millis(20),
    Duration::from_millis(40),
    Duration::from_millis(80),
    Duration::from_millis(160),
    Duration::from_millis(320),
    Duration::from_millis(640),
];

/// Windows ERROR_SHARING_VIOLATION.
const SHARING_VIOLATION: i32 = 32;

/// "Access denied" (also what renaming a folder with an open child gives) or a sharing violation.
pub fn is_transient(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(SHARING_VIOLATION)
}

/// Runs `f`; after each transient error sleeps the next delay and tries again. A non-transient
/// error, or a transient one after the last delay, is returned as is.
pub fn retry_with<T>(
    delays: &[Duration],
    mut f: impl FnMut() -> io::Result<T>,
    mut sleep: impl FnMut(Duration),
) -> io::Result<T> {
    let mut last = match f() {
        Err(e) if is_transient(&e) => e,
        r => return r,
    };
    for d in delays {
        sleep(*d);
        last = match f() {
            Err(e) if is_transient(&e) => e,
            r => return r,
        };
    }
    Err(last)
}

/// [`retry_with`] over [`DELAYS`] with real sleeps, logging each retry and the final give-up.
pub fn retry<T>(op: &'static str, path: &Path, f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut attempt = 0usize;
    let r = retry_with(DELAYS, f, |d| {
        attempt += 1;
        tracing::debug!(op, path = %path.display(), attempt, "transient fs error, retrying");
        std::thread::sleep(d);
    });
    if let Err(e) = &r
        && attempt == DELAYS.len()
        && is_transient(e)
    {
        tracing::warn!(op, path = %path.display(), error = %e, "transient fs error persisted, giving up");
    }
    r
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn denied() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn retry_with_succeeds_after_transient_errors() {
        let mut calls = 0;
        let mut slept = Vec::new();
        let r = retry_with(
            DELAYS,
            || {
                calls += 1;
                if calls <= 2 { Err(denied()) } else { Ok(7) }
            },
            |d| slept.push(d),
        );
        assert_eq!(r.unwrap(), 7);
        assert_eq!(slept, vec![DELAYS[0], DELAYS[1]]);
    }

    #[test]
    fn retry_with_gives_up_after_the_last_delay() {
        let mut slept = Vec::new();
        let r: io::Result<()> = retry_with(DELAYS, || Err(denied()), |d| slept.push(d));
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(slept, DELAYS.to_vec());
    }

    #[test]
    fn retry_with_does_not_retry_other_errors() {
        let mut slept = Vec::new();
        let r: io::Result<()> = retry_with(
            DELAYS,
            || Err(io::Error::from(io::ErrorKind::NotFound)),
            |d| slept.push(d),
        );
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(slept.is_empty());
        // A sharing violation is transient even though its kind is not PermissionDenied.
        let mut calls = 0;
        let r = retry_with(
            DELAYS,
            || {
                calls += 1;
                if calls == 1 {
                    Err(io::Error::from_raw_os_error(32))
                } else {
                    Ok(1)
                }
            },
            |d| slept.push(d),
        );
        assert_eq!(r.unwrap(), 1);
        assert_eq!(slept, vec![DELAYS[0]]);
    }

    #[test]
    fn delays_double_from_10_ms_and_stay_under_2_s() {
        assert_eq!(DELAYS[0], Duration::from_millis(10));
        for w in DELAYS.windows(2) {
            assert_eq!(w[1], w[0] * 2);
        }
        assert!(DELAYS.iter().sum::<Duration>() <= Duration::from_secs(2));
    }

    #[cfg(windows)]
    #[test]
    fn rename_of_a_dir_with_an_open_child_waits_for_release() {
        use std::fs;
        use std::os::windows::fs::OpenOptionsExt;
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        fs::create_dir(&a).unwrap();
        fs::write(a.join("f.txt"), b"x").unwrap();
        // Full sharing (read|write|delete), like a scanner: the folder rename still fails.
        let open = || {
            fs::OpenOptions::new()
                .read(true)
                .share_mode(1 | 2 | 4)
                .open(a.join("f.txt"))
                .unwrap()
        };
        let mut held = Some(open());
        retry_with(
            DELAYS,
            || fs::rename(&a, &b),
            |_| {
                held.take();
            },
        )
        .unwrap();
        assert!(b.join("f.txt").exists());
        assert!(held.is_none());

        fs::rename(&b, &a).unwrap();
        let _held = open();
        let mut slept = 0;
        let e = retry_with(DELAYS, || fs::rename(&a, &b), |_| slept += 1).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(5));
        assert_eq!(slept, DELAYS.len());
    }
}

/// Test layer: on the thread that installed it, each "retrying" event drops the held file.
#[cfg(test)]
pub(crate) mod release_on_retry {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::fs::File;
    use std::sync::{Arc, Mutex};

    pub(crate) type Held = Arc<Mutex<Option<File>>>;

    thread_local! {
        static CURRENT: std::cell::RefCell<Option<Held>> = const { std::cell::RefCell::new(None) };
    }

    struct Message(bool);

    impl tracing::field::Visit for Message {
        fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
            if f.name() == "message" && format!("{v:?}") == "transient fs error, retrying" {
                self.0 = true;
            }
        }
    }

    struct Releaser;

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Releaser {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut m = Message(false);
            event.record(&mut m);
            if m.0 {
                CURRENT.with(|c| {
                    if let Some(h) = c.borrow().as_ref() {
                        h.lock().unwrap().take();
                    }
                });
            }
        }
    }

    /// Process-global subscriber (installed once: a per-test `set_default` races on the
    /// callsite interest cache) routing to the calling thread's `held` until the guard drops.
    pub(crate) fn install(held: &Held) -> Installed {
        use tracing_subscriber::layer::SubscriberExt;
        static GLOBAL: std::sync::Once = std::sync::Once::new();
        GLOBAL.call_once(|| {
            let _ = tracing::subscriber::set_global_default(
                tracing_subscriber::registry().with(Releaser),
            );
        });
        CURRENT.with(|c| *c.borrow_mut() = Some(held.clone()));
        Installed
    }

    pub(crate) struct Installed;

    impl Drop for Installed {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = None);
        }
    }
}
