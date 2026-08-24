use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::*;

fn source_paths(directory: &Path) -> SourcePaths {
    SourcePaths::new(
        SourceFile::new(directory.join("config"), PathOrigin::CommandLine),
        SourceFile::new(directory.join("credentials"), PathOrigin::CommandLine),
    )
}

fn write_sources(directory: &Path, config: &[u8], credentials: &[u8]) -> SourcePaths {
    fs::write(directory.join("config"), config).expect("write config fixture");
    fs::write(directory.join("credentials"), credentials).expect("write credentials fixture");
    source_paths(directory)
}

#[test]
fn merges_modern_legacy_and_credentials_only_profiles() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[default]
region = us-east-1

[profile modern]
sso_session = company
sso_account_id = 111122223333
sso_role_name = Developer

[sso-session company]
sso_start_url = https://example.awsapps.com/start
sso_region = ap-northeast-1
sso_registration_scopes = sso:account:access

[profile legacy]
sso_start_url = https://legacy.awsapps.com/start
sso_region = eu-west-1
sso_account_id = 444455556666
sso_role_name = ReadOnly

[services custom]
endpoint_url = https://example.invalid
"#,
        b"[default]\naws_access_key_id = ignored\naws_secret_access_key = ignored\n\n[credentials-only]\naws_access_key_id = ignored\naws_secret_access_key = ignored\n",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert_eq!(
        catalog.selectable_names().collect::<Vec<_>>(),
        ["credentials-only", "default", "legacy", "modern"]
    );
    let modern = catalog.get("modern").expect("modern profile");
    assert!(matches!(
        modern.sso,
        Some(SsoMetadata::Modern {
            ref session_name,
            ref start_url,
            ref sso_region,
            ref registration_scopes,
            ref account_id,
            ref role_name,
        }) if session_name == "company"
            && start_url.as_deref() == Some("https://example.awsapps.com/start")
            && sso_region.as_deref() == Some("ap-northeast-1")
            && registration_scopes.as_deref() == Some("sso:account:access")
            && account_id.as_deref() == Some("111122223333")
            && role_name.as_deref() == Some("Developer")
    ));
    assert!(matches!(
        catalog.get("legacy").and_then(|profile| profile.sso.as_ref()),
        Some(SsoMetadata::Legacy { account_id, .. })
            if account_id.as_deref() == Some("444455556666")
    ));
    assert!(catalog.get("credentials-only").is_some_and(|profile| {
        !profile.has_config_section
            && profile.has_credentials_section
            && profile.static_credentials.secret_access_key
    }));
    assert!(catalog.get("custom").is_none());
    assert!(catalog.issues().is_empty());
    for name in ["credentials-only", "default", "legacy", "modern"] {
        assert!(
            catalog.profile_is_activatable(name),
            "{name} should be ready"
        );
    }
}

#[test]
fn credential_and_process_values_never_enter_the_domain_or_diagnostics() {
    let directory = TempDir::new().expect("temp directory");
    let sentinel = "DO-NOT-RETAIN-secret-sentinel";
    let config = format!(
        "[profile process]\ncredential_process = /bin/helper --token {sentinel}\nweb_identity_token_file = /secret/{sentinel}\nmfa_serial = arn:{sentinel}\n"
    );
    let credentials = format!(
        "[static]\naws_access_key_id = {sentinel}\naws_secret_access_key = {sentinel}\naws_session_token = {sentinel}\n"
    );
    let sources = write_sources(directory.path(), config.as_bytes(), credentials.as_bytes());

    let catalog = Catalog::load(sources).expect("load catalog");
    let snapshot = format!("{catalog:#?}");

    assert!(!snapshot.contains(sentinel));
    let process = catalog.get("process").expect("process profile");
    assert!(process.has_credential_process);
    assert!(process.has_web_identity_token_file);
    assert!(process.has_mfa_serial);
    assert!(
        catalog
            .get("static")
            .is_some_and(|profile| profile.static_credentials.any())
    );
}

#[test]
fn malformed_and_unsafe_sections_are_quarantined_but_later_sections_recover() {
    let directory = TempDir::new().expect("temp directory");
    let config = b"[profile good]\nregion = us-east-1\nmalformed line\nrole_arn = arn:aws:iam::123456789012:role/Good\n[profile bad\x1bname]\nregion = should-not-escape\n[profile broken\nregion = should-not-leak\n[profile recovered]\nregion = ap-southeast-2\n";
    let sources = write_sources(directory.path(), config, b"");

    let catalog = Catalog::load(sources).expect("load catalog");

    assert_eq!(
        catalog.get("good").and_then(|p| p.region.as_deref()),
        Some("us-east-1")
    );
    assert!(!catalog.get("good").is_some_and(Profile::is_role));
    assert_eq!(
        catalog.get("recovered").and_then(|p| p.region.as_deref()),
        Some("ap-southeast-2")
    );
    assert_eq!(catalog.len(), 2);
    assert!(!catalog.profile_is_activatable("good"));
    assert!(catalog.profile_is_activatable("recovered"));
    assert!(
        catalog
            .issues()
            .iter()
            .any(|issue| issue.kind == CatalogIssueKind::MalformedProperty)
    );
    assert!(
        catalog
            .issues()
            .iter()
            .any(|issue| issue.kind == CatalogIssueKind::UnsafeProfileName)
    );
    assert!(
        catalog
            .issues()
            .iter()
            .any(|issue| issue.kind == CatalogIssueKind::MalformedSection)
    );
}

