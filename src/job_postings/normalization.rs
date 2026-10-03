use super::{locations::merge_locations, JobPosting};
use serde::Serialize;

impl JobPosting {
    // Preserve the provider-facing constructor while normalization stays encapsulated.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn basic(
        source: &str,
        id: impl ToString,
        title: String,
        company: &str,
        location: Option<String>,
        workplace_type: Option<String>,
        url: String,
        details: impl Serialize,
    ) -> Self {
        let details_value = serde_json::to_value(&details).unwrap_or(serde_json::Value::Null);
        let details_json = serde_json::to_string(&details_value).unwrap_or_else(|_| "{}".into());
        let mut job = Self {
            source: source.into(),
            source_job_id: id.to_string(),
            title,
            company: company.into(),
            location,
            workplace_type,
            employment_type: None,
            department: None,
            team: None,
            description: None,
            description_text: None,
            posted_at: None,
            salary_min: None,
            salary_max: None,
            salary_currency: None,
            salary_interval: None,
            url,
            details_json,
        };
        // Normalize common ATS field names for filtering while retaining the complete original
        // object in details_json, including provider-specific and currently unknown fields.
        job.posted_at = first_date(
            &details_value,
            &[
                "postedAt",
                "posted_at",
                "publishedAt",
                "published_at",
                "first_published",
                "datePosted",
                "date_posted",
                "createdAt",
                "created_at",
            ],
        )
        .or(job.posted_at);
        job.employment_type = normalize_employment(
            first_string(
                &details_value,
                &["employmentType", "employment_type", "employmentStatus"],
            )
            .or_else(|| nested_string(&details_value, &["categories", "commitment"])),
        );
        job.department = first_string(&details_value, &["department", "departmentName"])
            .or_else(|| nested_string(&details_value, &["categories", "department"]))
            .or_else(|| named_array_values(&details_value, "departments"));
        job.team = first_string(&details_value, &["team", "teamName"])
            .or_else(|| nested_string(&details_value, &["categories", "team"]));
        job.salary_min = first_number(
            &details_value,
            &["salaryMin", "salary_min", "minSalary", "min_salary"],
        )
        .or_else(|| nested_number(&details_value, &["salaryRange", "min"]));
        job.salary_max = first_number(
            &details_value,
            &["salaryMax", "salary_max", "maxSalary", "max_salary"],
        )
        .or_else(|| nested_number(&details_value, &["salaryRange", "max"]));
        job.salary_currency = first_string(
            &details_value,
            &["salaryCurrency", "salary_currency", "currency"],
        )
        .or_else(|| nested_string(&details_value, &["salaryRange", "currency"]));
        job.salary_interval = first_string(
            &details_value,
            &[
                "salaryInterval",
                "salary_interval",
                "payPeriod",
                "salaryPeriod",
            ],
        )
        .or_else(|| nested_string(&details_value, &["salaryRange", "interval"]))
        .as_deref()
        .and_then(canonical_salary_interval);
        if let Some((minimum, maximum, currency, interval)) = ashby_salary(&details_value) {
            job.salary_min = job.salary_min.or(minimum);
            job.salary_max = job.salary_max.or(maximum);
            job.salary_currency = job.salary_currency.or(currency);
            job.salary_interval = job.salary_interval.or(interval);
        }
        job.description = first_string(
            &details_value,
            &[
                "description",
                "descriptionHtml",
                "descriptionBody",
                "content",
            ],
        );
        job.description_text = first_string(
            &details_value,
            &[
                "descriptionPlain",
                "descriptionBodyPlain",
                "openingPlain",
                "additionalPlain",
            ],
        );
        job.location = merge_locations(job.location, &details_value);
        job.workplace_type = normalize_workplace(job.workplace_type, &details_value);
        job
    }
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(value_to_string))
}

pub(super) fn value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn first_date(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let date = first_string(value, keys)?;
    date.parse::<i64>()
        .ok()
        .and_then(unix_millis_to_iso)
        .or(Some(date))
}

fn unix_millis_to_iso(milliseconds: i64) -> Option<String> {
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

pub(super) fn nested_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    value_to_string(current)
}

