/// Normalize a fixed UTC offset string to AFDATA canonical form.
///
/// Returns `"UTC"` for zero offset. Non-zero offsets return `+HH:MM` or
/// `-HH:MM`. This helper handles fixed offsets only; IANA timezone names and
/// DST rules are intentionally out of scope.
pub fn normalize_utc_offset(s: &str) -> Option<String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("utc") || s.eq_ignore_ascii_case("z") {
        return Some("UTC".to_string());
    }
    let sign = match s.as_bytes().first()? {
        b'+' => '+',
        b'-' => '-',
        _ => return None,
    };
    let body = &s[1..];
    let (hours, minutes) = parse_utc_offset_body(body)?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    if hours == 0 && minutes == 0 {
        return Some("UTC".to_string());
    }
    Some(format!("{sign}{hours:02}:{minutes:02}"))
}

/// Return true when `s` is an RFC 3339 `full-date` (`YYYY-MM-DD`).
pub fn is_valid_rfc3339_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let Some(year) = parse_ascii_u16_bytes(&bytes[0..4]) else {
        return false;
    };
    let Some(month) = parse_ascii_u8_bytes(&bytes[5..7]) else {
        return false;
    };
    let Some(day) = parse_ascii_u8_bytes(&bytes[8..10]) else {
        return false;
    };
    (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)
}

/// Return true when `s` is an RFC 3339 `partial-time` (`HH:MM:SS[.fraction]`).
///
/// AFDATA intentionally rejects `Z`/offset suffixes here: time-only fields are
/// not instants and cannot be resolved through timezone rules without a date.
pub fn is_valid_rfc3339_time(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 8 || bytes[2] != b':' || bytes[5] != b':' {
        return false;
    }
    let Some(hour) = parse_ascii_u8_bytes(&bytes[0..2]) else {
        return false;
    };
    let Some(minute) = parse_ascii_u8_bytes(&bytes[3..5]) else {
        return false;
    };
    let Some(second) = parse_ascii_u8_bytes(&bytes[6..8]) else {
        return false;
    };
    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    if bytes.len() == 8 {
        return true;
    }
    bytes[8] == b'.' && bytes.len() > 9 && bytes[9..].iter().all(u8::is_ascii_digit)
}

/// Return true when `s` is a complete RFC 3339 `date-time`, such as
/// `2026-02-14T10:30:00Z` or `2026-02-14T10:30:00.5+08:00`.
///
/// Composed from [`is_valid_rfc3339_date`] and [`is_valid_rfc3339_time`]: a
/// `full-date`, a `T`/`t` separator, a `partial-time` (with optional fractional
/// seconds), and a **mandatory** `time-offset` — either `Z`/`z` or `±HH:MM` with
/// `HH` in `00..23` and `MM` in `00..59`. The offset is required, so a bare
/// `2026-02-14T10:30:00` is rejected; a space separator is rejected (only `T`/`t`
/// is accepted); and a leap second (`:60`) is rejected, matching
/// [`is_valid_rfc3339_time`]. Non-ASCII input is rejected.
pub fn is_valid_rfc3339(s: &str) -> bool {
    if !s.is_ascii() || s.len() < 20 {
        return false;
    }
    if !is_valid_rfc3339_date(&s[0..10]) {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[10] != b'T' && bytes[10] != b't' {
        return false;
    }
    let rest = &s[11..];
    let last = rest.as_bytes()[rest.len() - 1];
    let partial = if last == b'Z' || last == b'z' {
        &rest[..rest.len() - 1]
    } else {
        if rest.len() < 6 {
            return false;
        }
        if !is_rfc3339_numoffset(&rest[rest.len() - 6..]) {
            return false;
        }
        &rest[..rest.len() - 6]
    };
    is_valid_rfc3339_time(partial)
}

/// Return true when `o` is an RFC 3339 `time-numoffset` (`±HH:MM`).
fn is_rfc3339_numoffset(o: &str) -> bool {
    let b = o.as_bytes();
    if b.len() != 6 || (b[0] != b'+' && b[0] != b'-') || b[3] != b':' {
        return false;
    }
    let (Some(hours), Some(minutes)) = (
        parse_ascii_u8_bytes(&b[1..3]),
        parse_ascii_u8_bytes(&b[4..6]),
    ) else {
        return false;
    };
    hours <= 23 && minutes <= 59
}

/// Return true when `s` is a structurally well-formed BCP 47 (RFC 5646) language tag.
///
/// This is a grammar-level check, not a registry lookup. It accepts hyphen-separated
/// ASCII-alphanumeric subtags (each 1-8 characters) whose primary subtag is a 2-3
/// letter language code, or the `x`/`i` privateuse/grandfathered lead. It rejects the
/// common mistakes: the POSIX-locale underscore form (`zh_CN`), empty or misplaced
/// hyphens, non-ASCII, and out-of-range primaries such as `chinese`. It does not check
/// that subtags are registered with IANA; a tool needing that guarantee validates further.
pub fn is_valid_bcp47(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    for (index, subtag) in s.split('-').enumerate() {
        let len = subtag.len();
        if len == 0 || len > 8 || !subtag.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return false;
        }
        if index == 0 {
            let is_language =
                (2..=3).contains(&len) && subtag.bytes().all(|b| b.is_ascii_alphabetic());
            let is_special = subtag == "x" || subtag == "i";
            if !is_language && !is_special {
                return false;
            }
        }
    }
    true
}

fn parse_utc_offset_body(body: &str) -> Option<(u8, u8)> {
    if body.is_empty() {
        return None;
    }
    if let Some((hours, minutes)) = body.split_once(':') {
        if hours.is_empty() || hours.len() > 2 || minutes.len() != 2 {
            return None;
        }
        return Some((parse_ascii_u8(hours)?, parse_ascii_u8(minutes)?));
    }
    if !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match body.len() {
        1 | 2 => Some((parse_ascii_u8(body)?, 0)),
        4 => Some((parse_ascii_u8(&body[..2])?, parse_ascii_u8(&body[2..])?)),
        _ => None,
    }
}

fn parse_ascii_u8(s: &str) -> Option<u8> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn parse_ascii_u8_bytes(bytes: &[u8]) -> Option<u8> {
    let n = parse_ascii_u16_bytes(bytes)?;
    u8::try_from(n).ok()
}

fn parse_ascii_u16_bytes(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value = 0u16;
    for byte in bytes {
        value = value.checked_mul(10)?;
        value = value.checked_add(u16::from(byte - b'0'))?;
    }
    Some(value)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    let year = u32::from(year);
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
