//! Just enough time handling for "2d ago": RFC 3339 UTC to unix seconds.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` into unix seconds.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        o if o.len() == 6 => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            sign * (o[1..3].parse::<i64>().ok()? * 3600 + o[4..6].parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset)
}

// Howard Hinnant's days_from_civil.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// Howard Hinnant's civil_from_days: days since 1970-01-01 to (y, m, d).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn to_rfc3339(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

/// `40s`, `12m`, `5h`, `3d`, `6w`.
pub fn ago(then: i64, now: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        86_400..1_209_600 => format!("{}d", s / 86_400),
        _ => format!("{}w", s / 604_800),
    }
}

/// Seconds the local time zone is ahead of UTC at `epoch` (negative west of
/// it). The platform C runtime supplies the local civil time and daylight
/// saving rules. Returns 0 when the conversion is unavailable.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn local_offset(epoch: i64) -> i64 {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: std::ffi::c_long,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm = Tm {
        sec: 0,
        min: 0,
        hour: 0,
        mday: 0,
        mon: 0,
        year: 0,
        wday: 0,
        yday: 0,
        isdst: 0,
        gmtoff: 0,
        zone: std::ptr::null(),
    };
    // SAFETY: `Tm` is `struct tm` as glibc and macOS lay it out; `localtime_r`
    // writes only into it and reads `epoch`.
    let done = unsafe { localtime_r(&epoch, &mut tm) };
    if done.is_null() { 0 } else { tm.gmtoff as i64 }
}

#[cfg(windows)]
pub fn local_offset(epoch: i64) -> i64 {
    // MSVC's `struct tm` has nine int fields and no `tm_gmtoff`.
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
    }
    unsafe extern "C" {
        fn _localtime64_s(out: *mut Tm, t: *const i64) -> i32;
    }
    let mut tm = std::mem::MaybeUninit::<Tm>::uninit();
    // SAFETY: the MSVC runtime fills `tm` on success and only reads `epoch`.
    if unsafe { _localtime64_s(tm.as_mut_ptr(), &epoch) } != 0 {
        return 0;
    }
    // SAFETY: `_localtime64_s` returned success, so every `tm` field is set.
    let tm = unsafe { tm.assume_init() };
    days_from_civil(
        i64::from(tm.year) + 1900,
        i64::from(tm.mon) + 1,
        i64::from(tm.mday),
    ) * 86_400
        + i64::from(tm.hour) * 3600
        + i64::from(tm.min) * 60
        + i64::from(tm.sec)
        - epoch
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn local_offset(_: i64) -> i64 {
    0
}

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// An absolute time in the local time zone: `13:27` when it is today,
/// `Tue 6 Oct 13:27` otherwise, `6 Oct 2025 13:27` from another year. Never
/// UTC; relative times are [`ago`].
pub fn when(epoch: i64, now: i64) -> String {
    when_in(epoch, now, local_offset(epoch), local_offset(now))
}

/// [`when`] with the zone's offsets given (for `epoch` and for `now`).
pub fn when_in(epoch: i64, now: i64, offset: i64, now_offset: i64) -> String {
    let local = epoch + offset;
    let (days, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let clock = format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60);
    let today = (now + now_offset).div_euclid(86_400);
    if days == today {
        return clock;
    }
    let (y, m, d) = civil_from_days(days);
    let month = MONTHS[(m - 1) as usize];
    if y != civil_from_days(today).0 {
        return format!("{d} {month} {y} {clock}");
    }
    // 1970-01-01 was a Thursday.
    let day = DAYS[(days + 3).rem_euclid(7) as usize];
    format!("{day} {d} {month} {clock}")
}

pub fn ago_str(rfc3339: &str, now: i64) -> String {
    parse_rfc3339(rfc3339).map_or_else(|| "—".into(), |t| ago(t, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc_offsets_and_fractions() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-10-06T03:15:53Z"), Some(1_791_256_553));
        assert_eq!(
            parse_rfc3339("2026-10-06T05:15:53.123+02:00"),
            Some(1_791_256_553)
        );
        assert_eq!(parse_rfc3339("nope"), None);
    }

    #[test]
    fn writes_what_it_parses() {
        for s in [0, 1_791_256_553, 951_782_400, 4_107_542_399] {
            assert_eq!(parse_rfc3339(&to_rfc3339(s)), Some(s), "{s}");
        }
        assert_eq!(to_rfc3339(1_791_256_553), "2026-10-06T03:15:53Z");
    }

    #[test]
    fn when_is_the_clock_today_and_the_date_otherwise_in_the_given_zone() {
        let t = 1_791_256_553; // Tue 6 Oct 2026 03:15:53 UTC
        assert_eq!(when_in(t, t, 0, 0), "03:15");
        // Ten hours east, the same instant is 13:15 that day.
        assert_eq!(when_in(t, t, 36_000, 36_000), "13:15");
        // The day before, in the same year.
        assert_eq!(when_in(t - 86_400, t, 0, 0), "Mon 5 Oct 03:15");
        // West of UTC the local day is the 5th: today is the 5th too.
        assert_eq!(when_in(t, t, -4 * 3600, -4 * 3600), "23:15");
        assert_eq!(when_in(t - 400 * 86_400, t, 0, 0), "1 Sep 2025 03:15");
    }

    #[test]
    fn the_local_offset_is_whole_quarter_hours_in_a_real_zone() {
        let o = local_offset(1_791_256_553);
        assert!(o % 900 == 0 && o.abs() <= 14 * 3600, "{o}");
    }

    #[test]
    fn ago_picks_the_largest_whole_unit() {
        assert_eq!(ago(0, 59), "59s");
        assert_eq!(ago(0, 7200), "2h");
        assert_eq!(ago(0, 3 * 86_400), "3d");
        assert_eq!(ago(0, 21 * 86_400), "3w");
    }
}
