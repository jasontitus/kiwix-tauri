// SPDX-License-Identifier: GPL-3.0-or-later
//! Timestamps shared by saved reading data, diagnostics and network handoffs.
#![forbid(unsafe_code)]
use std::fmt::Write;
use time::{
    format_description::well_known::{Rfc2822, Rfc3339},
    OffsetDateTime,
};

pub fn now() -> String {
    format(OffsetDateTime::now_utc())
}

pub fn utc_seconds() -> String {
    let time = OffsetDateTime::now_utc();
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        time.date(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

pub fn format(time: OffsetDateTime) -> String {
    format_seconds(time, time.second())
}

// Saved timestamps use 0/3/6/9 fractional digits and an explicit signed offset.
fn format_seconds(time: OffsetDateTime, seconds: u8) -> String {
    let mut text = format!(
        "{}T{:02}:{:02}:{seconds:02}",
        time.date(),
        time.hour(),
        time.minute()
    );
    let nanos = time.nanosecond();
    if nanos != 0 {
        if nanos.is_multiple_of(1_000_000) {
            write!(text, ".{:03}", nanos / 1_000_000).expect("String writer");
        } else if nanos.is_multiple_of(1000) {
            write!(text, ".{:06}", nanos / 1000).expect("String writer");
        } else {
            write!(text, ".{nanos:09}").expect("String writer");
        }
    }
    let offset = time.offset().whole_minutes();
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.unsigned_abs();
    write!(text, "{sign}{:02}:{:02}", minutes / 60, minutes % 60).expect("String writer");
    text
}

fn components(value: &str) -> Result<(OffsetDateTime, bool), &'static str> {
    if !matches!(value.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
        return Err("Invalid timestamp separator");
    }
    if value.as_bytes().get(17..19) == Some(b"60") {
        // Keep legacy leap-second text and its fraction for diagnostic exports.
        // OffsetDateTime has no leap seconds. Parse its calendar minute using
        // the preceding second and carry the marker outside the numeric value.
        let mut preceding = value.to_owned();
        preceding.replace_range(17..19, "59");
        return OffsetDateTime::parse(&preceding, &Rfc3339)
            .map(|date| (date, true))
            .map_err(|_| "Invalid timestamp");
    }
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|date| (date, false))
        .map_err(|_| "Invalid timestamp")
}

pub fn parse(value: &str) -> Result<OffsetDateTime, &'static str> {
    components(value).map(|(date, _)| date)
}

pub fn normalized(value: &str) -> Option<String> {
    let (date, leap_second) = components(value).ok()?;
    Some(format_seconds(
        date,
        if leap_second { 60 } else { date.second() },
    ))
}

pub fn parse_mail_date(value: &str) -> Result<OffsetDateTime, &'static str> {
    let input = value.trim();
    let parsed = OffsetDateTime::parse(input, &Rfc2822)
        .or_else(|error| {
            // The header is retained verbatim. Only validate it here. The stored
            // format allows leap seconds which the numeric date may not represent.
            let Some(index) = input.find(":60") else {
                return Err(error);
            };
            // Match seconds only; do not repair an invalid minute or offset.
            if index < 3 || input.as_bytes()[index - 3] != b':' {
                return Err(error);
            }
            let mut preceding = input.to_owned();
            preceding.replace_range(index..index + 3, ":59");
            OffsetDateTime::parse(&preceding, &Rfc2822)
        })
        .map_err(|_| "Invalid mail date")?;
    // The parser ignores a supplied weekday. A contradictory date provides no
    // usable Last-Modified validator for a byte-range resume.
    if let Some((weekday, _)) = input.split_once(',') {
        let expected = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            [usize::from(parsed.weekday().number_days_from_monday())];
        if !weekday.eq_ignore_ascii_case(expected) {
            return Err("Invalid mail date weekday");
        }
    }
    if parsed.offset().whole_hours().unsigned_abs() >= 24 {
        return Err("Invalid mail date offset");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_timestamps_keep_calendar_offset_fraction_and_leap_second() {
        for (input, output) in [
            ("2026-01-31T23:59:59Z", "2026-01-31T23:59:59+00:00"),
            (
                "2026-01-31t23:59:59.1-00:30",
                "2026-01-31T23:59:59.100-00:30",
            ),
            (
                "2026-01-31 23:59:59.00001+01:15",
                "2026-01-31T23:59:59.000010+01:15",
            ),
            (
                "2016-12-31T23:59:60.1234567891z",
                "2016-12-31T23:59:60.123456789+00:00",
            ),
        ] {
            assert_eq!(normalized(input).as_deref(), Some(output));
        }
        for value in [
            "2026-02-30T11:22:33Z",
            "2026-02-01X11:22:33Z",
            "2026-01-31T23:59:59+25:00",
        ] {
            assert!(parse(value).is_err());
        }
        assert!(parse_mail_date("Sun, 06 Nov 1994 08:49:37 GMT").is_ok());
        assert!(parse_mail_date("Mon, 06 Nov 1994 08:49:37 GMT").is_err());
        assert!(parse_mail_date("Sat, 31 Dec 2016 23:59:60 GMT").is_ok());
        assert!(parse_mail_date("Sat, 31 Dec 2016 23:60:00 GMT").is_err());
    }
}
