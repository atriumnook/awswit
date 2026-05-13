use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::AwswitError;
use crate::profile::Profile;

/// Handles loading and parsing of AWS config and credentials files.
#[derive(Debug, Default)]
pub struct AwsFiles {
    /// Profiles from ~/.aws/config
    pub config_profiles: HashMap<String, Profile>,
    /// Profiles from ~/.aws/credentials
    pub credentials_profiles: HashMap<String, Profile>,
}

impl AwsFiles {
    /// Load AWS config and credentials files. Missing files yield an empty
    /// map (caller decides whether that is an error); malformed lines are
    /// skipped with a `tracing::warn!` so a single bad section can't
    /// disable `awswit` for the whole shell.
    pub fn load(config_path: &str, credentials_path: &str) -> Result<Self, AwswitError> {
        let config_profiles = Self::load_config_file(config_path)?;
        let credentials_profiles = Self::load_credentials_file(credentials_path)?;

        Ok(Self {
            config_profiles,
            credentials_profiles,
        })
    }

    /// Hand-rolled tolerant INI loader.
    ///
    /// AWS config uses a tiny subset of INI: `[section]` headers, `key = value`
    /// pairs, and `;` / `#` comments. We deliberately avoid pulling in a full
    /// INI parser (which fails the whole file on a single malformed line) so
    /// that a stray `[profile bad` written by another tool can't break
    /// `awswit -l` for every shell on the system — `doctor` is the right
    /// place to surface those, not the unconditional load path.
    ///
    /// File permissions are intentionally not validated here. Securing
    /// `~/.aws/credentials` is the responsibility of the AWS SDK / aws-vault
    /// / the user — awswit parses the file contents but only retains section
    /// names and well-known metadata keys.
    fn load_ini_file(
        path: &str,
        label: &str,
        extract_profile_name: fn(&str) -> Option<String>,
    ) -> Result<HashMap<String, Profile>, AwswitError> {
        let path = shellexpand::tilde(path).to_string();

        if !Path::new(&path).exists() {
            tracing::debug!("{} not found: {}", label, path);
            return Ok(HashMap::new());
        }

        let content = fs::read_to_string(&path).map_err(|e| AwswitError::ConfigFileError {
            message: format!("Failed to read {}: {}", path, e),
        })?;

        let sections = parse_tolerant(&content, label, &path);

        let mut profiles = HashMap::new();
        for (section_name, fields) in &sections {
            if let Some(profile_name) = extract_profile_name(section_name) {
                profiles.insert(profile_name, section_to_profile(section_name, fields));
            }
        }

        tracing::debug!("Loaded {} profiles from {}", profiles.len(), label);
        Ok(profiles)
    }

    fn load_config_file(path: &str) -> Result<HashMap<String, Profile>, AwswitError> {
        Self::load_ini_file(path, "Config file", |section_name| {
            if let Some(rest) = section_name.strip_prefix("profile ") {
                Some(rest.to_string())
            } else if section_name == "default" {
                Some("default".to_string())
            } else {
                None // sso-session, services, etc. — not profiles
            }
        })
    }

    fn load_credentials_file(path: &str) -> Result<HashMap<String, Profile>, AwswitError> {
        Self::load_ini_file(path, "Credentials file", |section_name| {
            Some(section_name.to_string())
        })
    }

    /// Cheap path: walk `config_path` and return just the profile names,
    /// skipping `[sso-session …]` / `[services …]` / other non-profile
    /// sections and never reading `~/.aws/credentials`, history, or the
    /// SSO cache.
    ///
    /// Used by tab-completion and any other latency-critical caller.
    /// Returns an empty vec if the file is missing or unreadable — at the
    /// shell-prompt level, we should never make tab-completion hang because
    /// of a transient I/O issue.
    pub fn fast_profile_names(config_path: &str) -> Vec<String> {
        let expanded = shellexpand::tilde(config_path).to_string();
        let Ok(content) = fs::read_to_string(&expanded) else {
            return Vec::new();
        };

        let mut names = Vec::new();
        for raw in content.lines() {
            let line = raw.trim();
            let Some(rest) = line.strip_prefix('[') else {
                continue;
            };
            let Some(name) = rest.strip_suffix(']') else {
                continue;
            };
            let name = name.trim();
            if name == "default" {
                names.push("default".to_string());
            } else if let Some(p) = name.strip_prefix("profile ") {
                names.push(p.to_string());
            }
            // sso-session / services / other sections: ignored.
        }
        names.sort();
        names.dedup();
        names
    }

