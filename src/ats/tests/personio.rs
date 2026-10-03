use super::*;

#[test]
fn maps_personio_xml_fixture() {
    let xml = r#"
            <workzag-jobs>
              <position>
                <id>123</id>
                <name>R&amp;D Engineer</name>
                <department>Engineering</department>
                <employmentType>permanent</employmentType>
                <office>Berlin, Remote - EU</office>
                <createdAt>2026-10-01T00:00:00+00:00</createdAt>
              </position>
            </workzag-jobs>
        "#;
    let identity = identity(AtsProvider::Personio);
    let fetched = to_fetched_jobs(
        identity.clone(),
        map_personio_jobs(&identity, "fixture", xml).unwrap(),
    );
    let job = &fetched[0].posting;
    assert_eq!(job.source, "ats/personio/example");
    assert_eq!(job.source_job_id, "123");
    assert_eq!(job.title, "R&D Engineer");
    assert_eq!(job.location.as_deref(), Some("Berlin; Remote - EU"));
    assert_eq!(job.workplace_type.as_deref(), Some("remote"));
    assert_eq!(job.employment_type.as_deref(), Some("permanent"));
    assert_eq!(job.url, "https://example.jobs.personio.de/job/123");
}

#[test]
fn personio_validates_document_and_decodes_cdata() {
    let identity = identity(AtsProvider::Personio);
    let xml = r#"<workzag-jobs><position><id><![CDATA[a&b/42]]></id><name><![CDATA[R&D Engineer]]></name><office><![CDATA[Berlin, India]]></office></position></workzag-jobs>"#;
    let mapped = map_personio_jobs(&identity, "fixture", xml).unwrap();
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].id, "a&b/42");
    assert_eq!(mapped[0].title, "R&D Engineer");
    assert_eq!(mapped[0].locations, ["Berlin", "India"]);
    let fetched = to_fetched_jobs(identity.clone(), mapped);
    assert_eq!(
        fetched[0].posting.url,
        "https://example.jobs.personio.de/job/a%26b%2F42"
    );
    assert!(map_personio_jobs(
        &identity,
        "fixture",
        "<workzag-jobs><position><id>42</id><name>Engineer</name>"
    )
    .is_err());
    assert!(map_personio_jobs(&identity, "fixture", "<wrong-root/>").is_err());
    assert!(map_personio_jobs(
        &identity,
        "fixture",
        "<workzag-jobs><position><id>42</id></position></workzag-jobs>"
    )
    .is_err());
    assert!(map_personio_jobs(&identity, "fixture", "<workzag-jobs/>")
        .unwrap()
        .is_empty());
    assert_eq!(
        map_personio_jobs(
            &identity,
            "fixture",
            "<workzag-jobs><position><id>7</id><name>A&amp;B</name></position></workzag-jobs>"
        )
        .unwrap()[0]
            .title,
        "A&B"
    );
    assert_eq!(
            map_personio_jobs(&identity, "fixture", "<workzag-jobs><position><id>7</id><name><![CDATA[A&amp;B]]></name></position></workzag-jobs>").unwrap()[0].title,
            "A&amp;B"
        );
    for malformed in [
        "<workzag-jobs><position><id>7</id><name>A &bogus; B</name></position></workzag-jobs>",
        "<workzag-jobs><position><id>7</id><name><b>Engineer</b></name></position></workzag-jobs>",
        "<workzag-jobs><position><id>7</id><name>Engineer</name></position></workzag-jobs>trailing",
        "<workzag-jobs/><workzag-jobs/>",
    ] {
        assert!(
            map_personio_jobs(&identity, "fixture", malformed).is_err(),
            "accepted {malformed}"
        );
    }
}
