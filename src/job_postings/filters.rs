use super::{
    normalization::{canonical_salary_interval, normalize_employment},
    ListingQuery,
};
use crate::errors::ApiError;
use sqlx::{Postgres, QueryBuilder};

fn is_india_search(term: &str) -> bool {
    ["india", "bharat"]
        .iter()
        .any(|alias| term.eq_ignore_ascii_case(alias))
}

fn like_pattern(term: &str) -> String {
    format!(
        "%{}%",
        term.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

pub(super) fn location_search_patterns(term: &str) -> Vec<String> {
    let mut terms = vec![like_pattern(term)];
    if is_india_search(term) {
        terms.extend(
            [
                "Bengaluru",
                "Bangalore",
                "Mumbai",
                "Delhi",
                "New Delhi",
                "Hyderabad",
                "Pune",
                "Chennai",
                "Kolkata",
                "Noida",
                "Gurugram",
                "Gurgaon",
                "Ahmedabad",
                "Jaipur",
            ]
            .into_iter()
            .map(like_pattern),
        );
    }
    terms
}

fn employment_key(value: &str) -> String {
    normalize_employment(Some(value.to_owned()))
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn append_canonical_employment_filter<'a>(query: &mut QueryBuilder<'a, Postgres>, value: &str) {
    query.push(" AND CASE ");
    for (canonical, aliases) in [
        (
            "fulltime",
            "'fulltime', 'fulltimeemployee', 'fulltimeemployment'",
        ),
        (
            "parttime",
            "'parttime', 'parttimeemployee', 'parttimeemployment'",
        ),
        ("internship", "'intern', 'internship'"),
        (
            "contract",
            "'contract', 'contractor', 'fixedterm', 'fixedtermcontract'",
        ),
        ("permanent", "'permanent'"),
        ("temporary", "'temporary', 'temp'"),
    ] {
        query
            .push(" WHEN ")
            .push("regexp_replace(lower(COALESCE(employment_type, '')), '[^a-z0-9]', '', 'g') IN (")
            .push(aliases)
            .push(") THEN '")
            .push(canonical)
            .push("'");
    }
    query
        .push(" ELSE regexp_replace(lower(COALESCE(employment_type, '')), '[^a-z0-9]', '', 'g') END = ")
        .push_bind(employment_key(value));
}

pub(super) fn append_listing_filters<'a>(
    query: &mut QueryBuilder<'a, Postgres>,
    filters: &'a ListingQuery,
) {
    query.push(" WHERE is_active = TRUE");
    if let Some(category) = filters.role_category.as_deref().filter(|v| !v.is_empty()) {
        if category == "software-engineering" {
            query.push(" AND is_software_engineering = TRUE");
        } else {
            query.push(" AND role_category = ").push_bind(category);
        }
    }
    if let Some(country) = filters.country.as_deref().filter(|v| !v.is_empty()) {
        query
            .push(" AND (country_codes @> ")
            .push_bind(vec![country.to_owned()]);
        if filters.include_global.unwrap_or(false) {
            query.push(" OR EXISTS (SELECT 1 FROM unnest(string_to_array(COALESCE(location, ''), ';')) AS location_part(value) WHERE LOWER(BTRIM(location_part.value)) IN ('remote', 'worldwide', 'anywhere', 'global'))");
        }
        query.push(")");
    }
    if let Some(value) = filters
        .role
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        // Normalize the multi-word alias before tokenizing so "front end" stays one concept.
        let normalized_role = value
            .to_ascii_lowercase()
            .replace("front-end", "frontend")
            .replace("front end", "frontend");
        for token in normalized_role.split_whitespace() {
            let normalized = token.trim_matches(|ch: char| {
                !ch.is_alphanumeric() && !['-', '%', '_', '\\'].contains(&ch)
            });
            if normalized.is_empty() {
                continue;
            }
            if matches!(
                normalized,
                "intern" | "interns" | "internship" | "internships"
            ) {
                // An explicit word family, not a prefix: "intern*" also matches
                // "internal", "international", and "internet".
                query
                    .push(" AND title ~* ")
                    .push_bind(r"\m(intern|interns|internship|internships)\M");
            } else if normalized == "frontend" {
                query.push(" AND (title ILIKE '%frontend%' OR title ILIKE '%front-end%' OR title ILIKE '%front end%')");
            } else {
                query
                    .push(" AND title ILIKE ")
                    .push_bind(like_pattern(normalized))
                    .push(" ESCAPE E'\\\\'");
            }
        }
    }
    if let Some(value) = filters
        .location
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let term = value.trim();
        let patterns = location_search_patterns(term);
        query.push(" AND (");
        for (index, pattern) in patterns.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("COALESCE(location, '') ILIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE E'\\\\'");
        }
        if filters.include_global.unwrap_or(false) {
            query.push(" OR EXISTS (SELECT 1 FROM unnest(string_to_array(COALESCE(location, ''), ';')) AS location_part(value) WHERE LOWER(BTRIM(location_part.value)) IN ('remote', 'worldwide', 'anywhere', 'global'))");
        }
        query.push(")");
    }
    if let Some(value) = filters
        .workplace
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if value.eq_ignore_ascii_case("unknown") {
            query.push(" AND (workplace_type IS NULL OR BTRIM(workplace_type) = '')");
        } else if filters.include_unknown_workplace.unwrap_or(false) {
            query
                .push(" AND (COALESCE(workplace_type, '') ILIKE ")
                .push_bind(like_pattern(value.trim()))
                .push(" ESCAPE E'\\\\' OR workplace_type IS NULL OR BTRIM(workplace_type) = '')");
        } else {
            query
                .push(" AND COALESCE(workplace_type, '') ILIKE ")
                .push_bind(like_pattern(value.trim()))
                .push(" ESCAPE E'\\\\'");
        }
    }
    if let Some(value) = filters
        .employment
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        append_canonical_employment_filter(query, value.trim());
    }
    if let Some(value) = filters
        .department
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let pattern = like_pattern(value.trim());
        query
            .push(" AND (COALESCE(department, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE E'\\\\' OR COALESCE(team, '') ILIKE ")
            .push_bind(pattern)
            .push(" ESCAPE E'\\\\')");
    }
    if let Some(value) = filters
        .posted_after
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        query
            .push(" AND LEFT(posted_at, 10) >= ")
            .push_bind(value.to_owned());
    }
    if let Some(value) = filters
        .posted_before
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        query
            .push(" AND LEFT(posted_at, 10) <= ")
            .push_bind(value.to_owned());
    }
    if filters.salary_min.is_some() || filters.salary_max.is_some() {
        if let (Some(currency), Some(interval)) = (
            filters.currency.as_deref(),
            filters.salary_interval.as_deref(),
        ) {
            query
                .push(" AND LOWER(BTRIM(COALESCE(salary_currency, ''))) = ")
                .push_bind(currency.trim().to_ascii_lowercase());
            query
                .push(" AND salary_interval = ")
                .push_bind(canonical_salary_interval(interval).unwrap_or_default());
        }
    } else if let Some(interval) = filters.salary_interval.as_deref() {
        query
            .push(" AND salary_interval = ")
            .push_bind(canonical_salary_interval(interval).unwrap_or_default());
    }
    if let Some(value) = filters.salary_min {
        query
            .push(" AND COALESCE(salary_max, salary_min) >= ")
            .push_bind(value);
    }
    if let Some(value) = filters.salary_max {
        query
            .push(" AND COALESCE(salary_min, salary_max) <= ")
            .push_bind(value);
    }
    if let Some(value) = filters
        .currency
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if filters.salary_min.is_some() || filters.salary_max.is_some() {
            // Numeric filters use the exact requested currency above.
        } else {
            query
                .push(" AND COALESCE(salary_currency, '') ILIKE ")
                .push_bind(like_pattern(value.trim()))
                .push(" ESCAPE E'\\\\'");
        }
    }
}

