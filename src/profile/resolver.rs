use std::collections::HashMap;
use std::pin::Pin;
use std::future::Future;

use crate::aws::credentials::Credentials;
use crate::aws::sts::StsClient;
use crate::cache::manager::CacheManager;
use crate::config::AwswitConfig;
use crate::error::{AwswitError, Result};
use crate::profile::types::{Profile, ProfileType};

const MAX_ROLE_CHAIN_DEPTH: usize = 10;

/// Resolves a profile to AWS credentials
pub struct ProfileResolver {
    profiles: HashMap<String, Profile>,
    cache_manager: CacheManager,
    config: AwswitConfig,
    force_refresh: bool,
}

impl ProfileResolver {
    pub fn new(
        profiles: HashMap<String, Profile>,
        cache_manager: CacheManager,
        config: AwswitConfig,
        force_refresh: bool,
    ) -> Self {
        Self {
            profiles,
            cache_manager,
            config,
            force_refresh,
        }
    }

    /// Resolve a profile by name to credentials
    pub async fn resolve(
        &self,
        profile_name: &str,
        mfa_token: Option<&str>,
    ) -> Result<Credentials> {
        let profile = self
            .profiles
            .get(profile_name)
            .ok_or_else(|| AwswitError::profile_not_found(profile_name))?;

        self.resolve_profile(profile, mfa_token, 0).await
    }