    /// Merge config and credentials profiles.
    ///
    /// Profiles defined only in `~/.aws/credentials` are exposed by name so
    /// the picker can list them; awswit does not read or store the actual
    /// key material — the AWS SDK resolves credentials at runtime.
    pub fn merge_profiles(&self) -> HashMap<String, Profile> {
        let mut merged: HashMap<String, Profile> = self.config_profiles.clone();
        for (name, cred_profile) in &self.credentials_profiles {
            merged
                .entry(name.clone())
                .or_insert_with(|| cred_profile.clone());
        }
        merged
    }
}

/// Project an INI section's well-known keys onto a `Profile`.
fn section_to_profile(section: &str, fields: &HashMap<String, String>) -> Profile {
    let get = |k: &str| fields.get(k).cloned();
    Profile {
        name: section
            .strip_prefix("profile ")
            .unwrap_or(section)
            .to_string(),
        role_arn: get("role_arn"),
        source_profile: get("source_profile"),
        credential_source: get("credential_source"),
        mfa_serial: get("mfa_serial"),
        region: get("region"),
        sso_start_url: get("sso_start_url"),
        sso_region: get("sso_region"),
        sso_account_id: get("sso_account_id"),
        sso_role_name: get("sso_role_name"),
    }
}

/// Parse INI-style content tolerantly: skip malformed headers and keys with
/// a warn-level log, keep everything else.
fn parse_tolerant(
    content: &str,
    label: &str,
    path: &str,
) -> HashMap<String, HashMap<String, String>> {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current: Option<String> = None;

    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim_start_matches(['\t', ' ']);
        let line = strip_comment(line);
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix('[') {
            if let Some(name) = rest.strip_suffix(']') {
                let name = name.trim();
                if name.is_empty() {
                    tracing::warn!(
                        "{} {}:{} ignored empty section header",
                        label,
                        path,
                        idx + 1
                    );
                    current = None;
                } else {
                    current = Some(name.to_string());
                    sections.entry(name.to_string()).or_default();
                }
            } else {
                tracing::warn!(
                    "{} {}:{} ignored malformed section header: {:?}",
                    label,
                    path,
                    idx + 1,
                    raw
                );
                current = None;
            }
        } else if let Some(eq) = line.find('=') {
            let key = line[..eq].trim();
            let value = line[eq + 1..].trim();
            if key.is_empty() {
                continue;
            }
            if let Some(sec) = &current {
                sections
                    .entry(sec.clone())
                    .or_default()
                    .insert(key.to_string(), value.to_string());
            } else {
                tracing::warn!(
                    "{} {}:{} key=value outside any section: {:?}",
                    label,
                    path,
                    idx + 1,
                    raw
                );
            }
        } else {
            tracing::debug!("{} {}:{} ignored line: {:?}", label, path, idx + 1, raw);
        }
    }

    sections
}

