#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Windows Installer queue: mutex wait, 1618 retry and `msi.lock`, with fake busy/launch/sleep.
use std::cell::RefCell;
use std::io;
use std::sync::mpsc::channel;
use std::time::Duration;

use tempfile::TempDir;
use wingetrs::install::layout::Layout;
use wingetrs::install::msi_queue::{
    self, Attempt, MsiQueueError, POLICY, Policy, Status, attempt_from_code,
};

const HANG: Duration = Duration::from_secs(60);

/// Records every sleep and status; `busy` and `launch` answer from scripted lists.
struct Script {
    busy: Vec<bool>,
    attempts: Vec<io::Result<Attempt>>,
}

#[derive(Debug, PartialEq)]
enum Event {
    Busy,
    Launch,
    Sleep(Duration),
    Status(Status),
}

fn run(policy: &Policy, script: Script) -> (Result<(), MsiQueueError>, Vec<Event>) {
    let events = RefCell::new(Vec::new());
    let mut busy_answers = script.busy.into_iter();
    let mut attempts = script.attempts.into_iter();
    let mut busy = || {
        events.borrow_mut().push(Event::Busy);
        busy_answers.next().unwrap_or(false)
    };
    let mut launch = || {
        events.borrow_mut().push(Event::Launch);
        attempts.next().expect("no more attempts scripted")
    };
    let mut sleep = |d: Duration| events.borrow_mut().push(Event::Sleep(d));
    let mut status = |s: Status| events.borrow_mut().push(Event::Status(s));
    let r = msi_queue::run_queued(policy, &mut busy, &mut launch, &mut sleep, &mut status);
    (r, events.into_inner())
}

fn sleeps(events: &[Event]) -> Vec<Duration> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Sleep(d) => Some(*d),
            _ => None,
        })
        .collect()
}

#[test]
fn policy_values() {
    assert_eq!(POLICY.poll, Duration::from_secs(2));
    assert_eq!(POLICY.wait_max, Duration::from_secs(600));
    assert_eq!(
        POLICY.retries,
        [
            Duration::from_secs(5),
            Duration::from_secs(15),
            Duration::from_secs(45)
        ]
    );
}

#[test]
fn waits_while_installer_busy() {
    let (r, events) = run(
        &POLICY,
        Script {
            busy: vec![true, true, true, false],
            attempts: vec![Ok(Attempt::Done)],
        },
    );
    assert!(r.is_ok(), "{r:?}");
    let poll = POLICY.poll;
    assert_eq!(
        events,
        [
            Event::Busy,
            Event::Status(Status::WaitingInstaller),
            Event::Sleep(poll),
            Event::Busy,
            Event::Sleep(poll),
            Event::Busy,
            Event::Sleep(poll),
            Event::Busy,
            Event::Status(Status::Running { attempt: 1 }),
            Event::Launch,
        ]
    );
}

#[test]
fn busy_too_long_fails() {
    let (r, events) = run(
        &POLICY,
        Script {
            busy: vec![true; 10_000],
            attempts: Vec::new(),
        },
    );
    assert_eq!(r, Err(MsiQueueError::InstallerBusy));
    let checks = events.iter().filter(|e| **e == Event::Busy).count();
    let expected = (POLICY.wait_max.as_secs() / POLICY.poll.as_secs()) as usize;
    assert_eq!(checks, expected);
    assert!(!events.contains(&Event::Launch));
    assert_eq!(
        MsiQueueError::InstallerBusy.to_string(),
        "o Windows Installer está ocupado por outro programa"
    );
}

#[test]
fn retries_1618_with_backoff() {
    let (r, events) = run(
        &POLICY,
        Script {
            busy: Vec::new(),
            attempts: vec![
                Ok(Attempt::Busy1618),
                Ok(Attempt::Busy1618),
                Ok(Attempt::Done),
            ],
        },
    );
    assert_eq!(r, Ok(()));
    assert_eq!(
        sleeps(&events),
        [Duration::from_secs(5), Duration::from_secs(15)]
    );
    assert!(events.contains(&Event::Status(Status::Retrying { in_secs: 5 })));
    assert!(events.contains(&Event::Status(Status::Retrying { in_secs: 15 })));
    assert!(events.contains(&Event::Status(Status::Running { attempt: 3 })));
}

