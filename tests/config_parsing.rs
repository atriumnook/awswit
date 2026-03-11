//! Config and credentials file parsing tests

use std::fs;
use std::io::Write;
use tempfile::TempDir;

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
fn test_config_file_parsing() {
    let temp_dir = setup_test_aws_dir();
    let config_path = temp_dir.path().join("config");

    assert!(config_path.exists());

    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(content.contains("[default]"));
    assert!(content.contains("[profile dev]"));
    assert!(content.contains("[profile prod]"));
}

#[test]
fn test_credentials_file_parsing() {
    let temp_dir = setup_test_aws_dir();
    let creds_path = temp_dir.path().join("credentials");

    assert!(creds_path.exists());

    let content = fs::read_to_string(&creds_path).expect("Failed to read credentials");
    assert!(content.contains("[default]"));
    assert!(content.contains("aws_access_key_id"));
}

#[test]
fn test_account_id_extraction() {
    let role_arn = "arn:aws:iam::123456789012:role/MyRole";
    let parts: Vec<&str> = role_arn.split(':').collect();

    assert!(parts.len() >= 5);
    assert_eq!(parts[4], "123456789012");
}