#[test]
fn unicode_line_and_paragraph_separators_are_unsafe_catalog_text() {
    let directory = TempDir::new().expect("temp directory");
    let config = "[profile line\u{2028}separator]\nregion=us-east-1\n\
[profile paragraph\u{2029}separator]\nregion=us-west-2\n\
[profile invalid-metadata]\nregion=us\u{2028}-east-1\n\
[profile recovered]\nregion=ap-northeast-1\n";
    let sources = write_sources(directory.path(), config.as_bytes(), b"");

    let catalog = Catalog::load(sources).expect("load catalog");

    assert!(catalog.get("line\u{2028}separator").is_none());
    assert!(catalog.get("paragraph\u{2029}separator").is_none());
    assert!(!catalog.profile_is_activatable("invalid-metadata"));
    assert_eq!(
        catalog
            .get("recovered")
            .and_then(|profile| profile.region.as_deref()),
        Some("ap-northeast-1")
    );
    assert_eq!(
        catalog
            .issues()
            .iter()
            .filter(|issue| issue.kind == CatalogIssueKind::UnsafeProfileName)
            .count(),
        2
    );
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidMetadata {
                field: MetadataField::Region,
            }
    }));
}

#[test]
fn duplicate_sections_and_metadata_are_ambiguous_and_first_value_is_retained() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile duplicated]\nregion = us-east-1\nregion = eu-west-1\n[profile duplicated]\nregion = eu-central-1\n",
        b"[duplicated]\naws_access_key_id=x\n[duplicated]\naws_secret_access_key=y\n",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert_eq!(
        catalog.get("duplicated").and_then(|p| p.region.as_deref()),
        Some("us-east-1")
    );
    assert_eq!(
        catalog
            .issues()
            .iter()
            .filter(|issue| matches!(issue.kind, CatalogIssueKind::DuplicateSection { .. }))
            .count(),
        2
    );
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::DuplicateMetadata {
                field: MetadataField::Region,
            }
    }));
    assert!(!catalog.profile_is_activatable("duplicated"));
}

#[test]
fn graph_issues_are_deterministic_and_environment_proof_fails_closed() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[profile role]
role_arn = arn:aws:iam::111122223333:role/Outer
source_profile = environment
[profile environment]
credential_source = Environment
[profile missing]
role_arn = arn:aws:iam::111122223333:role/Missing
source_profile = absent
[profile cycle-b]
role_arn = arn:aws:iam::111122223333:role/CycleB
source_profile = cycle-a
[profile cycle-a]
role_arn = arn:aws:iam::111122223333:role/CycleA
source_profile = cycle-b
"#,
        b"",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert_eq!(
        catalog.environment_source("role"),
        EnvironmentSource::InvalidChain
    );
    assert_eq!(
        catalog.environment_source("environment"),
        EnvironmentSource::InvalidChain
    );
    assert_eq!(
        catalog.environment_source("missing"),
        EnvironmentSource::InvalidChain
    );
    assert_eq!(
        catalog.environment_source("cycle-a"),
        EnvironmentSource::InvalidChain
    );
    assert_eq!(
        catalog.environment_source("not-present"),
        EnvironmentSource::InvalidChain
    );
    assert!(!catalog.profile_is_activatable("role"));
    assert!(!catalog.profile_is_activatable("environment"));
    assert!(!catalog.profile_is_activatable("missing"));
    assert!(!catalog.profile_is_activatable("cycle-a"));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::MissingSourceProfile {
                profile: "missing".to_owned(),
                source_profile: "absent".to_owned(),
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::SourceProfileCycle {
                profiles: vec!["cycle-a".to_owned(), "cycle-b".to_owned()],
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "environment".to_owned(),
                violation: ProviderGraphViolation::CredentialSourceWithoutRoleArn,
            }
    }));
}

#[test]
fn credential_sources_are_typed_and_keep_their_configured_serialization() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[profile environment]
credential_source = Environment
[profile ecs]
credential_source = EcsContainer
[profile instance]
credential_source = Ec2InstanceMetadata
[profile future]
credential_source = FutureProvider
"#,
        b"",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert_eq!(
        catalog
            .get("environment")
            .and_then(|profile| profile.credential_source.as_ref()),
        Some(&CredentialSource::Environment)
    );
    assert_eq!(
        catalog
            .get("ecs")
            .and_then(|profile| profile.credential_source.as_ref()),
        Some(&CredentialSource::EcsContainer)
    );
    assert_eq!(
        catalog
            .get("instance")
            .and_then(|profile| profile.credential_source.as_ref()),
        Some(&CredentialSource::Ec2InstanceMetadata)
    );
    assert!(matches!(
        catalog
            .get("future")
            .and_then(|profile| profile.credential_source.as_ref()),
        Some(CredentialSource::Unknown(value)) if value == "FutureProvider"
    ));

    for name in ["environment", "ecs", "instance", "future"] {
        assert!(!catalog.profile_is_activatable(name), "{name}");
        assert_eq!(
            catalog.environment_source(name),
            EnvironmentSource::InvalidChain
        );
    }
    let standalone_violations: Vec<_> = catalog
        .issues()
        .iter()
        .filter_map(|issue| match &issue.kind {
            CatalogIssueKind::InvalidProviderGraph {
                profile,
                violation: ProviderGraphViolation::CredentialSourceWithoutRoleArn,
            } => Some(profile.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        standalone_violations,
        vec!["ecs", "environment", "future", "instance"]
    );

    for (name, configured) in [
        ("environment", "Environment"),
        ("ecs", "EcsContainer"),
        ("instance", "Ec2InstanceMetadata"),
        ("future", "FutureProvider"),
    ] {
        let serialized = serde_json::to_value(catalog.get(name).expect("profile serialization"))
            .expect("serialize profile");
        assert_eq!(serialized["credential_source"], configured);
    }
}

#[test]
fn login_provider_is_presence_only_and_conflicts_fail_closed() {
    let directory = TempDir::new().expect("temp directory");
    let sentinel = "arn:aws:iam::111122223333:user/LOGIN-IDENTITY-SENTINEL";
    let config = format!(
        "[profile login]\nlogin_session = {sentinel}\n\
         [profile conflicting]\nlogin_session = {sentinel}\ncredential_process = helper\n"
    );
    let sources = write_sources(directory.path(), config.as_bytes(), b"");

    let catalog = Catalog::load(sources).expect("load login-provider catalog");
    let login = catalog.get("login").expect("login profile");
    assert!(login.has_login_session);
    assert!(catalog.profile_is_activatable("login"));
    assert!(!catalog.profile_is_activatable("conflicting"));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "conflicting".to_owned(),
                violation: ProviderGraphViolation::ConflictingCredentialProviders,
            }
    }));

    let serialized = serde_json::to_string(&catalog).expect("serialize safe catalog");
    assert!(serialized.contains("has_login_session"));
    assert!(!serialized.contains(sentinel));
}

