//! Tests that need an unelevated process (Task 222). The GitHub runner is elevated without
//! UAC: there, and only there (`CI=true`), the tests named in `non_elevated.txt` end early;
//! `scripts/ci_non_elevated.ps1` then runs them for real as a standard user. Outside CI an
//! elevated run still fails them. `check_rules.ps1` refuses a call outside that list.

/// Whether a test that needs an unelevated process is skipped.
pub fn skip_decision(ci: bool, elevated: bool) -> bool {
    ci && elevated
}

/// Called first in each listed test: `true` ⇒ return at once (printed as `pulado: ...`).
#[cfg(windows)]
#[allow(clippy::expect_used)]
pub fn skip_if_elevated_in_ci(why: &str) -> bool {
    let ci = std::env::var("CI").is_ok_and(|v| v == "true");
    let elevated = wingetrs::winget_install::launch_win::is_elevated().expect("token read");
    let skip = skip_decision(ci, elevated);
    if skip {
        eprintln!("pulado: processo elevado na CI ({why})");
    }
    skip
}

#[test]
fn skip_helper_only_in_ci_and_elevated() {
    assert!(!skip_decision(false, true));
    assert!(!skip_decision(true, false));
    assert!(skip_decision(true, true));
    assert!(!skip_decision(false, false));
}
