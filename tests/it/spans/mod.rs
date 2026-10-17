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
        use tracing_subscriber::layer::SubscriberExt;
        static GLOBAL: std::sync::Once = std::sync::Once::new();
        GLOBAL.call_once(|| {
            let _ = tracing::subscriber::set_global_default(
                tracing_subscriber::registry().with(Router),
            );
        });
        CURRENT.with(|c| *c.borrow_mut() = Some(self.clone()));
        Installed
    }
}

/// Stops recording on drop.
pub struct Installed;

impl Drop for Installed {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = None);
    }
}