#[test]
fn login_provider_intent_is_proven_through_a_complete_source_chain() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile login]\nlogin_session=company\n\
[profile assumed]\nrole_arn=arn:aws:iam::111122223333:role/Assumed\nsource_profile=login\n\
[profile ordinary]\nregion=us-east-1\n",
        b"",
    );

    let catalog = Catalog::load(sources).expect("load login source-chain catalog");

    assert_eq!(
        catalog.environment_source("login"),
        EnvironmentSource::UsesLoginSession
    );
    assert_eq!(
        catalog.environment_source("assumed"),
        EnvironmentSource::UsesLoginSession
    );
    assert_eq!(
        catalog.environment_source("ordinary"),
        EnvironmentSource::DoesNotUseEnvironment
    );
}

#[test]
fn missing_sso_session_and_partial_legacy_sso_are_diagnosed() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile modern]\nsso_session = absent\nsso_account_id = 123456789012\nsso_role_name = Dev\n[profile partial]\nsso_start_url = https://example.invalid/start\n",
        b"",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert!(catalog.issues().iter().any(|issue| matches!(
        &issue.kind,
        CatalogIssueKind::MissingSsoSession { profile, session }
            if profile == "modern" && session == "absent"
    )));
    assert!(!catalog.profile_is_activatable("modern"));
    assert!(!catalog.profile_is_activatable("partial"));
    assert!(catalog.issues().iter().any(|issue| matches!(
        &issue.kind,
        CatalogIssueKind::IncompleteSso { profile, mode: SsoMode::Legacy, missing }
            if profile == "partial"
                && missing == &vec![
                    RequiredSsoField::Region,
                    RequiredSsoField::AccountId,
                    RequiredSsoField::RoleName,
                ]
    )));
}

#[test]
fn bearer_only_modern_sso_does_not_require_account_or_role() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile bearer]\nsso_session = bearer-session\n[sso-session bearer-session]\nsso_start_url = https://example.awsapps.com/start\nsso_region = us-west-2\nsso_registration_scopes = sso:account:access\n",
        b"",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert!(matches!(
        catalog.get("bearer").and_then(|profile| profile.sso.as_ref()),
        Some(SsoMetadata::Modern {
            account_id: None,
            role_name: None,
            registration_scopes,
            ..
        }) if registration_scopes.as_deref() == Some("sso:account:access")
    ));
    assert!(!catalog.issues().iter().any(|issue| matches!(
        &issue.kind,
        CatalogIssueKind::IncompleteSso { profile, .. } if profile == "bearer"
    )));
    assert!(catalog.profile_is_activatable("bearer"));
}

#[test]
fn modern_sso_requires_account_and_role_as_a_pair() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile account-only]\nsso_session = shared\nsso_account_id = 123456789012\n[profile role-only]\nsso_session = shared\nsso_role_name = ReadOnly\n[profile complete]\nsso_session = shared\nsso_account_id = 123456789012\nsso_role_name = ReadOnly\n[sso-session shared]\nsso_start_url = https://example.awsapps.com/start\nsso_region = us-west-2\n",
        b"",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert!(!catalog.profile_is_activatable("account-only"));
    assert!(!catalog.profile_is_activatable("role-only"));
    assert!(catalog.profile_is_activatable("complete"));
    assert!(catalog.issues().iter().any(|issue| matches!(
        &issue.kind,
        CatalogIssueKind::IncompleteSso { profile, mode: SsoMode::Modern, missing }
            if profile == "account-only" && missing == &vec![RequiredSsoField::RoleName]
    )));
    assert!(catalog.issues().iter().any(|issue| matches!(
        &issue.kind,
        CatalogIssueKind::IncompleteSso { profile, mode: SsoMode::Modern, missing }
            if profile == "role-only" && missing == &vec![RequiredSsoField::AccountId]
    )));
}