#[test]
fn gives_up_after_three_1618() {
    let (r, events) = run(
        &POLICY,
        Script {
            busy: Vec::new(),
            attempts: vec![
                Ok(Attempt::Busy1618),
                Ok(Attempt::Busy1618),
                Ok(Attempt::Busy1618),
                Ok(Attempt::Busy1618),
            ],
        },
    );
    assert_eq!(r, Err(MsiQueueError::InstallerBusy));
    assert_eq!(
        sleeps(&events),
        [
            Duration::from_secs(5),
            Duration::from_secs(15),
            Duration::from_secs(45)
        ]
    );
    assert_eq!(events.iter().filter(|e| **e == Event::Launch).count(), 4);
}

#[test]
fn failed_code_is_error() {
    let (r, events) = run(
        &POLICY,
        Script {
            busy: Vec::new(),
            attempts: vec![Ok(Attempt::Failed(1603)), Ok(Attempt::Done)],
        },
    );
    assert_eq!(r, Err(MsiQueueError::Failed(1603)));
    assert_eq!(events.iter().filter(|e| **e == Event::Launch).count(), 1);
    assert!(sleeps(&events).is_empty());
    assert_eq!(
        MsiQueueError::Failed(1603).to_string(),
        "o msiexec terminou com código 1603"
    );
}

#[test]
fn launch_io_error_is_error() {
    let (r, _) = run(
        &POLICY,
        Script {
            busy: Vec::new(),
            attempts: vec![Err(io::Error::other("sem msiexec"))],
        },
    );
    assert!(
        matches!(r, Err(MsiQueueError::Io(ref m)) if m.contains("sem msiexec")),
        "{r:?}"
    );
}

#[test]
fn exit_codes_map_to_attempts() {
    assert_eq!(attempt_from_code(0), Attempt::Done);
    assert_eq!(attempt_from_code(3010), Attempt::Done);
    assert_eq!(attempt_from_code(1618), Attempt::Busy1618);
    assert_eq!(attempt_from_code(1603), Attempt::Failed(1603));
    assert_eq!(attempt_from_code(-1), Attempt::Failed(-1));
}

#[test]
fn layout_paths() {
    let layout = Layout::new(r"C:\r");
    assert_eq!(layout.msi_lock(), std::path::Path::new(r"C:\r\msi.lock"));
    assert_eq!(
        layout.msiexec_log("gedit", 2),
        std::path::Path::new(r"C:\r\log\msiexec-gedit-2.log")
    );
}

#[test]
fn msi_lock_is_exclusive_across_threads() {
    let dir = TempDir::new().unwrap();
    let path = Layout::new(dir.path()).msi_lock();
    let (tx, rx) = channel::<&'static str>();
    let (waiting_tx, waiting_rx) = channel::<()>();
    std::thread::scope(|s| {
        let first = msi_queue::lock_msi(&path, &mut |_| {}).unwrap();
        tx.send("first in").unwrap();
        let second = {
            let tx = tx.clone();
            let path = &path;
            s.spawn(move || {
                let mut status = |st: Status| {
                    assert_eq!(st, Status::WaitingLock);
                    tx.send("second waiting").unwrap();
                    waiting_tx.send(()).unwrap();
                };
                let lock = msi_queue::lock_msi(path, &mut status).unwrap();
                tx.send("second in").unwrap();
                drop(lock);
            })
        };
        // The release happens only after the second thread reported it was waiting.
        let waited = waiting_rx.recv_timeout(HANG);
        tx.send("first out").unwrap();
        drop(first);
        if let Err(e) = second.join() {
            std::panic::resume_unwind(e);
        }
        waited.expect("second thread never reported WaitingLock");
    });
    drop(tx);
    let order: Vec<_> = rx.iter().collect();
    assert_eq!(
        order,
        ["first in", "second waiting", "first out", "second in"]
    );
}

#[cfg(windows)]
#[test]
#[ignore = "live: lê o mutex global do Windows Installer"]
fn installer_busy_live() {
    // Whatever the machine state, the probe answers without creating the mutex.
    let before = wingetrs::install::msiexec_win::installer_busy();
    let after = wingetrs::install::msiexec_win::installer_busy();
    eprintln!("installer_busy: {before} {after}");
}
