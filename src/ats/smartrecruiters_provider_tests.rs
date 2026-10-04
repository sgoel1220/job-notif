use super::*;
use crate::ats::AtsProvider;

#[test]
fn maps_named_section_description_in_semantic_order_and_plaintext() {
    let identity = AtsSourceIdentity {
        provider: AtsProvider::SmartRecruiters,
        company_id: None,
        company_name: "Example".into(),
        source_ref: "x".into(),
    };
    let data = json!({"content":[{"id":"1","name":"Engineer","detail":{"jobAd":{"sections":{
        "companyDescription":{"text":"<p>Company</p>"},
        "qualifications":{"text":"<p>Skills</p>"},
        "jobDescription":{"text":"<p>Build</p>"},
        "additionalInformation":{"text":"<p>Extra</p>"}
    }}}}]});
    let jobs = map_smartrecruiters_jobs(&identity, "test", &data, "Example").unwrap();
    assert_eq!(
        jobs[0].details["description"],
        "<p>Build</p>\n<p>Skills</p>\n<p>Extra</p>\n<p>Company</p>"
    );
    let fetched = super::super::to_fetched_jobs(identity, jobs);
    assert_eq!(
        fetched[0]
            .posting
            .description_text
            .as_ref()
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        "Build Skills Extra Company"
    );
}

#[test]
fn malformed_or_empty_details_preserve_listing_job() {
    let identity = AtsSourceIdentity {
        provider: AtsProvider::SmartRecruiters,
        company_id: None,
        company_name: "Example".into(),
        source_ref: "x".into(),
    };
    for detail in [
        json!({"jobAd":{"sections":[]}}),
        json!({"jobAd":{"sections":null}}),
    ] {
        let data = json!({"content":[{"id":"1","name":"Engineer","detail":detail}]});
        let jobs = map_smartrecruiters_jobs(&identity, "test", &data, "Example").unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].details.get("description").is_none());
    }
}