#[test]
fn invalid_provider_graph_never_proves_environment_credentials() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[profile base]
region = us-east-1
[profile both]
role_arn = arn:aws:iam::111122223333:role/Both
source_profile = base
credential_source = Environment
[profile no-role]
source_profile = base
[profile environment-base]
credential_source = Environment
[profile missing-source]
role_arn = arn:aws:iam::111122223333:role/Missing
[profile web-no-role]
web_identity_token_file = /token
[profile unknown]
role_arn = arn:aws:iam::111122223333:role/Unknown
credential_source = SomethingElse
[profile conflicting]
sso_session = company
credential_process = helper
[sso-session company]
sso_start_url = https://example.awsapps.com/start
sso_region = us-east-1
"#,
        b"",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert_eq!(
        catalog.environment_source("both"),
        EnvironmentSource::InvalidChain
    );
    assert_eq!(
        catalog.environment_source("no-role"),
        EnvironmentSource::InvalidChain
    );
    assert_eq!(
        catalog.environment_source("environment-base"),
        EnvironmentSource::InvalidChain
    );
    assert!(!catalog.profile_is_activatable("both"));
    assert!(!catalog.profile_is_activatable("no-role"));
    assert!(!catalog.profile_is_activatable("environment-base"));
    assert!(!catalog.profile_is_activatable("missing-source"));
    assert!(!catalog.profile_is_activatable("web-no-role"));
    assert!(!catalog.profile_is_activatable("unknown"));
    assert!(!catalog.profile_is_activatable("conflicting"));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "both".to_owned(),
                violation: ProviderGraphViolation::MultipleRoleCredentialSources,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "no-role".to_owned(),
                violation: ProviderGraphViolation::SourceProfileWithoutRoleArn,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "environment-base".to_owned(),
                violation: ProviderGraphViolation::CredentialSourceWithoutRoleArn,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "missing-source".to_owned(),
                violation: ProviderGraphViolation::RoleMissingCredentialSource,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "web-no-role".to_owned(),
                violation: ProviderGraphViolation::WebIdentityWithoutRoleArn,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "unknown".to_owned(),
                violation: ProviderGraphViolation::UnknownCredentialSource,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "conflicting".to_owned(),
                violation: ProviderGraphViolation::ConflictingCredentialProviders,
            }
    }));
}

#[test]
fn self_source_is_valid_only_for_a_role_with_complete_local_static_credentials() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile self]\nrole_arn=arn:aws:iam::111122223333:role/Self\nsource_profile=self\n\
[profile outer]\nrole_arn=arn:aws:iam::111122223333:role/Outer\nsource_profile=self\n\
[profile a-self]\nrole_arn=arn:aws:iam::111122223333:role/MemoizedSelf\nsource_profile=a-self\n\
[profile z-outer]\nrole_arn=arn:aws:iam::111122223333:role/MemoizedOuter\nsource_profile=a-self\n\
[profile incomplete]\nrole_arn=arn:aws:iam::111122223333:role/Incomplete\nsource_profile=incomplete\n\
[profile conflicting]\nrole_arn=arn:aws:iam::111122223333:role/Conflicting\nsource_profile=conflicting\ncredential_process=helper\n\
[profile no-role]\nsource_profile=no-role\n",
        b"[self]\naws_access_key_id=value\naws_secret_access_key=value\n\
[a-self]\naws_access_key_id=value\naws_secret_access_key=value\n\
[incomplete]\naws_access_key_id=value\n\
[conflicting]\naws_access_key_id=value\naws_secret_access_key=value\n\
[no-role]\naws_access_key_id=value\naws_secret_access_key=value\n",
    );

    let catalog = Catalog::load(sources).expect("load self-source catalog");

    assert!(catalog.profile_is_activatable("self"));
    assert!(!catalog.profile_is_activatable("outer"));
    assert!(catalog.profile_is_activatable("a-self"));
    assert!(!catalog.profile_is_activatable("z-outer"));
    assert_eq!(
        catalog.environment_source("self"),
        EnvironmentSource::DoesNotUseEnvironment
    );
    assert_eq!(
        catalog.environment_source("outer"),
        EnvironmentSource::InvalidChain
    );
    assert_eq!(
        catalog.environment_source("a-self"),
        EnvironmentSource::DoesNotUseEnvironment
    );
    assert_eq!(
        catalog.environment_source("z-outer"),
        EnvironmentSource::InvalidChain
    );
    for invalid in ["incomplete", "conflicting", "no-role"] {
        assert!(!catalog.profile_is_activatable(invalid), "{invalid}");
        assert_eq!(
            catalog.environment_source(invalid),
            EnvironmentSource::InvalidChain
        );
    }
    assert!(!catalog.issues().iter().any(|issue| matches!(
        &issue.kind,
        CatalogIssueKind::SourceProfileCycle { profiles }
            if profiles == &vec!["self".to_owned()]
    )));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "outer".to_owned(),
                violation: ProviderGraphViolation::SourceProfileNotCredentialCapable,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "z-outer".to_owned(),
                violation: ProviderGraphViolation::SourceProfileNotCredentialCapable,
            }
    }));
}

