#![allow(dead_code)]
//! Test layer that records every span that closed: (name, `kind` field, target, level).
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

pub type Row = (String, String, String, tracing::Level);

#[derive(Clone, Default)]
/// Rows of spans that were entered at least once, plus `(child name, parent name)` pairs.
pub struct Closed(
    pub Arc<Mutex<Vec<Row>>>,
    pub Arc<Mutex<Vec<(String, String)>>>,
);

struct Kind(String);

/// Parent span name (empty = root) and whether the span was ever entered.
struct Info {
    parent: String,
    entered: bool,
}

struct KindVisitor(Option<String>);

impl tracing::field::Visit for KindVisitor {
    fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
        if f.name() == "kind" {
            self.0 = Some(v.to_string());
        }
    }
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        if f.name() == "kind" {
            self.0 = Some(format!("{v:?}").trim_matches('"').to_string());
        }
    }
}

thread_local! {
    static CURRENT: std::cell::RefCell<Option<Closed>> = const { std::cell::RefCell::new(None) };
}

/// Routes closed spans to the `Closed` installed on the closing thread.
struct Router;

impl<S> tracing_subscriber::Layer<S> for Router
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        let mut v = KindVisitor(None);
        attrs.record(&mut v);
        let parent = ctx
            .span(id)
            .and_then(|s| s.parent())
            .map(|p| p.name().to_string())
            .unwrap_or_default();
        if let Some(span) = ctx.span(id) {
            let mut ext = span.extensions_mut();
            if let Some(k) = v.0 {
                ext.insert(Kind(k));
            }
            ext.insert(Info {
                parent,
                entered: false,
            });
        }
    }

    fn on_enter(&self, id: &tracing::span::Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(i) = span.extensions_mut().get_mut::<Info>()
        {
            i.entered = true;
        }
    }

    fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            let kind = span
                .extensions()
                .get::<Kind>()
                .map(|k| k.0.clone())
                .unwrap_or_default();
            let m = span.metadata();
            let (parent, entered) = span
                .extensions()
                .get::<Info>()
                .map(|i| (i.parent.clone(), i.entered))
                .unwrap_or_default();
            if !entered {
                return;
            }
            CURRENT.with(|c| {
                if let Some(closed) = c.borrow().as_ref() {
                    closed
                        .1
                        .lock()
                        .unwrap()
                        .push((m.name().to_string(), parent));
                    closed.0.lock().unwrap().push((
                        m.name().to_string(),
                        kind,
                        m.target().to_string(),
                        *m.level(),
                    ));
                }
            });
        }
    }
}

impl Closed {
    /// `name` or `name:kind`, in closing order.
    pub fn names(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(n, k, _, _)| {
                if k.is_empty() {
                    n.clone()
                } else {
                    format!("{n}:{k}")
                }
            })
            .collect()
    }

    /// Parent names seen for spans called `name` (empty string = no parent).
    pub fn parents(&self, name: &str) -> Vec<String> {
        self.1
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, p)| p.clone())
            .collect()
    }

    pub fn count(&self, name: &str) -> usize {
        self.names().iter().filter(|n| n.as_str() == name).count()
    }

    /// Records the spans closed on this thread into `self` until the guard drops. The
    /// subscriber is process-global (installed once): a per-test `set_default` raced with
    /// other tests of the binary on the callsite interest cache and made spans vanish.
    pub fn install(&self) -> Installed {
        install_global();
        CURRENT.with(|c| *c.borrow_mut() = Some(self.clone()));
        Installed
    }
}

/// The binary's one subscriber: `Router` for spans, plus formatted DEBUG events for the
/// threads that installed a `Log`.
fn install_global() {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    static GLOBAL: std::sync::Once = std::sync::Once::new();
    GLOBAL.call_once(|| {
        let events = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(|| ThreadLog)
            .with_filter(tracing_subscriber::filter::filter_fn(|m| {
                *m.level() <= tracing::Level::DEBUG && LOG.with(|l| l.borrow().is_some())
            }));
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(Router).with(events),
        );
    });
}

thread_local! {
    static LOG: std::cell::RefCell<Option<Log>> = const { std::cell::RefCell::new(None) };
}

/// Formatted tracing output (DEBUG and up) of the threads it is installed on.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<Vec<u8>>>);

impl Log {
    /// Records this thread's events into `self` until the guard drops. Same global
    /// subscriber as `Closed::install`, for the same reason.
    pub fn install(&self) -> LogInstalled {
        install_global();
        LOG.with(|l| *l.borrow_mut() = Some(self.clone()));
        LogInstalled
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// Stops recording events on drop.
pub struct LogInstalled;

impl Drop for LogInstalled {
    fn drop(&mut self) {
        LOG.with(|l| *l.borrow_mut() = None);
    }
}

/// Writes into the calling thread's `Log`, if any.
struct ThreadLog;

impl std::io::Write for ThreadLog {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        LOG.with(|l| {
            if let Some(log) = l.borrow().as_ref() {
                log.0.lock().unwrap().extend_from_slice(b);
            }
        });
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Stops recording on drop.
pub struct Installed;

impl Drop for Installed {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = None);
    }
}
