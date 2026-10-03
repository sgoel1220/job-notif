use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const REGISTRY_JSON: &str = include_str!("../companies.json");
pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const SUPPORTED_PROVIDERS: &[&str] = &[
    "greenhouse",
    "lever",
    "ashby",
    "smartrecruiters",
    "workable",
    "recruitee",
    "personio",
    "bamboohr",
    "workday",
    "remote-public",
    "atom-feed",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyRegistry {
    pub schema_version: u32,
    pub companies: Vec<CompanyEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyEntry {
    pub name: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryValidationError {
    UnsupportedSchemaVersion { version: u32 },
    EmptyCompanyName { index: usize },
    DuplicateCompany { name: String },
    UnsupportedProvider { company: String, provider: String },
    EnabledWithoutProvider { company: String },
    EnabledWithoutBoard { company: String },
    BoardWithoutProvider { company: String },
    ProviderWithoutBoard { company: String, provider: String },
}

pub fn load_registry() -> Result<CompanyRegistry, serde_json::Error> {
    CompanyRegistry::from_json(REGISTRY_JSON)
}

pub fn is_supported_provider(provider: &str) -> bool {
    SUPPORTED_PROVIDERS.contains(&provider)
}

impl CompanyRegistry {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn validate(&self) -> Result<(), Vec<RegistryValidationError>> {
        let mut errors = Vec::new();

        if self.schema_version != CURRENT_SCHEMA_VERSION {
            errors.push(RegistryValidationError::UnsupportedSchemaVersion {
                version: self.schema_version,
            });
        }

        let mut seen = HashSet::new();
        for (index, company) in self.companies.iter().enumerate() {
            let trimmed_name = company.name.trim();
            if trimmed_name.is_empty() {
                errors.push(RegistryValidationError::EmptyCompanyName { index });
                continue;
            }

            let normalized_name = trimmed_name.to_ascii_lowercase();
            if !seen.insert(normalized_name) {
                errors.push(RegistryValidationError::DuplicateCompany {
                    name: company.name.clone(),
                });
            }

            let provider = company
                .provider
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            let board = company
                .board
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());

            if let Some(provider) = provider {
                if !is_supported_provider(provider) {
                    errors.push(RegistryValidationError::UnsupportedProvider {
                        company: company.name.clone(),
                        provider: provider.to_owned(),
                    });
                }

                if board.is_none() {
                    errors.push(RegistryValidationError::ProviderWithoutBoard {
                        company: company.name.clone(),
                        provider: provider.to_owned(),
                    });
                }
            }

            if provider.is_none() && board.is_some() {
                errors.push(RegistryValidationError::BoardWithoutProvider {
                    company: company.name.clone(),
                });
            }

            if company.enabled && provider.is_none() {
                errors.push(RegistryValidationError::EnabledWithoutProvider {
                    company: company.name.clone(),
                });
            }

            if company.enabled && board.is_none() {
                errors.push(RegistryValidationError::EnabledWithoutBoard {
                    company: company.name.clone(),
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn enabled_companies(&self) -> impl Iterator<Item = &CompanyEntry> {
        self.companies.iter().filter(|company| company.enabled)
    }
}

impl CompanyEntry {
    pub fn is_resolved(&self) -> bool {
        self.provider.is_some() && self.board.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_registry() -> CompanyRegistry {
        CompanyRegistry {
            schema_version: CURRENT_SCHEMA_VERSION,
            companies: vec![CompanyEntry {
                name: "Example".into(),
                enabled: true,
                provider: Some("ashby".into()),
                board: Some("example".into()),
                source: Some("test".into()),
                notes: None,
            }],
        }
    }

    #[test]
    fn bundled_registry_is_valid() {
        let registry = load_registry().expect("registry JSON parses");
        registry.validate().expect("registry validates");
        assert!(registry.enabled_companies().count() >= 17);
        let remote = registry
            .enabled_companies()
            .find(|company| company.name == "Remote")
            .expect("verified official Remote feed is enabled");
        assert_eq!(remote.provider.as_deref(), Some("remote-public"));
        assert_eq!(remote.board.as_deref(), Some("remote"));
    }

    #[test]
    fn detects_duplicate_company_names_case_insensitively() {
        let mut registry = valid_registry();
        registry.companies.push(CompanyEntry {
            name: "example".into(),
            enabled: false,
            provider: None,
            board: None,
            source: None,
            notes: None,
        });

        let errors = registry.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|error| matches!(error, RegistryValidationError::DuplicateCompany { name } if name == "example")));
    }

    #[test]
    fn supports_all_known_provider_adapters() {
        for provider in [
            "greenhouse",
            "lever",
            "ashby",
            "smartrecruiters",
            "workable",
            "recruitee",
            "personio",
            "bamboohr",
            "workday",
            "remote-public",
            "atom-feed",
        ] {
            let mut registry = valid_registry();
            registry.companies[0].provider = Some(provider.into());
            registry
                .validate()
                .unwrap_or_else(|errors| panic!("{provider} should validate: {errors:?}"));
            assert!(is_supported_provider(provider));
        }
    }

    #[test]
    fn detects_unsupported_provider() {
        let mut registry = valid_registry();
        registry.companies[0].provider = Some("made-up-ats".into());

        let errors = registry.validate().unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            RegistryValidationError::UnsupportedProvider { provider, .. } if provider == "made-up-ats"
        )));
    }

    #[test]
    fn enabled_entries_require_provider_and_board() {
        let registry = CompanyRegistry {
            schema_version: CURRENT_SCHEMA_VERSION,
            companies: vec![CompanyEntry {
                name: "Missing".into(),
                enabled: true,
                provider: None,
                board: None,
                source: None,
                notes: None,
            }],
        };

        let errors = registry.validate().unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            RegistryValidationError::EnabledWithoutProvider { company } if company == "Missing"
        )));
    }

    #[test]
    fn provider_entries_require_non_empty_board() {
        let mut registry = valid_registry();
        registry.companies[0].board = Some("".into());

        let errors = registry.validate().unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            RegistryValidationError::ProviderWithoutBoard { company, provider }
                if company == "Example" && provider == "ashby"
        )));
    }
}
