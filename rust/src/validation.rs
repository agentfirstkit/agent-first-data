pub fn parse_size(s: &str) -> Option<u64> {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let units = [
        ("KiB", 1024u64),
        ("MiB", 1024u64.pow(2)),
        ("GiB", 1024u64.pow(3)),
        ("TiB", 1024u64.pow(4)),
        ("kB", 1000u64),
        ("MB", 1000u64.pow(2)),
        ("GB", 1000u64.pow(3)),
        ("TB", 1000u64.pow(4)),
        ("B", 1u64),
    ];
    let (unit, mult) = units
        .iter()
        .find(|(unit, _mult)| s.strip_suffix(unit).is_some())?;
    let num_str = &s[..s.len() - unit.len()];
    if num_str.is_empty() || !is_decimal_number(num_str) {
        return None;
    }
    if let Ok(n) = num_str.parse::<u64>() {
        let result = n.checked_mul(*mult)?;
        return (result <= MAX_SAFE_INTEGER).then_some(result);
    }
    // Integer overflow must not silently fall back to float parsing.
    if !num_str.contains('.') && !num_str.contains('e') && !num_str.contains('E') {
        return None;
    }
    let f: f64 = num_str.parse().ok()?;
    if f < 0.0 || f.is_nan() || f.is_infinite() {
        return None;
    }
    let result = f * *mult as f64;
    if result > MAX_SAFE_INTEGER as f64 {
        return None;
    }
    Some(result as u64)
}

fn is_decimal_number(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut digits = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        i += 1;
        if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == bytes.len()
}

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