#[test]
fn only_signing_credential_profiles_are_usable_as_role_sources() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[profile config-only]
region=us-east-1
[profile from-config-only]
role_arn=arn:aws:iam::111122223333:role/FromConfig
source_profile=config-only
[profile bearer]
sso_session=bearer-session
[profile from-bearer]
role_arn=arn:aws:iam::111122223333:role/FromBearer
source_profile=bearer
[profile standalone-environment]
credential_source=Environment
[profile from-standalone-environment]
role_arn=arn:aws:iam::111122223333:role/FromEnvironment
source_profile=standalone-environment
[profile process]
credential_process=helper
[profile from-process]
role_arn=arn:aws:iam::111122223333:role/FromProcess
source_profile=process
[profile login]
login_session=company
[profile from-login]
role_arn=arn:aws:iam::111122223333:role/FromLogin
source_profile=login
[profile credential-sso]
sso_session=credential-session
sso_account_id=111122223333
sso_role_name=Developer
[profile from-credential-sso]
role_arn=arn:aws:iam::111122223333:role/FromSso
source_profile=credential-sso
[profile web]
role_arn=arn:aws:iam::111122223333:role/Web
web_identity_token_file=/token
[profile from-web]
role_arn=arn:aws:iam::111122223333:role/FromWeb
source_profile=web
[profile from-static]
role_arn=arn:aws:iam::111122223333:role/FromStatic
source_profile=static
[profile chained-role]
role_arn=arn:aws:iam::111122223333:role/Chained
source_profile=from-static
[profile metadata-role]
role_arn=arn:aws:iam::111122223333:role/Metadata
credential_source=Ec2InstanceMetadata
[profile from-metadata-role]
role_arn=arn:aws:iam::111122223333:role/FromMetadata
source_profile=metadata-role
[sso-session bearer-session]
sso_start_url=https://example.awsapps.com/start
sso_region=us-east-1
[sso-session credential-session]
sso_start_url=https://example.awsapps.com/start
sso_region=us-east-1
"#,
        b"[static]\naws_access_key_id=value\naws_secret_access_key=value\n",
    );

    let catalog = Catalog::load(sources).expect("load source capability catalog");

    for locally_selectable in ["config-only", "bearer"] {
        assert!(
            catalog.profile_is_activatable(locally_selectable),
            "{locally_selectable} should remain a top-level selection"
        );
    }
    assert!(!catalog.profile_is_activatable("standalone-environment"));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "standalone-environment".to_owned(),
                violation: ProviderGraphViolation::CredentialSourceWithoutRoleArn,
            }
    }));
    for invalid_source_chain in [
        "from-config-only",
        "from-bearer",
        "from-standalone-environment",
    ] {
        assert!(
            !catalog.profile_is_activatable(invalid_source_chain),
            "{invalid_source_chain}"
        );
    }
    for non_signing_source in ["from-config-only", "from-bearer"] {
        assert!(catalog.issues().iter().any(|issue| {
            issue.kind
                == CatalogIssueKind::InvalidProviderGraph {
                    profile: non_signing_source.to_owned(),
                    violation: ProviderGraphViolation::SourceProfileNotCredentialCapable,
                }
        }));
    }
    for valid_source_chain in [
        "from-process",
        "from-login",
        "from-credential-sso",
        "from-web",
        "from-static",
        "chained-role",
        "from-metadata-role",
    ] {
        assert!(
            catalog.profile_is_activatable(valid_source_chain),
            "{valid_source_chain}"
        );
    }
}

#[test]
fn environment_credential_source_roles_are_not_safely_selectable() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[profile a-safe]
region=us-east-1
[profile b-environment-role]
role_arn=arn:aws:iam::111122223333:role/Environment
credential_source=Environment
[profile c-ecs-role]
role_arn=arn:aws:iam::111122223333:role/Ecs
credential_source=EcsContainer
[profile d-imds-role]
role_arn=arn:aws:iam::111122223333:role/Imds
credential_source=Ec2InstanceMetadata
[profile e-environment-standalone]
credential_source=Environment
"#,
        b"",
    );

    let catalog = Catalog::load(sources).expect("load environment-source catalog");

    assert_eq!(
        catalog.selectable_names().collect::<Vec<_>>(),
        vec!["a-safe", "c-ecs-role", "d-imds-role"]
    );
    assert_eq!(
        catalog.environment_source("b-environment-role"),
        EnvironmentSource::InvalidChain
    );
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "b-environment-role".to_owned(),
                violation:
                    ProviderGraphViolation::EnvironmentCredentialSourceCannotBeSelectedSafely,
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::InvalidProviderGraph {
                profile: "e-environment-standalone".to_owned(),
                violation: ProviderGraphViolation::CredentialSourceWithoutRoleArn,
            }
    }));
}

#[test]
fn incomplete_static_credential_tuple_reports_only_missing_key_names() {
    let directory = TempDir::new().expect("temp directory");
    let sentinel = "STATIC-secret-must-not-leak";
    let credentials = format!(
        "[access-only]\naws_access_key_id={sentinel}\n[secret-only]\naws_secret_access_key={sentinel}\n[complete]\naws_access_key_id={sentinel}\naws_secret_access_key={sentinel}\n"
    );
    let sources = write_sources(directory.path(), b"", credentials.as_bytes());

    let catalog = Catalog::load(sources).expect("load catalog");
    let diagnostics = format!("{:#?}", catalog.issues());

    assert!(!diagnostics.contains(sentinel));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::IncompleteStaticCredentials {
                profile: "access-only".to_owned(),
                missing: vec![StaticCredentialField::SecretAccessKey],
            }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.kind
            == CatalogIssueKind::IncompleteStaticCredentials {
                profile: "secret-only".to_owned(),
                missing: vec![StaticCredentialField::AccessKeyId],
            }
    }));
    assert!(!catalog.issues().iter().any(|issue| matches!(
        &issue.kind,
        CatalogIssueKind::IncompleteStaticCredentials { profile, .. } if profile == "complete"
    )));
    assert!(!catalog.profile_is_activatable("access-only"));
    assert!(!catalog.profile_is_activatable("secret-only"));
    assert!(catalog.profile_is_activatable("complete"));
}

#[test]
fn long_session_token_is_bounded_by_the_line_budget_not_metadata_retention() {
    let directory = TempDir::new().expect("temp directory");
    let sentinel = "LONG-SESSION-TOKEN-MUST-NOT-BE-RETAINED";
    let token = format!("{sentinel}{}", "x".repeat(8 * 1024));
    let credentials = format!(
        "[temporary]\naws_access_key_id=access\naws_secret_access_key=secret\naws_session_token={token}\n"
    );
    let sources = write_sources(directory.path(), b"", credentials.as_bytes());

    let catalog = Catalog::load(sources).expect("load long session token catalog");

    assert!(catalog.profile_is_activatable("temporary"));
    assert!(
        catalog
            .get("temporary")
            .is_some_and(|profile| profile.static_credentials.session_token)
    );
    assert!(
        !serde_json::to_string(&catalog)
            .expect("serialize credential-blind catalog")
            .contains(sentinel)
    );
}

