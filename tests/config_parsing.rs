use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_load_profiles_from_config() {
    let mut config_file = NamedTempFile::new().unwrap();
    writeln!(
        config_file,
        r#"[default]
region = us-east-1
output = json

[profile dev]
role_arn = arn:aws:iam::123456789012:role/DevRole
source_profile = default
region = us-west-2

[profile prod]
role_arn = arn:aws:iam::987654321098:role/ProdRole
source_profile = default
mfa_serial = arn:aws:iam::123456789012:mfa/user
region = eu-west-1
"#
    )
    .unwrap();

    let mut creds_file = NamedTempFile::new().unwrap();
    writeln!(
        creds_file,
        r#"[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#
    )
    .unwrap();

    let profiles = awswit::config::load_profiles(
        Some(config_file.path().to_str().unwrap()),
        Some(creds_file.path().to_str().unwrap()),
    )
    .unwrap();

    // Should have 3 profiles: default, dev, prod
    assert_eq!(profiles.len(), 3);
    assert!(profiles.contains_key("default"));
    assert!(profiles.contains_key("dev"));
    assert!(profiles.contains_key("prod"));

    // Check default profile
    let default = &profiles["default"];
    assert_eq!(default.region.as_deref(), Some("us-east-1"));
    assert_eq!(
        default.aws_access_key_id.as_deref(),
        Some("AKIAIOSFODNN7EXAMPLE")
    );
    assert!(default.has_credentials());

    // Check dev profile
    let dev = &profiles["dev"];
    assert_eq!(
        dev.role_arn.as_deref(),
        Some("arn:aws:iam::123456789012:role/DevRole")
    );
    assert_eq!(dev.source_profile.as_deref(), Some("default"));
    assert_eq!(dev.region.as_deref(), Some("us-west-2"));
    assert_eq!(
        dev.profile_type(),
        awswit::profile::ProfileType::Role
    );

    // Check prod profile
    let prod = &profiles["prod"];
    assert!(prod.requires_mfa());
    assert_eq!(
        prod.mfa_serial.as_deref(),
        Some("arn:aws:iam::123456789012:mfa/user")
    );
}

#[test]
fn test_load_profiles_missing_files() {
    let profiles = awswit::config::load_profiles(
        Some("/nonexistent/config"),
        Some("/nonexistent/credentials"),
    )
    .unwrap();

    assert!(profiles.is_empty());
}

#[test]
fn test_load_profiles_sso() {
    let mut config_file = NamedTempFile::new().unwrap();
    writeln!(
        config_file,
        r#"[profile sso-profile]
sso_start_url = https://my-sso.awsapps.com/start
sso_region = us-east-1
sso_account_id = 123456789012
sso_role_name = ReadOnlyAccess
region = us-east-1
"#
    )
    .unwrap();

    let creds_file = NamedTempFile::new().unwrap();

    let profiles = awswit::config::load_profiles(
        Some(config_file.path().to_str().unwrap()),
        Some(creds_file.path().to_str().unwrap()),
    )
    .unwrap();

    assert!(profiles.contains_key("sso-profile"));
    let sso = &profiles["sso-profile"];
    assert_eq!(
        sso.profile_type(),
        awswit::profile::ProfileType::Sso
    );
    assert_eq!(
        sso.sso_start_url.as_deref(),
        Some("https://my-sso.awsapps.com/start")
    );
}

#[test]
fn test_credential_process_profile() {
    let mut config_file = NamedTempFile::new().unwrap();
    writeln!(
        config_file,
        r#"[profile custom-creds]
credential_process = /usr/bin/my-credential-tool
region = us-east-1
"#
    )
    .unwrap();

    let creds_file = NamedTempFile::new().unwrap();

    let profiles = awswit::config::load_profiles(
        Some(config_file.path().to_str().unwrap()),
        Some(creds_file.path().to_str().unwrap()),
    )
    .unwrap();

    let profile = &profiles["custom-creds"];
    assert_eq!(
        profile.profile_type(),
        awswit::profile::ProfileType::CredentialProcess
    );
}
