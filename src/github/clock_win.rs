//! This machine's offset from UTC, for the hour the GitHub limit resets.
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};

/// Seconds since 1970-01-01 of a calendar time (days from the civil date, Howard Hinnant).
fn seconds(t: &SYSTEMTIME) -> i64 {
    let (y, m, d) = (i64::from(t.wYear), i64::from(t.wMonth), i64::from(t.wDay));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + i64::from(t.wHour) * 3600 + i64::from(t.wMinute) * 60 + i64::from(t.wSecond)
}

/// Local time minus UTC now, in seconds, rounded to a quarter hour (the two clocks are read
/// one after the other).
pub fn local_offset() -> Option<i64> {
    // SAFETY: both only fill and return a SYSTEMTIME.
    let (local, utc) = unsafe { (GetLocalTime(), GetSystemTime()) };
    if local.wYear == 0 || utc.wYear == 0 {
        return None;
    }
    let diff = seconds(&local) - seconds(&utc);
    Some((diff + 450).div_euclid(900) * 900)
}