#[test]
fn static_credential_tuples_must_be_complete_within_each_source_file() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile split]\naws_access_key_id=config-value\n\
[profile shadowed-fragment]\naws_access_key_id=config-value\n",
        b"[split]\naws_secret_access_key=credentials-value\n\
[shadowed-fragment]\naws_access_key_id=credentials-value\naws_secret_access_key=credentials-value\n",
    );

    let catalog = Catalog::load(sources).expect("load split static credential catalog");

    for name in ["split", "shadowed-fragment"] {
        assert!(
            !catalog.profile_is_activatable(name),
            "{name} must not rely on cross-file static credential repair"
        );
    }
    assert!(catalog.issues().iter().any(|issue| {
        issue.source == CatalogFileKind::Config
            && issue.kind
                == CatalogIssueKind::IncompleteStaticCredentials {
                    profile: "split".to_owned(),
                    missing: vec![StaticCredentialField::SecretAccessKey],
                }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.source == CatalogFileKind::Credentials
            && issue.kind
                == CatalogIssueKind::IncompleteStaticCredentials {
                    profile: "split".to_owned(),
                    missing: vec![StaticCredentialField::AccessKeyId],
                }
    }));
    assert!(catalog.issues().iter().any(|issue| {
        issue.source == CatalogFileKind::Config
            && issue.kind
                == CatalogIssueKind::IncompleteStaticCredentials {
                    profile: "shadowed-fragment".to_owned(),
                    missing: vec![StaticCredentialField::SecretAccessKey],
                }
    }));
}

#[test]
fn activation_readiness_is_local_and_transitive_across_parser_recovery() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile healthy]\nregion=us-east-1\n[profile broken]\nregion=us-west-2\nnot a property\n[profile through-broken]\nrole_arn=arn:aws:iam::111122223333:role/Derived\nsource_profile=broken\n[profile invalid-metadata]\nregion=us-east-1\x1b[31m\n[profile bad-sso]\nsso_session=ambiguous-session\n[profile healthy-sso]\nsso_session=healthy-session\n[sso-session ambiguous-session]\nsso_start_url=https://example.awsapps.com/start\nsso_region=us-east-1\nsso_region=us-west-2\n[sso-session healthy-session]\nsso_start_url=https://example.awsapps.com/start\nsso_region=us-east-1\n",
        b"[broken-credentials]\naws_access_key_id=value\nnot a property\n[healthy-credentials]\naws_access_key_id=value\naws_secret_access_key=value\n",
    );

    let catalog = Catalog::load(sources).expect("load catalog");

    assert!(catalog.profile_is_activatable("healthy"));
    assert!(catalog.profile_is_activatable("healthy-sso"));
    assert!(catalog.profile_is_activatable("healthy-credentials"));
    assert!(!catalog.profile_is_activatable("broken"));
    assert!(!catalog.profile_is_activatable("through-broken"));
    assert!(!catalog.profile_is_activatable("invalid-metadata"));
    assert!(!catalog.profile_is_activatable("bad-sso"));
    assert!(!catalog.profile_is_activatable("broken-credentials"));
    assert!(!catalog.profile_is_activatable("absent"));
}

#[test]
fn default_missing_paths_are_empty_but_explicit_missing_paths_fail() {
    let directory = TempDir::new().expect("temp directory");
    let defaults = SourcePaths::new(
        SourceFile::new(directory.path().join("missing-config"), PathOrigin::Default),
        SourceFile::new(
            directory.path().join("missing-credentials"),
            PathOrigin::Default,
        ),
    );

    let catalog = Catalog::load(defaults).expect("missing defaults are empty");
    assert!(catalog.is_empty());

    let explicit = SourcePaths::new(
        SourceFile::new(
            directory.path().join("missing-config"),
            PathOrigin::Environment,
        ),
        SourceFile::new(
            directory.path().join("missing-credentials"),
            PathOrigin::Default,
        ),
    );
    assert!(matches!(
        Catalog::load(explicit),
        Err(CatalogError::Open {
            source_kind: CatalogFileKind::Config,
            ..
        })
    ));
}

#[test]
fn line_limit_discards_the_whole_section_and_recovers_at_next_header() {
    let directory = TempDir::new().expect("temp directory");
    let oversized = "x".repeat(70 * 1024);
    let config = format!(
        "[profile discarded]\nregion = {oversized}\nrole_arn = should-not-attach\n[profile recovered]\nregion=us-west-2\n"
    );
    let sources = write_sources(directory.path(), config.as_bytes(), b"");

    let catalog = Catalog::load(sources).expect("load catalog");

    assert_eq!(
        catalog.get("recovered").and_then(|p| p.region.as_deref()),
        Some("us-west-2")
    );
    assert_eq!(
        catalog.get("discarded").and_then(|p| p.region.as_deref()),
        None
    );
    assert!(
        catalog
            .issues()
            .iter()
            .any(|issue| issue.kind == CatalogIssueKind::LineTooLong)
    );
    assert!(!catalog.profile_is_activatable("discarded"));
    assert!(catalog.profile_is_activatable("recovered"));
}

