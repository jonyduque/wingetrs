//! What the TUI shows, as plain lines for `WINGETRS_TRANSCRIPT`: new toasts, the dialog on
//! screen and the job under way. Pure: the event loop writes the lines to the transcript.
use crate::install::progress::phase_title;
use crate::tui::app::{App, Modal, ToastKind};
use crate::tui::view::modal_title;

const PREFIX: &str = "[tui] ";

/// What was already reported, so each change becomes one line and a still screen none.
#[derive(Default)]
pub struct Observer {
    /// Toasts of the last look, oldest first, with their repeat counts.
    toasts: Vec<(ToastKind, String, u32)>,
    /// The lines of the dialog on screen; `None` with no dialog.
    modal: Option<Vec<String>>,
    /// Label and phase title of the job in progress; `None` with no job.
    job: Option<(String, Option<String>)>,
}

impl Observer {
    /// The lines for what changed in `app` since the last call.
    pub fn observe(&mut self, app: &App) -> Vec<String> {
        let mut lines = Vec::new();
        self.toasts_lines(app, &mut lines);
        self.modal_lines(app, &mut lines);
        self.job_lines(app, &mut lines);
        lines
    }

    fn toasts_lines(&mut self, app: &App, lines: &mut Vec<String>) {
        // `push_toast` inserts at the front: oldest first reads in the order they happened.
        let now: Vec<(ToastKind, String, u32)> = app
            .toasts
            .iter()
            .rev()
            .map(|t| (t.kind, t.text.clone(), t.count))
            .collect();
        for (kind, text, count) in &now {
            let before = self
                .toasts
                .iter()
                .find(|(k, t, _)| k == kind && t == text)
                .map(|(_, _, c)| *c);
            if before.is_none_or(|c| *count > c) {
                lines.push(format!("{PREFIX}toast {}: {text}", kind_word(*kind)));
            }
        }
        self.toasts = now;
    }

    fn modal_lines(&mut self, app: &App, lines: &mut Vec<String>) {
        let now = app.modal.as_ref().map(modal_lines);
        if now != self.modal {
            match &now {
                Some(shown) => lines.extend(shown.iter().cloned()),
                None => lines.push(format!("{PREFIX}modal fechado")),
            }
            self.modal = now;
        }
    }

    fn job_lines(&mut self, app: &App, lines: &mut Vec<String>) {
        let now = app.running.as_ref().map(|r| {
            let phase = r.progress.as_ref().map(|p| match &p.phase {
                Some((phase, _)) => phase_title(phase),
                None => "baixando".to_string(),
            });
            (r.label.clone(), phase)
        });
        if now == self.job {
            return;
        }
        match (&now, &self.job) {
            (Some((label, Some(phase))), _) => {
                lines.push(format!("{PREFIX}job: {label} · {phase}"));
            }
            (Some((label, None)), _) => lines.push(format!("{PREFIX}job: {label}")),
            (None, Some((label, _))) => lines.push(format!("{PREFIX}job terminou: {label}")),
            (None, None) => {}
        }
        self.job = now;
    }
}

fn kind_word(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::Success => "ok",
        ToastKind::Info => "info",
        ToastKind::Warning => "aviso",
        ToastKind::Error => "erro",
    }
}

/// The title of the dialog and what it says in text (items, notes, the quit question).
fn modal_lines(m: &Modal) -> Vec<String> {
    let mut lines = vec![format!("{PREFIX}modal: {}", modal_title(m))];
    match m {
        Modal::Confirm { items, notes, .. } => {
            for it in items {
                let mut line = format!("{PREFIX}  {}", it.name);
                if let Some(from) = &it.from {
                    line.push(' ');
                    line.push_str(from);
                }
                if let Some(to) = &it.to {
                    line.push_str(" → ");
                    line.push_str(to);
                }
                line.push_str(&format!(" ({})", it.manager));
                lines.push(line);
            }
            for note in notes {
                lines.push(format!("{PREFIX}  nota: {note}"));
            }
        }
        Modal::ConfirmQuit { text } => {
            for l in text.lines() {
                lines.push(format!("{PREFIX}  {l}"));
            }
        }
        _ => {}
    }
    lines
}
