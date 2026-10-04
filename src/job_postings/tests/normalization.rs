use super::*;

#[test]
fn maps_ashby_compensation_only_from_salary_component_and_keeps_location_details() {
    let raw = serde_json::json!({
        "employmentType": "FullTime",
        "address": {"postalAddress": {"addressLocality": "Bengaluru", "addressCountry": "India"}},
        "secondaryLocations": [{"location": "Mumbai", "address": {"addressCountry": "India"}}],
        "compensation": {"summaryComponents": [
            {"compensationType": "EquityPercentage", "minValue": 1.0, "maxValue": 2.0},
            {"compensationType": "Salary", "interval": "1 YEAR", "currencyCode": "INR", "minValue": 1200000, "maxValue": 1800000}
        ]}
    });
    let job = JobPosting::basic(
        "ashby",
        "1",
        "Engineer".into(),
        "Example",
        Some("Remote".into()),
        None,
        "https://example.test/job".into(),
        &raw,
    );
    assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
    assert_eq!(job.salary_min, Some(1_200_000.0));
    assert_eq!(job.salary_max, Some(1_800_000.0));
    assert_eq!(job.salary_currency.as_deref(), Some("INR"));
    assert_eq!(job.salary_interval.as_deref(), Some("year"));
    let api_job = serde_json::to_value(&job).unwrap();
    assert_eq!(api_job["salary_interval"], "year");
    let location = job.location.unwrap();
    assert!(
        location.contains("Remote")
            && location.contains("Bengaluru")
            && location.contains("India")
            && location.contains("Mumbai")
    );
}

#[test]
fn maps_real_ats_date_salary_locations_and_categories() {
    let raw = serde_json::json!({
        "id": "native-42",
        "createdAt": 1751559111614i64,
        "salaryRange": {"min": 120000, "max": 160000, "currency": "USD", "interval": "year"},
        "workplaceType": "Hybrid",
        "secondaryLocations": [{"location": "Dublin"}],
        "categories": {"location": "Remote - UK", "allLocations": ["Remote - UK", "Dublin"], "commitment": "Full-time", "department": "Engineering", "team": "Platform"},
        "descriptionBody": "<p>Build systems</p>",
        "descriptionBodyPlain": "Build systems plainly"
    });
    let job = JobPosting::basic(
        "lever",
        "url-id",
        "Engineer".into(),
        "Example",
        None,
        None,
        "https://example.test/jobs/42".into(),
        &raw,
    );
    assert_eq!(job.posted_at.as_deref(), Some("2025-07-03T16:11:51.614Z"));
    assert_eq!(job.salary_min, Some(120000.0));
    assert_eq!(job.salary_max, Some(160000.0));
    assert_eq!(job.salary_currency.as_deref(), Some("USD"));
    assert_eq!(job.workplace_type.as_deref(), Some("hybrid"));
    assert_eq!(job.employment_type.as_deref(), Some("Full-time"));
    assert_eq!(job.department.as_deref(), Some("Engineering"));
    assert_eq!(job.team.as_deref(), Some("Platform"));
    assert_eq!(job.location.as_deref(), Some("Remote - UK; Dublin"));
    assert_eq!(
        job.description_text.as_deref(),
        Some("Build systems plainly")
    );
}

#[test]
fn maps_greenhouse_first_published_and_office_metadata() {
    let raw = serde_json::json!({
        "first_published": "2026-10-02T11:31:50-04:00",
        "updated_at": "2026-10-02T12:00:00-04:00",
        "application_deadline": "2026-11-01",
        "departments": [{"name": "Product Engineering"}],
        "offices": [{"name": "Dublin", "location": "Ireland"}],
        "content": "<p>Role details</p>"
    });
    let job = JobPosting::basic(
        "greenhouse",
        "1",
        "Engineer".into(),
        "Example",
        None,
        None,
        "https://example.test/jobs/1".into(),
        &raw,
    );
    assert_eq!(job.posted_at.as_deref(), Some("2026-10-02T11:31:50-04:00"));
    assert_eq!(job.department.as_deref(), Some("Product Engineering"));
    assert_eq!(job.location.as_deref(), Some("Dublin, Ireland"));
    let mut adapter_override = job.clone();
    adapter_override.location = Some("Dublin".into());
    assert_eq!(
        location_for_storage(&adapter_override).as_deref(),
        Some("Dublin, Ireland")
    );
    assert_eq!(job.description.as_deref(), Some("<p>Role details</p>"));
}