#[test]
fn physical_line_budget_counts_the_newline_and_accepts_exact_eof_boundary() {
    use std::io::{BufReader, Cursor};

    let exact_eof = vec![b'#'; parser::MAX_LINE_BYTES];
    let parsed = parser::parse_config(BufReader::new(Cursor::new(exact_eof)), Path::new("config"))
        .expect("exact physical-line limit at EOF");
    assert!(parsed.issues.is_empty());

    let mut over_with_newline = vec![b'#'; parser::MAX_LINE_BYTES];
    over_with_newline.push(b'\n');
    let parsed = parser::parse_config(
        BufReader::new(Cursor::new(over_with_newline)),
        Path::new("config"),
    )
    .expect("line overflow is a bounded issue");
    assert!(
        parsed
            .issues
            .iter()
            .any(|issue| issue.kind == CatalogIssueKind::LineTooLong)
    );
}

#[test]
fn source_and_issue_budgets_fail_instead_of_growing_without_bound() {
    use std::io::{BufReader, Cursor};

    let oversized = vec![b'#'; (parser::MAX_SOURCE_BYTES as usize) + 1];
    let error = parser::parse_config(BufReader::new(Cursor::new(oversized)), Path::new("config"))
        .expect_err("oversized source must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    let many_issues = "outside=value\n".repeat(parser::MAX_ISSUES + 1);
    let error = parser::parse_config(
        BufReader::new(Cursor::new(many_issues.into_bytes())),
        Path::new("config"),
    )
    .expect_err("issue flood must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    let at_issue_limit = format!(
        "{}[profile recovered]\nregion=us-east-1\n",
        "outside=value\n".repeat(parser::MAX_ISSUES)
    );
    let parsed = parser::parse_config(
        BufReader::new(Cursor::new(at_issue_limit.into_bytes())),
        Path::new("config"),
    )
    .expect("exact issue budget must still permit a valid recovery section");
    assert_eq!(parsed.issues.len(), parser::MAX_ISSUES);
    assert!(parsed.profiles.contains_key("recovered"));
}

#[test]
fn catalog_entry_budget_bounds_many_short_sections() {
    use std::io::{BufReader, Cursor};

    let mut at_limit = String::new();
    for index in 0..parser::MAX_CATALOG_ENTRIES {
        at_limit.push_str(&format!("[profile p{index}]\n"));
    }
    let parsed = parser::parse_config(
        BufReader::new(Cursor::new(at_limit.into_bytes())),
        Path::new("config"),
    )
    .expect("entry budget boundary must parse");
    assert_eq!(parsed.profiles.len(), parser::MAX_CATALOG_ENTRIES);

    let mut over_limit = String::new();
    for index in 0..=parser::MAX_CATALOG_ENTRIES {
        over_limit.push_str(&format!("[p{index}]\n"));
    }
    let error = parser::parse_credentials(
        BufReader::new(Cursor::new(over_limit.into_bytes())),
        Path::new("credentials"),
    )
    .expect_err("entry flood must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn duplicate_presence_only_provider_keys_are_quarantined_without_reading_values() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[profile duplicate-process]
credential_process = first secret command
credential_process = second secret command

[profile duplicate-web]
role_arn = arn:aws:iam::111122223333:role/Web
web_identity_token_file = /first/private/token
web_identity_token_file = /second/private/token

[profile duplicate-login]
login_session = arn:aws:iam::111122223333:user/FIRST-LOGIN-SENTINEL
login_session = arn:aws:iam::111122223333:user/SECOND-LOGIN-SENTINEL
"#,
        br#"[duplicate-static]
aws_access_key_id = FIRST-SECRET
aws_access_key_id = SECOND-SECRET
aws_secret_access_key = THIRD-SECRET

[credentials-process-is-unknown]
credential_process = command-that-belongs-in-config
"#,
    );

    let catalog = Catalog::load(sources).expect("load catalog");
    for name in [
        "duplicate-process",
        "duplicate-web",
        "duplicate-login",
        "duplicate-static",
    ] {
        assert!(
            !catalog.profile_is_activatable(name),
            "{name} must fail closed"
        );
    }
    for field in [
        MetadataField::CredentialProcess,
        MetadataField::WebIdentityTokenFile,
        MetadataField::LoginSession,
        MetadataField::AccessKeyId,
    ] {
        assert!(
            catalog
                .issues()
                .iter()
                .any(|issue| { issue.kind == CatalogIssueKind::DuplicateMetadata { field } })
        );
    }
    let credentials_process = catalog
        .get("credentials-process-is-unknown")
        .expect("credentials section remains discoverable");
    assert!(!credentials_process.has_credential_process);
    let diagnostics = serde_json::to_string(catalog.issues()).expect("serialize issues");
    for secret in [
        "first secret",
        "second secret",
        "FIRST-SECRET",
        "/first/private",
        "FIRST-LOGIN-SENTINEL",
    ] {
        assert!(!diagnostics.contains(secret));
    }
}

#[test]
fn indentation_distinguishes_normal_properties_from_unknown_aggregate_children() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        br#"[profile indented]
  region = ap-northeast-1
  credential_process = helper

[profile aggregate]
future_setting =
  credential_process = nested-helper-must-be-ignored
  [profile nested-header-must-be-ignored]
region = us-east-1
"#,
        b"",
    );

    let catalog = Catalog::load(sources).expect("load catalog");
    let indented = catalog.get("indented").expect("indented profile");
    assert_eq!(indented.region.as_deref(), Some("ap-northeast-1"));
    assert!(indented.has_credential_process);
    let aggregate = catalog.get("aggregate").expect("aggregate profile");
    assert_eq!(aggregate.region.as_deref(), Some("us-east-1"));
    assert!(!aggregate.has_credential_process);
    assert!(catalog.get("nested-header-must-be-ignored").is_none());
}