    /// Resolve a profile to credentials, handling role chains recursively
    fn resolve_profile<'a>(
        &'a self,
        profile: &'a Profile,
        mfa_token: Option<&'a str>,
        depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials>> + Send + 'a>> {
        Box::pin(self.resolve_profile_inner(profile, mfa_token, depth))
    }

    async fn resolve_profile_inner(
        &self,
        profile: &Profile,
        mfa_token: Option<&str>,
        depth: usize,
    ) -> Result<Credentials> {
        if depth >= MAX_ROLE_CHAIN_DEPTH {
            return Err(AwswitError::role_chain_too_deep(format!(
                "Chain depth exceeded {} for profile: {}",
                MAX_ROLE_CHAIN_DEPTH, profile.name
            )));
        }

        // Check cache first (unless force refresh)
        if !self.force_refresh {
            if let Some(cached) = self.cache_manager.get(&profile.name)? {
                if !cached.is_expired() {
                    tracing::debug!("Using cached credentials for {}", profile.name);
                    return Ok(cached);
                }
            }
        }

        let credentials = match profile.profile_type() {
            ProfileType::User => self.resolve_user_profile(profile, mfa_token).await?,
            ProfileType::Role => self.resolve_role_profile(profile, mfa_token, depth).await?,
            ProfileType::CredentialProcess => self.resolve_credential_process(profile).await?,
            ProfileType::CredentialSource => {
                self.resolve_credential_source(profile, mfa_token, depth)
                    .await?
            }
            ProfileType::Sso => {
                return Err(AwswitError::Other(
                    "SSO profiles are not fully supported yet. Use 'aws sso login' first."
                        .to_string(),
                ));
            }
        };

        // Cache the credentials
        self.cache_manager.put(&profile.name, &credentials)?;

        Ok(credentials)
    }

    /// Resolve a user profile (direct credentials or with session token for MFA)
    async fn resolve_user_profile(
        &self,
        profile: &Profile,
        mfa_token: Option<&str>,
    ) -> Result<Credentials> {
        if !profile.has_credentials() {
            return Err(AwswitError::config_file_error(format!(
                "Profile '{}' has no credentials configured",
                profile.name
            )));
        }

        let base_creds = Credentials {
            access_key_id: profile.aws_access_key_id.clone().unwrap(),
            secret_access_key: profile.aws_secret_access_key.clone().unwrap(),
            session_token: profile.aws_session_token.clone(),
            expiration: None,
            region: profile.region.clone().or_else(|| self.config.region.clone()),
        };

        // If MFA is required, get session token
        if profile.requires_mfa() {
            let mfa_serial = profile.mfa_serial.as_ref().unwrap();
            let token = mfa_token
                .ok_or_else(|| AwswitError::mfa_required(mfa_serial.clone()))?;

            let sts = StsClient::new(&base_creds).await;
            let duration = self.config.debug.session_token_duration as i32;
            return sts
                .get_session_token(mfa_serial, token, duration, base_creds.region.as_deref())
                .await;
        }

        Ok(base_creds)
    }

    /// Resolve a role profile by first resolving the source profile then assuming the role
    async fn resolve_role_profile(
        &self,
        profile: &Profile,
        mfa_token: Option<&str>,
        depth: usize,
    ) -> Result<Credentials> {
        let role_arn = profile.role_arn.as_ref().unwrap();

        // Resolve source profile credentials
        let source_creds = if let Some(ref source_name) = profile.source_profile {
            let source = self
                .profiles
                .get(source_name)
                .ok_or_else(|| AwswitError::profile_not_found(source_name))?;
            self.resolve_profile(source, mfa_token, depth + 1).await?
        } else if profile.has_credentials() {
            // Profile has its own credentials
            Credentials {
                access_key_id: profile.aws_access_key_id.clone().unwrap(),
                secret_access_key: profile.aws_secret_access_key.clone().unwrap(),
                session_token: profile.aws_session_token.clone(),
                expiration: None,
                region: profile.region.clone(),
            }
        } else {
            return Err(AwswitError::config_file_error(format!(
                "Role profile '{}' has no source_profile or direct credentials",
                profile.name
            )));
        };

        let sts = StsClient::new(&source_creds).await;

        let session_name = profile
            .role_session_name
            .clone()
            .or_else(|| self.config.role_session_name.clone())
            .unwrap_or_else(|| format!("awswit-{}", profile.name));

        let duration = profile
            .duration_seconds
            .unwrap_or(self.config.role_duration);

        let region = profile
            .region
            .clone()
            .or_else(|| self.config.region.clone());

        // If source profile does NOT have MFA but this role profile does
        let role_mfa_token = if profile.requires_mfa()
            && profile
                .source_profile
                .as_ref()
                .and_then(|sp| self.profiles.get(sp))
                .map(|sp| !sp.requires_mfa())
                .unwrap_or(true)
        {
            let mfa_serial = profile.mfa_serial.as_ref().unwrap();
            Some((
                mfa_serial.clone(),
                mfa_token
                    .ok_or_else(|| AwswitError::mfa_required(mfa_serial.clone()))?
                    .to_string(),
            ))
        } else {
            None
        };

        sts.assume_role(
            role_arn,
            &session_name,
            profile.external_id.as_deref(),
            duration,
            role_mfa_token
                .as_ref()
                .map(|(serial, _)| serial.as_str()),
            role_mfa_token
                .as_ref()
                .map(|(_, token)| token.as_str()),
            region.as_deref(),
        )
        .await
    }

    /// Resolve a credential_process profile.
    /// Uses direct argv execution to avoid shell injection risks.
    async fn resolve_credential_process(&self, profile: &Profile) -> Result<Credentials> {
        let command = profile.credential_process.as_ref().unwrap();

        // Parse command into program + arguments using shell-style word splitting
        let parts = shell_words_split(command)?;
        if parts.is_empty() {
            return Err(AwswitError::config_file_error(format!(
                "Empty credential_process command for profile '{}'",
                profile.name
            )));
        }

        let output = tokio::process::Command::new(&parts[0])
            .args(&parts[1..])
            .output()
            .await
            .map_err(|e| {
                AwswitError::config_file_error(format!(
                    "Failed to run credential_process '{}': {}",
                    command, e
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AwswitError::config_file_error(format!(
                "credential_process '{}' failed: {}",
                command, stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let cred_output: serde_json::Value =
            serde_json::from_str(&stdout).map_err(|e| {
                AwswitError::config_file_error(format!(
                    "Failed to parse credential_process output: {}",
                    e
                ))
            })?;

        let access_key = cred_output["AccessKeyId"]
            .as_str()
            .ok_or_else(|| {
                AwswitError::config_file_error("Missing AccessKeyId in credential_process output")
            })?
            .to_string();

        let secret_key = cred_output["SecretAccessKey"]
            .as_str()
            .ok_or_else(|| {
                AwswitError::config_file_error(
                    "Missing SecretAccessKey in credential_process output",
                )
            })?
            .to_string();

        let session_token = cred_output["SessionToken"]
            .as_str()
            .map(|s| s.to_string());

        let expiration = cred_output["Expiration"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        Ok(Credentials {
            access_key_id: access_key,
            secret_access_key: secret_key,
            session_token,
            expiration,
            region: profile.region.clone().or_else(|| self.config.region.clone()),
        })
    }

    /// Resolve a credential_source profile
    async fn resolve_credential_source(
        &self,
        profile: &Profile,
        mfa_token: Option<&str>,
        depth: usize,
    ) -> Result<Credentials> {
        let source = profile.credential_source.as_ref().unwrap();

        // Get base credentials from the credential source
        let base_creds = match source.as_str() {
            "Environment" => {
                let access_key = std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
                    AwswitError::config_file_error(
                        "AWS_ACCESS_KEY_ID not set for credential_source=Environment",
                    )
                })?;
                let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
                    AwswitError::config_file_error(
                        "AWS_SECRET_ACCESS_KEY not set for credential_source=Environment",
                    )
                })?;
                let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
                Credentials {
                    access_key_id: access_key,
                    secret_access_key: secret_key,
                    session_token,
                    expiration: None,
                    region: profile.region.clone(),
                }
            }
            _ => {
                return Err(AwswitError::config_file_error(format!(
                    "Unsupported credential_source: {}. Supported: Environment",
                    source
                )));
            }
        };

        // If there's a role_arn, assume it using the source credentials
        if let Some(ref role_arn) = profile.role_arn {
            let sts = StsClient::new(&base_creds).await;
            let session_name = profile
                .role_session_name
                .clone()
                .unwrap_or_else(|| format!("awswit-{}", profile.name));
            let duration = profile
                .duration_seconds
                .unwrap_or(self.config.role_duration);

            let role_mfa = if profile.requires_mfa() {
                let mfa_serial = profile.mfa_serial.as_ref().unwrap();
                Some((
                    mfa_serial.clone(),
                    mfa_token
                        .ok_or_else(|| AwswitError::mfa_required(mfa_serial.clone()))?
                        .to_string(),
                ))
            } else {
                None
            };

            return sts
                .assume_role(
                    role_arn,
                    &session_name,
                    profile.external_id.as_deref(),
                    duration,
                    role_mfa.as_ref().map(|(s, _)| s.as_str()),
                    role_mfa.as_ref().map(|(_, t)| t.as_str()),
                    profile.region.as_deref(),
                )
                .await;
        }

        let _ = (mfa_token, depth);

        Ok(base_creds)
    }

    /// Get a profile by name
    pub fn get_profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    /// Get all profile names
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.profiles.keys().cloned().collect();
        names.sort();
        names
    }
}

/// Split a command string into argv components, respecting quotes.
/// This avoids passing untrusted strings through `sh -c`.
fn shell_words_split(command: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for ch in command.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quote => {
                escape_next = true;
            }
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    if in_single_quote || in_double_quote {
        return Err(AwswitError::config_file_error(format!(
            "Unterminated quote in credential_process command: {}",
            command
        )));
    }

    Ok(parts)
}
