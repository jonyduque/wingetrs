//! The elevated helper (`wingetrs bridge-worker`) only starts from a folder only administrators
//! write (`elevate_win::start` checks it before UAC). The refusal text, shared by CLI and TUI,
//! and the transport that hands it to them once per session.
use super::elevated::{Helper, Transport};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Start of the refusal; the folder and a period follow.
pub const REFUSAL_START: &str = "Para usar funções de administrador, o wingetrs precisa estar numa pasta que só o administrador grava (ex.: C:\\Program Files\\wingetrs). Ele está em ";

/// The refusal for an executable in `folder`.
pub fn refusal_text(folder: &Path) -> String {
    format!("{REFUSAL_START}{}.", folder.display())
}

/// Whether `text` (a failed start of the helper) is the folder refusal.
pub fn is_refusal(text: &str) -> bool {
    text.starts_with(REFUSAL_START)
}

/// Callback that shows the refusal (a CLI line, a TUI toast).
pub type Notice = Box<dyn Fn(&str) + Send + Sync>;

/// `inner`, with the folder refusal handed to `notice` the first time only.
pub struct NoticeTransport {
    inner: Box<dyn Transport>,
    notice: Notice,
    told: AtomicBool,
}

impl NoticeTransport {
    pub fn new(inner: Box<dyn Transport>, notice: Notice) -> NoticeTransport {
        NoticeTransport {
            inner,
            notice,
            told: AtomicBool::new(false),
        }
    }
}

impl Transport for NoticeTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        self.start_with_notice(&mut || {})
    }

    fn start_with_notice(&self, notice: &mut dyn FnMut()) -> Result<Box<dyn Helper>, String> {
        self.inner.start_with_notice(notice).inspect_err(|e| {
            if is_refusal(e) && !self.told.swap(true, Ordering::SeqCst) {
                (self.notice)(e);
            }
        })
    }
}
