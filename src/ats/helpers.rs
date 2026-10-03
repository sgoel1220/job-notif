use super::{AtsFetchError, AtsProvider, AtsSourceIdentity};
use serde_json::Value;
use std::future::Future;
use std::time::Duration;

pub(super) fn invalid_feed(
    identity: &AtsSourceIdentity,
    url: impl Into<String>,
    message: impl Into<String>,
) -> AtsFetchError {
    AtsFetchError::InvalidFeed {
        provider: identity.provider,
        company: identity.company_name.clone(),
        url: url.into(),
        message: message.into(),
    }
}

pub(super) fn required_string(
    identity: &AtsSourceIdentity,
    url: &str,
    value: &Value,
    path: &[&str],
    label: &str,
) -> Result<String, AtsFetchError> {
    string_at(value, path).ok_or_else(|| invalid_feed(identity, url, format!("missing {label}")))
}

pub(super) fn array_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Vec<Value>> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_array()
}

pub(super) fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    value_to_string(current)
}

pub(super) fn bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_bool()
}

pub(super) fn optional_usize(value: &Value, key: &str) -> Option<usize> {
    value.get(key).and_then(|value| match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|number| usize::try_from(number).ok()),
        Value::String(text) => text.parse().ok(),
        _ => None,
    })
}

pub(super) fn first_array_object_string(
    value: &Value,
    key: &str,
    object_key: &str,
) -> Option<String> {
    value
        .get(key)?
        .as_array()?
        .iter()
        .find_map(|item| string_at(item, &[object_key]))
}

pub(super) fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

pub(super) fn ashby_workplace_type(job: &Value, locations: &[String]) -> Option<String> {
    match string_at(job, &["workplaceType"])
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("hybrid") => Some("hybrid".to_owned()),
        Some("remote") => Some("remote".to_owned()),
        Some("onsite") | Some("on-site") | Some("on site") => Some("onsite".to_owned()),
        _ => workplace_type(locations, bool_at(job, &["isRemote"]), false),
    }
}

pub(super) fn workplace_type(
    locations: &[String],
    explicit_remote: Option<bool>,
    explicit_hybrid: bool,
) -> Option<String> {
    let location_text = locations.join(" ").to_ascii_lowercase();
    if explicit_hybrid || (explicit_remote.is_none() && location_text.contains("hybrid")) {
        Some("hybrid".to_owned())
    } else if explicit_remote == Some(true)
        || (explicit_remote.is_none() && location_text.contains("remote"))
    {
        Some("remote".to_owned())
    } else if explicit_remote == Some(false) {
        Some("onsite".to_owned())
    } else {
        None
    }
}

pub(super) fn join_locations(locations: &[String]) -> Option<String> {
    let mut values = locations
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    dedupe(&mut values);
    (!values.is_empty()).then(|| values.join("; "))
}

pub(super) fn dedupe(values: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::<String>::new();
    values.retain(|value| seen.insert(value.to_ascii_lowercase()));
}

pub(super) fn is_absolute_iso_date(value: &str) -> bool {
    let date = value.get(..10).unwrap_or("");
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
        || (value.len() > 10 && !matches!(value.as_bytes()[10], b'T' | b't' | b' '))
    {
        return false;
    }
    if value.len() > 10 {
        let tail = &value[11..];
        if tail.len() < 8
            || !tail.as_bytes()[0..2].iter().all(u8::is_ascii_digit)
            || tail.as_bytes()[2] != b':'
            || !tail.as_bytes()[3..5].iter().all(u8::is_ascii_digit)
            || tail.as_bytes()[5] != b':'
            || !tail.as_bytes()[6..8].iter().all(u8::is_ascii_digit)
        {
            return false;
        }
        let hour: u8 = tail[0..2].parse().unwrap_or(255);
        let minute: u8 = tail[3..5].parse().unwrap_or(255);
        let second: u8 = tail[6..8].parse().unwrap_or(255);
        if hour > 23 || minute > 59 || second > 60 {
            return false;
        }
        let suffix = &tail[8..];
        let suffix = if let Some(fraction) = suffix.strip_prefix('.') {
            let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return false;
            }
            &fraction[digits..]
        } else {
            suffix
        };
        if suffix != "Z" && suffix != "z" {
            if suffix.len() != 6
                || !matches!(suffix.as_bytes()[0], b'+' | b'-')
                || suffix.as_bytes()[3] != b':'
                || !suffix.as_bytes()[1..3].iter().all(u8::is_ascii_digit)
                || !suffix.as_bytes()[4..6].iter().all(u8::is_ascii_digit)
            {
                return false;
            }
            let tz_hour: u8 = suffix[1..3].parse().unwrap_or(255);
            let tz_minute: u8 = suffix[4..6].parse().unwrap_or(255);
            if tz_hour > 23 || tz_minute > 59 {
                return false;
            }
        }
    }
    let year: u32 = date[..4].parse().unwrap_or(0);
    let month: u32 = date[5..7].parse().unwrap_or(0);
    let day: u32 = date[8..10].parse().unwrap_or(0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    };
    day >= 1 && day <= days
}

pub(super) fn ensure_complete_snapshot(
    identity: &AtsSourceIdentity,
    url: &str,
    fetched: usize,
    total: usize,
) -> Result<(), AtsFetchError> {
    if fetched < total {
        Err(invalid_feed(
            identity,
            url,
            format!("incomplete snapshot: fetched {fetched} of {total}"),
        ))
    } else {
        Ok(())
    }
}

pub(super) async fn run_with_deadline<T, F>(
    timeout: Duration,
    provider: AtsProvider,
    company: String,
    future: F,
) -> Result<T, AtsFetchError>
where
    F: Future<Output = Result<T, AtsFetchError>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| AtsFetchError::Timeout {
            provider,
            company,
            timeout,
        })?
}

pub(super) fn next_page_limit(
    page_size: usize,
    current_jobs: usize,
    max_jobs: usize,
) -> Option<usize> {
    if current_jobs >= max_jobs || page_size == 0 || max_jobs == 0 {
        return None;
    }
    Some(page_size.min(max_jobs - current_jobs))
}

// XML text and CDATA decoding is handled event-by-event in parse_personio_xml.

pub(super) fn percent_encode_component(input: &str) -> String {
    let mut encoded = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub(super) fn unix_millis_to_iso(milliseconds: i64) -> Option<String> {
    let seconds = milliseconds.div_euclid(1_000);
    let millis = milliseconds.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}
