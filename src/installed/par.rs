//! Order-preserving parallel map with a shared work index.
use std::sync::atomic::{AtomicUsize, Ordering};

/// Workers pull the next index, so one slow item does not stall a fixed chunk.
/// Items for which `f` returns `None` are skipped; a worker panic is propagated.
pub fn parallel<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    f: impl Fn(&T) -> Option<R> + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let mut slots: Vec<(usize, R)> = Vec::new();
    std::thread::scope(|s| {
        let (f, next) = (&f, &next);
        let handles: Vec<_> = (0..workers.max(1).min(items.len().max(1)))
            .map(|_| {
                s.spawn(move || {
                    let mut local = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        if let Some(r) = f(item) {
                            local.push((i, r));
                        }
                    }
                    local
                })
            })
            .collect();
        for h in handles {
            match h.join() {
                Ok(mut v) => slots.append(&mut v),
                Err(p) => std::panic::resume_unwind(p),
            }
        }
    });
    slots.sort_by_key(|(i, _)| *i);
    slots.into_iter().map(|(_, r)| r).collect()
}
