use std::io::Write;
use tempfile::NamedTempFile;

use awswit::profile::{Profile, ProfileType};

#[test]
fn test_profile_type_detection() {
    // User profile
    let mut p = Profile::new("user".to_string());
    p.aws_access_key_id = Some("AKID".to_string());
    p.aws_secret_access_key = Some("SECRET".to_string());
    assert_eq!(p.profile_type(), ProfileType::User);

    // Role profile
    let mut p = Profile::new("role".to_string());
    p.role_arn = Some("arn:aws:iam::123456789012:role/Test".to_string());
    p.source_profile = Some("default".to_string());
    assert_eq!(p.profile_type(), ProfileType::Role);

    // SSO profile
    let mut p = Profile::new("sso".to_string());
    p.sso_start_url = Some("https://example.awsapps.com/start".to_string());
    assert_eq!(p.profile_type(), ProfileType::Sso);

    // CredentialProcess profile
    let mut p = Profile::new("cp".to_string());
    p.credential_process = Some("/usr/bin/get-creds".to_string());
    assert_eq!(p.profile_type(), ProfileType::CredentialProcess);

    // CredentialSource profile
    let mut p = Profile::new("cs".to_string());
    p.credential_source = Some("Environment".to_string());
    assert_eq!(p.profile_type(), ProfileType::CredentialSource);
}

#[test]
fn test_profile_chain_detection() {
    let mut config_file = NamedTempFile::new().unwrap();
    writeln!(
        config_file,
        r#"[default]
region = us-east-1

[profile level1]
role_arn = arn:aws:iam::111111111111:role/Level1
source_profile = default

[profile level2]
role_arn = arn:aws:iam::222222222222:role/Level2
source_profile = level1

[profile level3]
role_arn = arn:aws:iam::333333333333:role/Level3
source_profile = level2
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

    // Verify chain structure
    let level1 = &profiles["level1"];
    assert_eq!(level1.source_profile.as_deref(), Some("default"));

    let level2 = &profiles["level2"];
    assert_eq!(level2.source_profile.as_deref(), Some("level1"));

    let level3 = &profiles["level3"];
    assert_eq!(level3.source_profile.as_deref(), Some("level2"));
}

#[test]
fn test_profile_detail_lines() {
    let mut p = Profile::new("test-profile".to_string());
    p.role_arn = Some("arn:aws:iam::123456789012:role/Test".to_string());
    p.source_profile = Some("default".to_string());
    p.region = Some("us-east-1".to_string());
    p.mfa_serial = Some("arn:aws:iam::123456789012:mfa/user".to_string());

    let details = p.detail_lines();
    assert!(details.iter().any(|(k, _)| k == "Type"));
    assert!(details.iter().any(|(k, _)| k == "Role ARN"));
    assert!(details.iter().any(|(k, _)| k == "Source Profile"));
    assert!(details.iter().any(|(k, _)| k == "MFA Serial"));
    assert!(details.iter().any(|(k, _)| k == "Region"));
}

#[test]
fn test_profile_has_credentials() {
    let mut p = Profile::new("test".to_string());
    assert!(!p.has_credentials());

    p.aws_access_key_id = Some("AKID".to_string());
    assert!(!p.has_credentials());

    p.aws_secret_access_key = Some("SECRET".to_string());
    assert!(p.has_credentials());
}

#[test]
fn test_profile_requires_mfa() {
    let mut p = Profile::new("test".to_string());
    assert!(!p.requires_mfa());

    p.mfa_serial = Some("arn:aws:iam::123456789012:mfa/user".to_string());
    assert!(p.requires_mfa());
}

#[test]
fn test_mixed_config_and_credentials() {
    let mut config_file = NamedTempFile::new().unwrap();
    writeln!(
        config_file,
        r#"[default]
region = us-east-1
output = json

[profile shared]
role_arn = arn:aws:iam::123456789012:role/Shared
source_profile = default
"#
    )
    .unwrap();

    let mut creds_file = NamedTempFile::new().unwrap();
    writeln!(
        creds_file,
        r#"[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY

[extra-user]
aws_access_key_id = AKIAEXTRAUSER1234567
aws_secret_access_key = ExtraUserSecretKeyHere12345678901234567
"#
    )
    .unwrap();

    let profiles = awswit::config::load_profiles(
        Some(config_file.path().to_str().unwrap()),
        Some(creds_file.path().to_str().unwrap()),
    )
    .unwrap();

    // default from both files, shared from config, extra-user from creds
    assert_eq!(profiles.len(), 3);
    assert!(profiles.contains_key("default"));
    assert!(profiles.contains_key("shared"));
    assert!(profiles.contains_key("extra-user"));

    // extra-user should have credentials but no region (from creds file only)
    let extra = &profiles["extra-user"];
    assert!(extra.has_credentials());
    assert!(extra.region.is_none());
}
