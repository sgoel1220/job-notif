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

    let companies = registry.by_name();
    assert!(companies["Supabase"].is_resolved());
    assert_eq!(companies["Supabase"].provider.as_deref(), Some("ashby"));
    assert_eq!(companies["Supabase"].board.as_deref(), Some("supabase"));
    assert_eq!(companies["AG1"].provider.as_deref(), Some("greenhouse"));
    assert_eq!(companies["AG1"].board.as_deref(), Some("ag1"));
    assert_eq!(companies["Doist"].provider.as_deref(), Some("workable"));
    assert_eq!(companies["GrowthX AI"].board.as_deref(), Some("GrowthX AI"));
    assert_eq!(companies["Deel"].enabled, false);
    assert!(companies["Deel"].provider.is_none());
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
