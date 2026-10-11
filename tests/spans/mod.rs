#![allow(dead_code)]
//! Test layer that records every span that closed: (name, `kind` field, target, level).
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

pub type Row = (String, String, String, tracing::Level);

#[derive(Clone, Default)]
pub struct Closed(pub Arc<Mutex<Vec<Row>>>);

struct Kind(String);

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
        if let (Some(span), Some(k)) = (ctx.span(id), v.0) {
            span.extensions_mut().insert(Kind(k));
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
            CURRENT.with(|c| {
                if let Some(closed) = c.borrow().as_ref() {
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