fn strip_comment(line: &str) -> &str {
    // AWS config supports full-line `;` and `#` comments. Inline comments
    // are technically allowed too, but we conservatively strip only when
    // the comment marker appears after whitespace — `key = value#nope` is
    // an unusual but valid value.
    let mut last = line;
    if let Some(idx) = last.find(" ;").or_else(|| last.find(" #")) {
        last = &last[..idx];
    }
    if last.starts_with(';') || last.starts_with('#') {
        return "";
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_config() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"
[default]
region = us-east-1

[profile dev]
role_arn = arn:aws:iam::123456789012:role/DevRole
source_profile = default
mfa_serial = arn:aws:iam::123456789012:mfa/user
region = us-west-2

[profile prod]
role_arn = arn:aws:iam::987654321098:role/ProdRole
source_profile = dev
external_id = abc123
"#
        )
        .unwrap();
        file
    }

    fn create_temp_credentials() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#
        )
        .unwrap();
        file
    }

    #[test]
    fn loads_well_formed_config() {
        let cfg = create_temp_config();
        let creds = create_temp_credentials();
        let f =
            AwsFiles::load(cfg.path().to_str().unwrap(), creds.path().to_str().unwrap()).unwrap();
        assert!(f.config_profiles.contains_key("dev"));
        assert!(f.config_profiles.contains_key("prod"));
        let dev = &f.config_profiles["dev"];
        assert_eq!(
            dev.role_arn.as_deref(),
            Some("arn:aws:iam::123456789012:role/DevRole")
        );
    }

    #[test]
    fn merge_includes_credentials_only_profiles() {
        let cfg = create_temp_config();
        let creds = create_temp_credentials();
        let merged = AwsFiles::load(cfg.path().to_str().unwrap(), creds.path().to_str().unwrap())
            .unwrap()
            .merge_profiles();
        assert!(merged.contains_key("default"));
    }

    #[test]
    fn missing_files_yield_empty_set() {
        let f = AwsFiles::load("/nonexistent/foo", "/nonexistent/bar").unwrap();
        assert!(f.config_profiles.is_empty());
        assert!(f.credentials_profiles.is_empty());
    }

    #[test]
    fn malformed_section_header_is_skipped_not_fatal() {
        let mut cfg = NamedTempFile::new().unwrap();
        write!(
            cfg,
            "[profile good]
region = us-east-1

[profile bad
region = whatever

[profile other]
region = eu-west-1
"
        )
        .unwrap();
        let creds = NamedTempFile::new().unwrap();

        let f = AwsFiles::load(cfg.path().to_str().unwrap(), creds.path().to_str().unwrap())
            .expect("must not error on malformed section");
        // Good and other should be present; the bad section's region went
        // nowhere because no section was current when its `region =` line ran.
        assert!(f.config_profiles.contains_key("good"));
        assert!(f.config_profiles.contains_key("other"));
        assert_eq!(
            f.config_profiles["good"].region.as_deref(),
            Some("us-east-1")
        );
    }

    #[test]
    fn line_comments_are_stripped() {
        let mut cfg = NamedTempFile::new().unwrap();
        write!(
            cfg,
            "; pre-section comment
[profile commented]
# region = us-east-1
region = us-east-2  ; inline
"
        )
        .unwrap();
        let creds = NamedTempFile::new().unwrap();
        let f =
            AwsFiles::load(cfg.path().to_str().unwrap(), creds.path().to_str().unwrap()).unwrap();
        assert_eq!(
            f.config_profiles["commented"].region.as_deref(),
            Some("us-east-2")
        );
    }

    #[test]
    fn sso_session_sections_are_ignored() {
        let mut cfg = NamedTempFile::new().unwrap();
        write!(
            cfg,
            "[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = us-east-1

[profile dev]
region = us-west-2
"
        )
        .unwrap();
        let creds = NamedTempFile::new().unwrap();
        let f =
            AwsFiles::load(cfg.path().to_str().unwrap(), creds.path().to_str().unwrap()).unwrap();
        assert!(f.config_profiles.contains_key("dev"));
        assert!(!f.config_profiles.contains_key("corp"));
        assert!(!f.config_profiles.contains_key("sso-session corp"));
    }

    #[test]
    fn empty_section_header_is_skipped() {
        let mut cfg = NamedTempFile::new().unwrap();
        write!(
            cfg,
            "[]
region = us-east-1

[profile dev]
region = us-west-2
"
        )
        .unwrap();
        let creds = NamedTempFile::new().unwrap();
        let f =
            AwsFiles::load(cfg.path().to_str().unwrap(), creds.path().to_str().unwrap()).unwrap();
        assert!(f.config_profiles.contains_key("dev"));
    }
}