pub(super) fn validate_salary_filters(filters: &ListingQuery) -> Result<(), ApiError> {
    if filters.role_category.as_deref().is_some_and(|value| {
        ![
            "",
            "intern",
            "sde-1",
            "sde-2",
            "software-engineering",
            "other",
        ]
        .contains(&value)
    }) {
        return Err(ApiError::bad_request(
            "Unsupported role_category; use intern, sde-1, sde-2, software-engineering, or other.",
        ));
    }
    if filters
        .country
        .as_deref()
        .is_some_and(|value| !["", "US", "IN"].contains(&value))
    {
        return Err(ApiError::bad_request("Unsupported country; use US or IN."));
    }
    let has_numeric = filters.salary_min.is_some() || filters.salary_max.is_some();
    let currency = filters
        .currency
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let interval = filters
        .salary_interval
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    if has_numeric && (currency.is_none() || interval.is_none()) {
        return Err(ApiError::bad_request(
            "Numeric salary filters require both currency and salary_interval.",
        ));
    }
    if has_numeric && currency.is_some_and(|value| value.chars().any(char::is_whitespace)) {
        return Err(ApiError::bad_request(
            "Currency must be a single currency code.",
        ));
    }
    if let Some(interval) = interval {
        if canonical_salary_interval(interval).is_none() {
            return Err(ApiError::bad_request(
                "Unsupported salary_interval; use hour, day, week, month, or year.",
            ));
        }
    }
    if filters.salary_min.is_some_and(|value| !value.is_finite())
        || filters.salary_max.is_some_and(|value| !value.is_finite())
    {
        return Err(ApiError::bad_request(
            "Salary bounds must be finite numbers.",
        ));
    }
    Ok(())
}