#[test]
fn preserves_raw_fields_and_normalizes_filterable_metadata() {
    let raw = serde_json::json!({
        "title": "Engineer",
        "publishedAt": "2026-10-01T09:30:00Z",
        "salary_min": "120000",
        "salary_max": 160000,
        "salaryCurrency": "USD",
        "workAuthorization": ["US", "Canada"],
        "customProviderField": {"keep": true}
    });
    let job = JobPosting::basic(
        "test",
        "42",
        "Engineer".into(),
        "Example",
        None,
        None,
        "https://example.test/jobs/42".into(),
        &raw,
    );
    assert_eq!(job.posted_at.as_deref(), Some("2026-10-01T09:30:00Z"));
    assert_eq!(job.salary_min, Some(120000.0));
    assert_eq!(job.salary_max, Some(160000.0));
    assert_eq!(job.salary_currency.as_deref(), Some("USD"));
    let details: serde_json::Value = serde_json::from_str(&job.details_json).unwrap();
    assert_eq!(details["workAuthorization"][0], "US");
    assert_eq!(details["customProviderField"]["keep"], true);
}

#[test]
fn hybrid_labels_do_not_become_fully_remote() {
    for label in ["Hybrid / Remote", "Remote days (hybrid)"] {
        let posting = JobPosting::basic(
            "fixture",
            "hybrid",
            "Engineer".into(),
            "Example",
            None,
            None,
            "https://example.test/job".into(),
            serde_json::json!({"workplaceType": label, "isRemote": true}),
        );
        assert_eq!(posting.workplace_type.as_deref(), Some("hybrid"));
    }
}

#[test]
fn invalid_raw_metadata_does_not_erase_normalized_location() {
    let mut posting = JobPosting::basic(
        "fixture",
        "location",
        "Engineer".into(),
        "Example",
        Some("Bengaluru, India".into()),
        None,
        "https://example.test/job".into(),
        serde_json::json!({}),
    );
    posting.details_json = "invalid JSON".into();
    assert_eq!(
        location_for_storage(&posting).as_deref(),
        Some("Bengaluru, India")
    );
}

#[test]
fn html_only_descriptions_get_plaintext_with_entities_and_boundaries() {
    let html = "<p>R&amp;D &lt;team&gt;</p><ul><li>Build systems</li><li>Ship safely</li></ul>";
    let raw = serde_json::json!({"description": html});
    let job = JobPosting::basic(
        "workday",
        "1",
        "Engineer".into(),
        "Example",
        None,
        None,
        "https://example.test/job".into(),
        &raw,
    );
    assert_eq!(job.description.as_deref(), Some(html));
    let text = job.description_text.unwrap();
    assert!(text.contains("R&D <team>"), "{text}");
    assert!(text.contains("Build systems") && text.contains("Ship safely"));
    assert!(!text.contains("<p>") && !text.contains("&amp;"));
}

#[test]
fn provider_plaintext_is_preferred_over_html_fallback() {
    let raw =
        serde_json::json!({"description": "<p>HTML</p>", "descriptionPlain": "Provider plaintext"});
    let job = JobPosting::basic(
        "smartrecruiters",
        "1",
        "Engineer".into(),
        "Example",
        None,
        None,
        "https://example.test/job".into(),
        &raw,
    );
    assert_eq!(job.description_text.as_deref(), Some("Provider plaintext"));
}