#[test]
fn credentials_unknown_aggregates_do_not_gain_config_or_nested_credential_semantics() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"",
        br#"[ordinary-indented]
  aws_access_key_id = present
  aws_secret_access_key = present

[future-aggregate]
future_provider =
  aws_access_key_id = nested-must-be-ignored
aws_secret_access_key = present

[config-key-is-unknown]
credential_process = helper-belongs-in-config
  aws_access_key_id = nested-must-be-ignored
aws_secret_access_key = present
"#,
    );

    let catalog = Catalog::load(sources).expect("load catalog");
    assert!(catalog.profile_is_activatable("ordinary-indented"));
    for name in ["future-aggregate", "config-key-is-unknown"] {
        let profile = catalog.get(name).expect("credentials-only profile");
        assert!(!profile.static_credentials.access_key_id);
        assert!(profile.static_credentials.secret_access_key);
        assert!(!profile.has_credential_process);
        assert!(!catalog.profile_is_activatable(name));
    }
}

#[test]
fn empty_presence_only_values_are_invalid_without_becoming_secret_data() {
    let directory = TempDir::new().expect("temp directory");
    let sources = write_sources(
        directory.path(),
        b"[profile process]\ncredential_process =\n[profile web]\nrole_arn=arn:aws:iam::111122223333:role/Web\nweb_identity_token_file=\n[profile login]\nlogin_session=\n",
        b"[static]\naws_access_key_id=\naws_secret_access_key=\n",
    );
    let catalog = Catalog::load(sources).expect("load catalog");
    for name in ["process", "web", "login", "static"] {
        assert!(!catalog.profile_is_activatable(name));
    }
}

#[cfg(unix)]
#[test]
fn catalog_follows_symlinks_whose_targets_are_regular_files() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new().expect("temp directory");
    let target = directory.path().join("target-config");
    fs::write(
        &target,
        b"[profile redirected]\ncredential_process=helper\n",
    )
    .unwrap();
    let config = directory.path().join("config");
    symlink(&target, &config).unwrap();
    fs::write(directory.path().join("credentials"), b"").unwrap();

    let catalog = Catalog::load(source_paths(directory.path())).unwrap();

    assert!(catalog.profile_is_activatable("redirected"));
    assert_eq!(
        fs::read(&target).unwrap(),
        b"[profile redirected]\ncredential_process=helper\n"
    );
}

#[cfg(windows)]
#[test]
fn windows_catalog_reparse_points_remain_fail_closed() {
    use std::os::windows::fs::symlink_file;

    let directory = TempDir::new().expect("temp directory");
    let target = directory.path().join("target-config");
    let contents = b"[profile redirected]\ncredential_process=helper\n";
    fs::write(&target, contents).unwrap();
    symlink_file(&target, directory.path().join("config")).unwrap();
    fs::write(directory.path().join("credentials"), b"").unwrap();

    // Unlike Unix O_NONBLOCK + fstat, std does not give this reader a way to
    // guarantee that following an arbitrary Windows reparse point cannot enter
    // a blocking/custom provider before the post-open metadata check. Keep the
    // Windows boundary narrower until that property can be proven.
    let error = Catalog::load(source_paths(directory.path())).unwrap_err();

    assert!(matches!(error, CatalogError::Open { .. }));
    assert_eq!(fs::read(&target).unwrap(), contents);
}

#[cfg(windows)]
#[test]
fn windows_device_namespaces_are_rejected_before_open() {
    for path in [
        Path::new(r"\\.\pipe\awswit-config"),
        Path::new(r"\\?\GLOBALROOT\Device\NamedPipe\awswit-config"),
    ] {
        assert!(path_uses_windows_device_namespace(path), "{path:?}");
    }
    for path in [
        Path::new(r"C:\Users\example\.aws\config"),
        Path::new(r"\\?\C:\Users\example\.aws\config"),
        Path::new(r"\\server\share\.aws\config"),
        Path::new(r"\\?\UNC\server\share\.aws\config"),
    ] {
        assert!(!path_uses_windows_device_namespace(path), "{path:?}");
    }
}

#[test]
#[ignore = "release-mode performance contract; CI runs this explicitly"]
fn performance_contract_catalog_load_and_filter_1000_profiles_p95() {
    use std::time::{Duration, Instant};

    if cfg!(debug_assertions) {
        panic!("run this contract with --release");
    }
    const PROFILE_COUNT: usize = 1_000;
    const SAMPLES: usize = 31;
    const P95_BUDGET: Duration = Duration::from_millis(200);

    let directory = TempDir::new().expect("performance fixture directory");
    let mut config =
        String::from("[profile perf-0000]\ncredential_process = deterministic-placeholder\n");
    for index in 1..PROFILE_COUNT {
        config.push_str(&format!(
            "[profile perf-{index:04}]\nrole_arn = arn:aws:iam::111122223333:role/Perf{index:04}\nsource_profile = perf-{:04}\n",
            index - 1
        ));
    }
    let sources = write_sources(directory.path(), config.as_bytes(), b"");
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let catalog = Catalog::load(sources.clone()).expect("load performance fixture");
        assert_eq!(catalog.selectable_names().count(), PROFILE_COUNT);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let p95 = samples[(SAMPLES * 95).div_ceil(100) - 1];
    eprintln!("catalog-1000 load+filter p95={p95:?}, budget={P95_BUDGET:?}");
    assert!(
        p95 <= P95_BUDGET,
        "catalog-1000 load+filter p95 {p95:?} exceeded {P95_BUDGET:?}"
    );
}
