//! Config and credentials file parsing tests using real library code

use std::fs;
use std::io::Write;
use tempfile::TempDir;

use awswit::config::AwsFiles;

fn setup_test_aws_dir() -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let config_path = temp_dir.path().join("config");
    let mut config_file = fs::File::create(&config_path).expect("Failed to create config");
    writeln!(
        config_file,
        r#"
[default]
region = us-east-1

[profile dev]
role_arn = arn:aws:iam::123456789012:role/DevRole
source_profile = default
region = us-west-2

[profile prod]
role_arn = arn:aws:iam::987654321098:role/ProdRole
source_profile = default
mfa_serial = arn:aws:iam::123456789012:mfa/user
"#
    )
    .expect("Failed to write config");

    let creds_path = temp_dir.path().join("credentials");
    let mut creds_file = fs::File::create(&creds_path).expect("Failed to create credentials");
    writeln!(
        creds_file,
        r#"
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#
    )
    .expect("Failed to write credentials");

    temp_dir
}

#[test]
fn test_load_and_parse_config_profiles() {
    let temp_dir = setup_test_aws_dir();
    let config_path = temp_dir.path().join("config");
    let creds_path = temp_dir.path().join("credentials");

    let aws_files = AwsFiles::load(
        config_path.to_str().unwrap(),
        creds_path.to_str().unwrap(),
    )
    .unwrap();

    assert!(aws_files.config_profiles.contains_key("dev"));
    assert!(aws_files.config_profiles.contains_key("prod"));
    assert!(aws_files.config_profiles.contains_key("default"));

    let dev = &aws_files.config_profiles["dev"];
    assert_eq!(
        dev.role_arn,
        Some("arn:aws:iam::123456789012:role/DevRole".to_string())
    );
    assert_eq!(dev.source_profile, Some("default".to_string()));
    assert_eq!(dev.region, Some("us-west-2".to_string()));
}

#[test]
fn test_merge_profiles_credentials_override() {
    let temp_dir = setup_test_aws_dir();
    let config_path = temp_dir.path().join("config");
    let creds_path = temp_dir.path().join("credentials");

    let aws_files = AwsFiles::load(
        config_path.to_str().unwrap(),
        creds_path.to_str().unwrap(),
    )
    .unwrap();

    let merged = aws_files.merge_profiles();

    let default = &merged["default"];
    assert_eq!(
        default.aws_access_key_id,
        Some("AKIAIOSFODNN7EXAMPLE".to_string())
    );
    assert_eq!(default.region, Some("us-east-1".to_string()));
}

#[test]
fn test_account_id_extraction_via_profile() {
    let temp_dir = setup_test_aws_dir();
    let config_path = temp_dir.path().join("config");
    let creds_path = temp_dir.path().join("credentials");

    let aws_files = AwsFiles::load(
        config_path.to_str().unwrap(),
        creds_path.to_str().unwrap(),
    )
    .unwrap();

    let dev = &aws_files.config_profiles["dev"];
    assert_eq!(dev.get_account_id(), Some("123456789012".to_string()));
}
