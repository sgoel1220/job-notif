#[path = "../src/company_registry.rs"]
mod company_registry;

use company_registry::{
    load_registry, CompanyEntry, CompanyRegistry, RegistryValidationError, CURRENT_SCHEMA_VERSION,
};

fn valid_registry() -> CompanyRegistry {
    CompanyRegistry {
        schema_version: CURRENT_SCHEMA_VERSION,
        companies: vec![CompanyEntry {
            name: "Example".into(),
            enabled: true,
            provider: Some("greenhouse".into()),
            board: Some("example".into()),
            source: Some("test".into()),
            notes: None,
        }],
    }
}

#[test]
fn bundled_registry_parses_and_validates() {
    let registry = load_registry().expect("companies.json parses");
    registry.validate().expect("companies.json validates");

    let company = |name: &str| {
        registry
            .companies
            .iter()
            .find(|item| item.name == name)
            .unwrap()
    };
    assert!(company("Supabase").is_resolved());
    assert_eq!(company("Supabase").provider.as_deref(), Some("ashby"));
    assert_eq!(company("Supabase").board.as_deref(), Some("supabase"));
    assert_eq!(company("AG1").provider.as_deref(), Some("greenhouse"));
    assert_eq!(company("AG1").board.as_deref(), Some("ag1"));
    assert_eq!(company("Doist").provider.as_deref(), Some("workable"));
    assert_eq!(company("GrowthX AI").board.as_deref(), Some("GrowthX AI"));
    // Unresolved companies may become enabled after evidence-backed research.
    assert!(registry.companies.iter().any(|entry| entry.name == "Deel"));
}

#[derive(serde::Deserialize)]
struct TrackedCompany {
    company_name: String,
    coverage_status: String,
    research_status: String,
    research_owner: String,
    research_run: String,
    last_checked: String,
    provider: String,
    board: String,
    root_cause: String,
    evidence_url: String,
    feed_evidence: String,
    next_action: String,
}

fn tracked_companies() -> Vec<TrackedCompany> {
    csv::Reader::from_reader(include_bytes!("../companies.csv").as_slice())
        .deserialize()
        .collect::<Result<Vec<_>, _>>()
        .expect("company tracking CSV parses, including quoted evidence and notes")
}

#[test]
fn every_csv_company_has_a_registry_entry() {
    let registry = load_registry().expect("companies.json parses");
    let mut seen = std::collections::HashSet::new();
    for row in tracked_companies() {
        let name = row.company_name.trim();
        assert!(!name.is_empty());
        assert!(
            seen.insert(name.to_ascii_lowercase()),
            "duplicate CSV company {name}"
        );
        let entry = registry
            .companies
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("CSV company {name} must remain represented"));
        assert_eq!(
            row.coverage_status,
            if entry.enabled {
                "enabled"
            } else {
                "unresolved"
            }
        );
        assert_eq!(row.provider, entry.provider.as_deref().unwrap_or_default());
        assert_eq!(row.board, entry.board.as_deref().unwrap_or_default());
    }
}

#[test]
fn research_tracking_has_exclusive_claims_and_actionable_outcomes() {
    let statuses = [
        "enabled_not_rechecked",
        "in_progress",
        "verified_live",
        "valid_empty",
        "unsupported_provider",
        "custom_careers",
        "inactive_or_acquired",
        "blocked",
        "identity_conflict",
        "unresolved",
    ];
    for row in tracked_companies() {
        assert!(
            statuses.contains(&row.research_status.as_str()),
            "{} has unknown status",
            row.company_name
        );
        assert!(
            !row.next_action.trim().is_empty(),
            "{} needs a next action",
            row.company_name
        );
        if row.research_status == "in_progress" {
            assert!(
                !row.research_owner.trim().is_empty(),
                "unowned research claim"
            );
            assert!(
                !row.research_run.trim().is_empty(),
                "claim missing run identity"
            );
        } else if row.research_status != "enabled_not_rechecked" {
            assert!(
                !row.last_checked.trim().is_empty(),
                "completed research missing check date"
            );
            assert!(
                !row.evidence_url.trim().is_empty()
                    || (row.research_status == "unresolved"
                        && !row.root_cause.trim().is_empty()
                        && !row.feed_evidence.trim().is_empty()),
                "completed research must cite evidence or explicitly record unavailable source"
            );
            if !matches!(
                row.research_status.as_str(),
                "verified_live" | "valid_empty"
            ) {
                assert!(
                    !row.root_cause.trim().is_empty(),
                    "unresolved research needs a concrete cause"
                );
            }
        }
    }
}

#[test]
fn enabled_boards_have_auditable_provenance() {
    let registry = load_registry().expect("companies.json parses");
    for entry in registry.enabled_companies() {
        assert!(
            entry
                .source
                .as_deref()
                .is_some_and(|source| !source.trim().is_empty()),
            "{} must record its source",
            entry.name
        );
        assert!(
            entry
                .notes
                .as_deref()
                .is_some_and(|notes| !notes.trim().is_empty()),
            "{} must record verification context",
            entry.name
        );
    }
}

#[test]
fn validates_duplicate_company() {
    let mut registry = valid_registry();
    registry.companies.push(CompanyEntry {
        name: "example".into(),
        enabled: false,
        provider: None,
        board: None,
        source: Some("test".into()),
        notes: None,
    });

    let errors = registry.validate().unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        RegistryValidationError::DuplicateCompany { name } if name == "example"
    )));
}

#[test]
fn validates_supported_provider() {
    let mut registry = valid_registry();
    registry.companies[0].provider = Some("unknown-provider".into());

    let errors = registry.validate().unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        RegistryValidationError::UnsupportedProvider { company, provider }
            if company == "Example" && provider == "unknown-provider"
    )));
}

#[test]
fn validates_enabled_entries_have_provider() {
    let registry = CompanyRegistry {
        schema_version: CURRENT_SCHEMA_VERSION,
        companies: vec![CompanyEntry {
            name: "Missing Provider".into(),
            enabled: true,
            provider: None,
            board: None,
            source: Some("test".into()),
            notes: None,
        }],
    };

    let errors = registry.validate().unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        RegistryValidationError::EnabledWithoutProvider { company }
            if company == "Missing Provider"
    )));
}

#[test]
fn validates_provider_entries_have_board() {
    let mut registry = valid_registry();
    registry.companies[0].board = None;

    let errors = registry.validate().unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        RegistryValidationError::ProviderWithoutBoard { company, provider }
            if company == "Example" && provider == "greenhouse"
    )));
}

#[test]
fn validates_board_is_not_set_without_provider() {
    let registry = CompanyRegistry {
        schema_version: CURRENT_SCHEMA_VERSION,
        companies: vec![CompanyEntry {
            name: "Board Only".into(),
            enabled: false,
            provider: None,
            board: Some("board-only".into()),
            source: Some("test".into()),
            notes: None,
        }],
    };

    let errors = registry.validate().unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        RegistryValidationError::BoardWithoutProvider { company } if company == "Board Only"
    )));
}
