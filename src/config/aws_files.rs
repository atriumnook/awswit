use std::collections::HashMap;
use std::path::PathBuf;

use configparser::ini::Ini;

use crate::error::{AwswitError, Result};
use crate::profile::types::Profile;

/// Get the path to the AWS config file
pub fn get_config_path(override_path: Option<&str>) -> PathBuf {
    if let Some(path) = override_path {
        return PathBuf::from(shellexpand::tilde(path).to_string());
    }
    if let Ok(path) = std::env::var("AWS_CONFIG_FILE") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".aws")
        .join("config")
}

/// Get the path to the AWS credentials file
pub fn get_credentials_path(override_path: Option<&str>) -> PathBuf {
    if let Some(path) = override_path {
        return PathBuf::from(shellexpand::tilde(path).to_string());
    }
    if let Ok(path) = std::env::var("AWS_SHARED_CREDENTIALS_FILE") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".aws")
        .join("credentials")
}

/// Parse an INI file and return a map of section -> key -> value
fn parse_ini_file(path: &PathBuf) -> Result<HashMap<String, HashMap<String, String>>> {
    let mut ini = Ini::new();
    ini.set_comment_symbols(&['#', ';']);

    if !path.exists() {
        return Ok(HashMap::new());
    }

    let map = ini.load(path.to_str().unwrap_or_default()).map_err(|e| {
        AwswitError::config_file_error(format!("Failed to parse {}: {}", path.display(), e))
    })?;

    let mut result = HashMap::new();
    for (section, values) in map {
        let mut section_map = HashMap::new();
        for (key, value) in values {
            if let Some(v) = value {
                section_map.insert(key, v);
            }
        }
        result.insert(section, section_map);
    }

    Ok(result)
}

/// Normalize profile section name from config file
/// Config file uses "profile xxx" prefix, credentials file uses just "xxx"
fn normalize_profile_name(section: &str) -> String {
    if let Some(name) = section.strip_prefix("profile ") {
        name.to_string()
    } else {
        section.to_string()
    }
}

/// Load all AWS profiles from config and credentials files
pub fn load_profiles(
    config_path: Option<&str>,
    credentials_path: Option<&str>,
) -> Result<HashMap<String, Profile>> {
    let config_file = get_config_path(config_path);
    let creds_file = get_credentials_path(credentials_path);

    let config_sections = parse_ini_file(&config_file)?;
    let creds_sections = parse_ini_file(&creds_file)?;

    let mut profiles: HashMap<String, Profile> = HashMap::new();

    // Process config file
    for (section, values) in &config_sections {
        let name = normalize_profile_name(section);
        let profile = profiles
            .entry(name.clone())
            .or_insert_with(|| Profile::new(name));
        apply_config_values(profile, values);
    }

    // Process credentials file (merge into existing profiles)
    for (section, values) in &creds_sections {
        let name = section.clone();
        let profile = profiles
            .entry(name.clone())
            .or_insert_with(|| Profile::new(name));
        apply_credential_values(profile, values);
    }

    Ok(profiles)
}

fn apply_config_values(profile: &mut Profile, values: &HashMap<String, String>) {
    if let Some(v) = values.get("role_arn") {
        profile.role_arn = Some(v.clone());
    }
    if let Some(v) = values.get("source_profile") {
        profile.source_profile = Some(v.clone());
    }
    if let Some(v) = values.get("credential_source") {
        profile.credential_source = Some(v.clone());
    }
    if let Some(v) = values.get("external_id") {
        profile.external_id = Some(v.clone());
    }
    if let Some(v) = values.get("role_session_name") {
        profile.role_session_name = Some(v.clone());
    }
    if let Some(v) = values.get("duration_seconds") {
        profile.duration_seconds = v.parse().ok();
    }
    if let Some(v) = values.get("mfa_serial") {
        profile.mfa_serial = Some(v.clone());
    }
    if let Some(v) = values.get("region") {
        profile.region = Some(v.clone());
    }
    if let Some(v) = values.get("output") {
        profile.output = Some(v.clone());
    }
    if let Some(v) = values.get("credential_process") {
        profile.credential_process = Some(v.clone());
    }
    // SSO fields
    if let Some(v) = values.get("sso_start_url") {
        profile.sso_start_url = Some(v.clone());
    }
    if let Some(v) = values.get("sso_region") {
        profile.sso_region = Some(v.clone());
    }
    if let Some(v) = values.get("sso_account_id") {
        profile.sso_account_id = Some(v.clone());
    }
    if let Some(v) = values.get("sso_role_name") {
        profile.sso_role_name = Some(v.clone());
    }
    // awswit-specific
    if let Some(v) = values.get("manager") {
        profile.manager = Some(v.clone());
    }
    if let Some(v) = values.get("autoawswit") {
        profile.autoawswit = Some(v == "true" || v == "1" || v == "yes");
    }
}

fn apply_credential_values(profile: &mut Profile, values: &HashMap<String, String>) {
    if let Some(v) = values.get("aws_access_key_id") {
        profile.aws_access_key_id = Some(v.clone());
    }
    if let Some(v) = values.get("aws_secret_access_key") {
        profile.aws_secret_access_key = Some(v.clone());
    }
    if let Some(v) = values.get("aws_session_token") {
        profile.aws_session_token = Some(v.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_normalize_profile_name() {
        assert_eq!(normalize_profile_name("profile dev"), "dev");
        assert_eq!(normalize_profile_name("default"), "default");
        assert_eq!(normalize_profile_name("profile prod-admin"), "prod-admin");
    }

    #[test]
    fn test_parse_config_file() {
        let mut config_file = NamedTempFile::new().unwrap();
        writeln!(
            config_file,
            "[default]\nregion = us-east-1\n\n[profile dev]\nrole_arn = arn:aws:iam::123456789012:role/dev\nsource_profile = default\nregion = us-west-2"
        )
        .unwrap();

        let mut creds_file = NamedTempFile::new().unwrap();
        writeln!(
            creds_file,
            "[default]\naws_access_key_id = AKIAIOSFODNN7EXAMPLE\naws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        )
        .unwrap();

        let profiles = load_profiles(
            Some(config_file.path().to_str().unwrap()),
            Some(creds_file.path().to_str().unwrap()),
        )
        .unwrap();

        assert!(profiles.contains_key("default"));
        assert!(profiles.contains_key("dev"));

        let default = &profiles["default"];
        assert_eq!(default.region.as_deref(), Some("us-east-1"));
        assert_eq!(
            default.aws_access_key_id.as_deref(),
            Some("AKIAIOSFODNN7EXAMPLE")
        );

        let dev = &profiles["dev"];
        assert_eq!(
            dev.role_arn.as_deref(),
            Some("arn:aws:iam::123456789012:role/dev")
        );
        assert_eq!(dev.source_profile.as_deref(), Some("default"));
        assert_eq!(dev.region.as_deref(), Some("us-west-2"));
    }

    #[test]
    fn test_empty_files() {
        let config_file = NamedTempFile::new().unwrap();
        let creds_file = NamedTempFile::new().unwrap();

        let profiles = load_profiles(
            Some(config_file.path().to_str().unwrap()),
            Some(creds_file.path().to_str().unwrap()),
        )
        .unwrap();

        assert!(profiles.is_empty());
    }
}