fn nested_number(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.replace(',', "").parse().ok(),
        _ => None,
    }
}

fn named_array_values(value: &serde_json::Value, key: &str) -> Option<String> {
    let names: Vec<_> = value
        .get(key)?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("name").and_then(value_to_string))
        .collect();
    (!names.is_empty()).then(|| names.join("; "))
}

/// Ashby's public feed exposes structured salary values in the source's stated currency and
/// interval (for example USD / "1 YEAR"). We preserve the numeric amounts as supplied and do
/// not annualize, convert currencies, or treat equity/bonus components as salary.
pub(super) fn canonical_salary_interval(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let tokens: String = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    let digits = tokens.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0 && &tokens[..digits] != "1" {
        return None;
    }
    let unit = &tokens[digits..];
    match unit {
        "h" | "hr" | "hrs" | "hour" | "hours" | "hourly" | "perhour" => Some("hour".into()),
        "d" | "day" | "days" | "daily" | "perday" => Some("day".into()),
        "w" | "wk" | "wks" | "week" | "weeks" | "weekly" | "perweek" => Some("week".into()),
        "mo" | "mos" | "month" | "months" | "monthly" | "permonth" => Some("month".into()),
        "y" | "yr" | "yrs" | "year" | "years" | "yearly" | "annual" | "annually" | "peryear" => {
            Some("year".into())
        }
        _ => None,
    }
}

type SalaryRange = (Option<f64>, Option<f64>, Option<String>, Option<String>);

fn ashby_salary(value: &serde_json::Value) -> Option<SalaryRange> {
    let compensation = value.get("compensation")?;
    let components = compensation
        .get("summaryComponents")
        .and_then(serde_json::Value::as_array)
        .filter(|items| !items.is_empty())
        .or_else(|| {
            compensation
                .pointer("/compensationTiers/0/components")
                .and_then(serde_json::Value::as_array)
        })?;
    let salary = components.iter().find(|item| {
        item.get("compensationType")
            .and_then(value_to_string)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("salary"))
    })?;
    let minimum = first_number(salary, &["minValue"]);
    let maximum = first_number(salary, &["maxValue"]);
    (minimum.is_some() || maximum.is_some()).then(|| {
        (
            minimum,
            maximum,
            first_string(salary, &["currencyCode"]),
            first_string(salary, &["interval"])
                .as_deref()
                .and_then(canonical_salary_interval),
        )
    })
}

pub(super) fn normalize_employment(value: Option<String>) -> Option<String> {
    let value = value?;
    let normalized = value.to_ascii_lowercase();
    let compact: String = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    let canonical = match compact.as_str() {
        "fulltime" | "fulltimeemployee" | "fulltimeemployment" => "Full-time",
        "parttime" | "parttimeemployee" | "parttimeemployment" => "Part-time",
        "intern" | "internship" => "Internship",
        "contractor" | "contract" | "fixedterm" | "fixedtermcontract" => "Contract",
        "permanent" => "Permanent",
        "temporary" | "temp" => "Temporary",
        _ => return Some(value),
    };
    Some(canonical.to_owned())
}

fn normalize_workplace(current: Option<String>, value: &serde_json::Value) -> Option<String> {
    let label = first_string(value, &["workplaceType", "workplace_type", "workplace"])
        .or(current)
        .or_else(|| {
            value
                .get("isRemote")
                .or_else(|| value.get("is_remote"))
                .and_then(serde_json::Value::as_bool)
                .filter(|remote| *remote)
                .map(|_| "remote".to_owned())
        })?;
    let lower = label.to_lowercase();
    // Explicit hybrid arrangements must not become fully remote just because
    // the provider also mentions remote days in the same label.
    if lower.contains("hybrid") {
        Some("hybrid".into())
    } else if lower.contains("remote") {
        Some("remote".into())
    } else if ["on-site", "onsite", "in-office", "office"]
        .iter()
        .any(|word| lower.contains(word))
    {
        Some("onsite".into())
    } else {
        Some(label)
    }
}

fn first_number(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| match value {
            serde_json::Value::Number(number) => number.as_f64(),
            serde_json::Value::String(text) => text.replace(',', "").parse().ok(),
            _ => None,
        })
    })
}
