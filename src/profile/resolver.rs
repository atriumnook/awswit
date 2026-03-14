use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::aws::{Credentials, StsOperations};
use crate::cache::CredentialStore;
use crate::cli::Args;
use crate::config::AwswitConfig;
use crate::error::AwswitError;
use crate::profile::Profile;

/// Build a consistent cache key for MFA sessions.
/// Uses hex-encoded mfa_serial for deterministic, cross-version stability
/// (DefaultHasher is not guaranteed to be stable across Rust versions).
fn mfa_cache_key(access_key_id: &str, mfa_serial: &str) -> String {
    let hex: String = mfa_serial
        .as_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    format!("v1-session-{}-{}", access_key_id, hex)
}

/// Resolves profile credentials, handling role chains and MFA
pub struct ProfileResolver<'a> {
    profiles: &'a HashMap<String, Profile>,
    config: &'a AwswitConfig,
}

impl<'a> ProfileResolver<'a> {
    pub fn new(profiles: &'a HashMap<String, Profile>, config: &'a AwswitConfig) -> Self {
        Self { profiles, config }
    }

    /// Resolve credentials for a profile, handling role chains
    pub async fn resolve_credentials(
        &self,
        profile_name: &str,
        args: &Args,
        sts_client: &dyn StsOperations,
        cache_manager: &dyn CredentialStore,
    ) -> Result<Credentials, AwswitError> {
        tracing::info!("Resolving credentials for profile: {}", profile_name);

        // Check if we're using --role-arn directly
        if let Some(ref role_arn) = args.resolve_role_arn() {
            return self.assume_role_from_cli(role_arn, args, sts_client).await;
        }

        // Get the target profile
        let target_profile =
            self.profiles
                .get(profile_name)
                .ok_or_else(|| AwswitError::ProfileNotFound {
                    name: profile_name.to_string(),
                })?;

        // If it's a role profile, resolve the chain — even if credential_process is set,
        // because the credential_process should be used as a source credential provider
        // within the role chain, not as a shortcut that skips AssumeRole.
        if target_profile.is_role_profile() {
            return self
                .resolve_role_chain(profile_name, args, sts_client, cache_manager)
                .await;
        }

        // Handle credential_process for non-role profiles
        if target_profile.uses_credential_process() {
            return self.get_credentials_from_process(target_profile).await;
        }

        // User profile - get session token if MFA required
        if target_profile.requires_mfa() {
            return self
                .get_session_token_credentials(target_profile, args, sts_client, cache_manager)
                .await;
        }

        // Simple user profile - return the credentials directly
        self.profile_to_credentials(target_profile)
    }

    /// Resolve role duration from args → profile → config, with validation.
    fn resolve_role_duration(
        &self,
        args: &Args,
        profile: Option<&Profile>,
    ) -> Result<Option<i32>, AwswitError> {
        let duration = args
            .role_duration
            .or_else(|| profile.and_then(|p| p.duration_seconds))
            .or_else(|| {
                let rd = self.config.role_duration;
                if rd > 0 { Some(rd) } else { None }
            });
        if let Some(d) = duration {
            crate::cli::validate_role_duration(d)?;
        }
        Ok(duration)
    }

    /// Assume role directly from CLI arguments
    async fn assume_role_from_cli(
        &self,
        role_arn: &str,
        args: &Args,
        sts_client: &dyn StsOperations,
    ) -> Result<Credentials, AwswitError> {
        tracing::info!("Assuming role from CLI: {}", role_arn);

        // Get source credentials
        let source_credentials = if let Some(ref source_profile_name) = args.source_profile {
            let source_profile = self.profiles.get(source_profile_name).ok_or_else(|| {
                AwswitError::SourceProfileNotFound {
                    name: source_profile_name.clone(),
                }
            })?;

            // Handle credential_process on the source profile
            if source_profile.uses_credential_process() {
                Some(self.get_credentials_from_process(source_profile).await?)
            } else if source_profile.requires_mfa() {
                tracing::warn!(
                    "Source profile requires MFA — MFA prompts are not supported with --role-arn"
                );
                Some(self.profile_to_credentials(source_profile)?)
            } else {
                Some(self.profile_to_credentials(source_profile)?)
            }
        } else {
            None
        };

        let session_name = args.get_session_name("awswit-cli-role");
        let role_duration = self.resolve_role_duration(args, None)?;
        let region = args.region.clone().or_else(|| self.config.region.clone());

        sts_client
            .assume_role(
                source_credentials.as_ref(),
                role_arn,
                &session_name,
                args.external_id.as_deref(),
                region.as_deref(),
                role_duration,
                None, // mfa_serial
                None, // mfa_token
            )
            .await
    }

    /// Resolve a role chain and get final credentials
    async fn resolve_role_chain(
        &self,
        profile_name: &str,
        args: &Args,
        sts_client: &dyn StsOperations,
        cache_manager: &dyn CredentialStore,
    ) -> Result<Credentials, AwswitError> {
        // Get the role chain (with cycle detection via visited set)
        let chain = self.get_role_chain(profile_name)?;
        tracing::debug!(
            "Role chain: {:?}",
            chain.iter().map(|p| &p.name).collect::<Vec<_>>()
        );

        let target_profile =
            self.profiles
                .get(profile_name)
                .ok_or_else(|| AwswitError::ProfileNotFound {
                    name: profile_name.to_string(),
                })?;

        // Get source credentials
        let source_credentials = self.get_source_credentials(&chain).await?;

        // Determine role duration and validate against AWS STS limits
        let role_duration = self.resolve_role_duration(args, Some(target_profile))?;

        // Check for MFA requirement
        let mfa_serial = self.get_mfa_serial_for_chain(&chain);

        // If MFA is required and role_duration > 3600, we need special handling
        if mfa_serial.is_some() && role_duration.map(|d| d > 3600).unwrap_or(false) {
            return self
                .assume_role_with_mfa_large_duration(
                    target_profile,
                    &source_credentials,
                    args,
                    sts_client,
                    role_duration,
                    &mfa_serial,
                )
                .await;
        }

        // If MFA required, get session token first
        let assume_source = if let Some(ref mfa_serial) = mfa_serial {
            self.get_session_token_with_mfa(
                &source_credentials,
                mfa_serial,
                args,
                sts_client,
                cache_manager,
            )
            .await?
        } else {
            source_credentials
        };

        // Iterate through the role chain, assuming each role in sequence.
        // The last profile in `chain` is the target; intermediate hops use profile settings.
        let mut current_creds = assume_source;
        for (i, role_profile) in chain.iter().enumerate() {
            // Skip the first element if it's not a role profile (it's the source)
            if !role_profile.is_role_profile() {
                continue;
            }

            let is_final_hop = i == chain.len() - 1;

            let role_arn =
                role_profile
                    .role_arn
                    .as_ref()
                    .ok_or_else(|| AwswitError::InvalidProfile {
                        profile_name: role_profile.name.clone(),
                        message: "missing role_arn".to_string(),
                    })?;

            let (session_name, region, external_id, hop_duration) = if is_final_hop {
                // Final hop: apply CLI args overrides
                let session_name = crate::cli::sanitize_session_name(
                    &args
                        .session_name
                        .clone()
                        .or(role_profile.role_session_name.clone())
                        .or(self.config.role_session_name.clone())
                        .unwrap_or_else(|| profile_name.to_string()),
                );
                let region = args
                    .region
                    .clone()
                    .or(role_profile.region.clone())
                    .or(self.config.region.clone());
                let external_id = args
                    .external_id
                    .clone()
                    .or(role_profile.external_id.clone());
                (session_name, region, external_id, role_duration)
            } else {
                // Intermediate hop: use profile settings only
                let session_name = crate::cli::sanitize_session_name(
                    &role_profile
                        .role_session_name
                        .clone()
                        .unwrap_or_else(|| role_profile.name.clone()),
                );
                let region = role_profile.region.clone();
                let external_id = role_profile.external_id.clone();
                let hop_duration = role_profile.duration_seconds;
                (session_name, region, external_id, hop_duration)
            };

            current_creds = sts_client
                .assume_role(
                    Some(&current_creds),
                    role_arn,
                    &session_name,
                    external_id.as_deref(),
                    region.as_deref(),
                    hop_duration,
                    None,
                    None,
                )
                .await?;
        }

        Ok(current_creds)
    }

    /// Get the role chain for a profile, detecting cycles immediately via HashSet
    fn get_role_chain(&self, profile_name: &str) -> Result<Vec<&Profile>, AwswitError> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        let mut current_name = profile_name.to_string();

        loop {
            // Check for cycles before doing anything else
            if !visited.insert(current_name.clone()) {
                return Err(AwswitError::RoleChainCycle {
                    chain: current_name,
                });
            }

            let profile =
                self.profiles
                    .get(&current_name)
                    .ok_or_else(|| AwswitError::ProfileNotFound {
                        name: current_name.clone(),
                    })?;

            chain.push(profile);

            // If this profile has a source_profile that's also a role, continue the chain
            if let Some(ref source_name) = profile.source_profile
                && let Some(source_profile) = self.profiles.get(source_name)
                && source_profile.is_role_profile()
            {
                current_name = source_name.clone();
                continue;
            }

            // End of chain
            break;
        }

        // Reverse so first element is the source
        chain.reverse();
        Ok(chain)
    }

    /// Get source credentials for a role chain
    async fn get_source_credentials(&self, chain: &[&Profile]) -> Result<Credentials, AwswitError> {
        // Get the first profile in the chain (source)
        let source_profile = chain.first().ok_or_else(|| AwswitError::ValidationError {
            message: "Empty role chain".to_string(),
        })?;

        // Check credential_source
        if let Some(ref cred_source) = source_profile.credential_source {
            return self.get_credentials_from_source(cred_source).await;
        }

        // Check for source_profile
        if let Some(ref source_name) = source_profile.source_profile {
            let user_profile = self.profiles.get(source_name).ok_or_else(|| {
                AwswitError::SourceProfileNotFound {
                    name: source_name.clone(),
                }
            })?;

            // Handle credential_process on the user profile
            if user_profile.uses_credential_process() {
                return self.get_credentials_from_process(user_profile).await;
            }

            return self.profile_to_credentials(user_profile);
        }

        // Check if the source profile itself uses credential_process
        if source_profile.uses_credential_process() {
            return self.get_credentials_from_process(source_profile).await;
        }

        // Use the profile's own credentials
        self.profile_to_credentials(source_profile)
    }

    /// Get MFA serial for a role chain.
    ///
    /// Checks every profile in the chain, then walks the full source_profile
    /// ancestry of the chain root so that grandparent (or deeper) MFA serials
    /// are detected.
    fn get_mfa_serial_for_chain(&self, chain: &[&Profile]) -> Option<String> {
        // Check each profile already in the chain
        for profile in chain {
            if let Some(ref mfa_serial) = profile.mfa_serial {
                return Some(mfa_serial.clone());
            }
        }

        // Walk the source_profile ancestry from the chain root (first element,
        // which is the deepest role profile after reversal) to find MFA in
        // non-role ancestors that are not part of the chain itself.
        if let Some(root) = chain.first() {
            let mut current_source = root.source_profile.as_deref();
            let mut visited = HashSet::new();
            while let Some(source_name) = current_source {
                if !visited.insert(source_name) {
                    break; // cycle guard
                }
                if let Some(source_profile) = self.profiles.get(source_name) {
                    if let Some(ref mfa_serial) = source_profile.mfa_serial {
                        return Some(mfa_serial.clone());
                    }
                    current_source = source_profile.source_profile.as_deref();
                } else {
                    break;
                }
            }
        }

        None
    }

    /// Get session token with MFA
    async fn get_session_token_with_mfa(
        &self,
        source_credentials: &Credentials,
        mfa_serial: &str,
        args: &Args,
        sts_client: &dyn StsOperations,
        cache_manager: &dyn CredentialStore,
    ) -> Result<Credentials, AwswitError> {
        // Check cache first (unless force refresh)
        if !args.force_refresh {
            let cache_key = mfa_cache_key(&source_credentials.access_key_id, mfa_serial);
            if let Some(cached) = cache_manager.get(&cache_key)?
                && !cached.is_expired()
            {
                tracing::info!("Using cached MFA session credentials");
                return Ok(cached);
            }
        }

        // Need to get new session token
        let token = self.get_mfa_token(args)?;

        let session = sts_client
            .get_session_token(
                source_credentials,
                Some(mfa_serial),
                Some(&token),
                self.config.session_token_duration,
            )
            .await?;

        // Cache the session
        let cache_key = mfa_cache_key(&source_credentials.access_key_id, mfa_serial);
        cache_manager.set(&cache_key, &session)?;

        Ok(session)
    }

    /// Get MFA token from args or prompt user
    fn get_mfa_token(&self, args: &Args) -> Result<String, AwswitError> {
        if let Some(ref token) = args.mfa_token {
            validate_mfa_token(token)?;
            return Ok(token.clone());
        }

        // In non-interactive mode (e.g., exec subcommand), we cannot prompt for MFA
        if args.no_interactive {
            return Err(AwswitError::MfaTokenRequired);
        }

        // Prompt user
        use dialoguer::Input;
        let token: String = Input::new()
            .with_prompt("Enter MFA token")
            .interact_text()
            .map_err(|_| AwswitError::MfaTokenRequired)?;

        validate_mfa_token(&token)?;
        Ok(token)
    }

    /// Get session token credentials for a user profile with MFA
    async fn get_session_token_credentials(
        &self,
        profile: &Profile,
        args: &Args,
        sts_client: &dyn StsOperations,
        cache_manager: &dyn CredentialStore,
    ) -> Result<Credentials, AwswitError> {
        let source_credentials = self.profile_to_credentials(profile)?;
        let mfa_serial = profile
            .mfa_serial
            .as_ref()
            .ok_or(AwswitError::MfaTokenRequired)?;

        self.get_session_token_with_mfa(
            &source_credentials,
            mfa_serial,
            args,
            sts_client,
            cache_manager,
        )
        .await
    }

    /// Assume role with MFA when duration > 1 hour (skip get_session_token)
    async fn assume_role_with_mfa_large_duration(
        &self,
        profile: &Profile,
        source_credentials: &Credentials,
        args: &Args,
        sts_client: &dyn StsOperations,
        role_duration: Option<i32>,
        mfa_serial: &Option<String>,
    ) -> Result<Credentials, AwswitError> {
        if args.auto_refresh && role_duration.map(|d| d > 3600).unwrap_or(false) {
            return Err(AwswitError::AutoRefreshDurationLimit);
        }

        let mfa_serial = mfa_serial.clone();
        let (mfa_serial_val, mfa_token_val) = if let Some(ref serial) = mfa_serial {
            let t = self.get_mfa_token(args)?;
            (Some(serial.clone()), Some(t))
        } else {
            (None, None)
        };

        let role_arn = profile
            .role_arn
            .as_ref()
            .ok_or_else(|| AwswitError::InvalidProfile {
                profile_name: profile.name.clone(),
                message: "missing role_arn".to_string(),
            })?;

        let session_name = crate::cli::sanitize_session_name(
            &args
                .session_name
                .clone()
                .or(profile.role_session_name.clone())
                .or(self.config.role_session_name.clone())
                .unwrap_or_else(|| profile.name.clone()),
        );

        let region = args
            .region
            .clone()
            .or(profile.region.clone())
            .or(self.config.region.clone());

        sts_client
            .assume_role(
                Some(source_credentials),
                role_arn,
                &session_name,
                profile.external_id.as_deref(),
                region.as_deref(),
                role_duration,
                mfa_serial_val.as_deref(),
                mfa_token_val.as_deref(),
            )
            .await
    }

    /// Get credentials from credential_source
    async fn get_credentials_from_source(&self, source: &str) -> Result<Credentials, AwswitError> {
        match source {
            "Environment" => {
                let access_key =
                    std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| AwswitError::EnvError {
                        message: "AWS_ACCESS_KEY_ID not set".to_string(),
                    })?;
                let secret_key =
                    std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| AwswitError::EnvError {
                        message: "AWS_SECRET_ACCESS_KEY not set".to_string(),
                    })?;
                let session_token = std::env::var("AWS_SESSION_TOKEN").ok();

                Ok(Credentials {
                    access_key_id: access_key,
                    secret_access_key: secret_key,
                    session_token,
                    expiration: None,
                    region: std::env::var("AWS_REGION")
                        .ok()
                        .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok()),
                })
            }
            "Ec2InstanceMetadata" | "EcsContainer" => {
                tracing::info!("Using AWS SDK default credential chain for {}", source);
                let sdk_config =
                    aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
                let provider = sdk_config.credentials_provider().ok_or_else(|| {
                    AwswitError::InvalidCredentialSource {
                        name: format!("No credentials provider available for {}", source),
                    }
                })?;
                use aws_credential_types::provider::ProvideCredentials;
                let creds = provider.provide_credentials().await.map_err(|e| {
                    AwswitError::InvalidCredentialSource {
                        name: format!("Failed to get credentials from {}: {}", source, e),
                    }
                })?;
                Ok(Credentials {
                    access_key_id: creds.access_key_id().to_string(),
                    secret_access_key: creds.secret_access_key().to_string(),
                    session_token: creds.session_token().map(|s| s.to_string()),
                    expiration: creds.expiry().and_then(|e| {
                        e.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| {
                            chrono::DateTime::<chrono::Utc>::from_timestamp(
                                d.as_secs() as i64,
                                d.subsec_nanos(),
                            )
                        })
                    }),
                    region: None,
                })
            }
            _ => Err(AwswitError::InvalidCredentialSource {
                name: source.to_string(),
            }),
        }
    }

    /// Get credentials from credential_process
    async fn get_credentials_from_process(
        &self,
        profile: &Profile,
    ) -> Result<Credentials, AwswitError> {
        let command =
            profile
                .credential_process
                .as_ref()
                .ok_or_else(|| AwswitError::InvalidProfile {
                    profile_name: profile.name.clone(),
                    message: "missing credential_process".to_string(),
                })?;

        // TRUST BOUNDARY: credential_process is executed via a shell exactly as
        // specified in the user's AWS config file (~/.aws/config). This matches
        // AWS CLI behavior. The config file is trusted user input — if an attacker
        // can modify it, they already have arbitrary code execution.
        //
        // On Unix, we use `sh -c`; on Windows, `cmd /c`.
        tracing::info!("Running credential_process for profile '{}'", profile.name);

        #[cfg(windows)]
        let mut cmd = tokio::process::Command::new("cmd");
        #[cfg(windows)]
        cmd.arg("/c").arg(command);

        #[cfg(not(windows))]
        let mut cmd = tokio::process::Command::new("sh");
        #[cfg(not(windows))]
        cmd.arg("-c").arg(command);

        let child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| AwswitError::CredentialProcessFailed {
                message: e.to_string(),
            })?;

        let timeout_duration = std::time::Duration::from_secs(30);
        let wait_future = child.wait_with_output();
        tokio::pin!(wait_future);

        let output = tokio::select! {
            result = &mut wait_future => {
                result.map_err(|e| AwswitError::CredentialProcessFailed {
                    message: format!("Failed to wait on credential_process: {}", e),
                })?
            }
            _ = tokio::time::sleep(timeout_duration) => {
                // Timed out — wait_future (owning the child) is dropped here,
                // which kills the child process via tokio::process::Child's Drop impl.
                return Err(AwswitError::CredentialProcessFailed {
                    message: "credential_process timed out after 30 seconds".to_string(),
                });
            }
        };

        if !output.status.success() {
            // Log stderr at debug only - may contain secrets
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!("credential_process stderr: {}", stderr);
            return Err(AwswitError::CredentialProcessFailed {
                message: format!("credential_process exited with status {}", output.status),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let creds: CredentialProcessOutput = serde_json::from_str(&stdout)?;

        if creds.version != 1 {
            return Err(AwswitError::CredentialProcessFailed {
                message: format!(
                    "credential_process returned unsupported Version {}. Only Version 1 is supported.",
                    creds.version
                ),
            });
        }

        let expiration = match creds.expiration {
            Some(ref e) => Some(
                chrono::DateTime::parse_from_rfc3339(e)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|_| AwswitError::CredentialProcessFailed {
                        message: format!("Invalid expiration date format: {}", e),
                    })?,
            ),
            None => None,
        };

        Ok(Credentials {
            access_key_id: creds.access_key_id,
            secret_access_key: creds.secret_access_key,
            session_token: creds.session_token,
            expiration,
            region: profile.region.clone(),
        })
    }

    /// Convert a profile to credentials
    fn profile_to_credentials(&self, profile: &Profile) -> Result<Credentials, AwswitError> {
        let access_key =
            profile
                .aws_access_key_id
                .clone()
                .ok_or_else(|| AwswitError::MissingProfileKey {
                    profile_name: profile.name.clone(),
                    key: "aws_access_key_id".to_string(),
                })?;
        let secret_key = profile.aws_secret_access_key.clone().ok_or_else(|| {
            AwswitError::MissingProfileKey {
                profile_name: profile.name.clone(),
                key: "aws_secret_access_key".to_string(),
            }
        })?;

        Ok(Credentials {
            access_key_id: access_key,
            secret_access_key: secret_key,
            session_token: profile.aws_session_token.clone(),
            expiration: None,
            region: profile.region.clone(),
        })
    }
}

/// Validate MFA token: must be 6-8 ASCII digits.
///
/// Standard TOTP tokens are 6 digits (RFC 6238). AWS also accepts 8-digit
/// tokens from certain hardware MFA devices (e.g., Gemalto tokens).
pub fn validate_mfa_token(token: &str) -> Result<(), AwswitError> {
    if token.len() < 6 || token.len() > 8 {
        return Err(AwswitError::InvalidMfaToken {
            message: format!(
                "MFA token must be 6-8 digits, got {} characters",
                token.len()
            ),
        });
    }
    if !token.chars().all(|c| c.is_ascii_digit()) {
        return Err(AwswitError::InvalidMfaToken {
            message: "MFA token must contain only ASCII digits (0-9)".to_string(),
        });
    }
    Ok(())
}

/// Output format for credential_process
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CredentialProcessOutput {
    #[serde(default = "default_credential_process_version")]
    version: u32,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    expiration: Option<String>,
}

fn default_credential_process_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheManager;

    #[test]
    fn mfa_cache_key_is_consistent() {
        let key1 = mfa_cache_key("AKIAEXAMPLE", "arn:aws:iam::123456789012:mfa/user");
        let key2 = mfa_cache_key("AKIAEXAMPLE", "arn:aws:iam::123456789012:mfa/user");
        assert_eq!(key1, key2);
    }

    #[test]
    fn mfa_cache_key_differs_for_different_inputs() {
        let key1 = mfa_cache_key("AKIAEXAMPLE", "arn:aws:iam::123456789012:mfa/user1");
        let key2 = mfa_cache_key("AKIAEXAMPLE", "arn:aws:iam::123456789012:mfa/user2");
        assert_ne!(key1, key2);

        let key3 = mfa_cache_key("AKIAEXAMPLE1", "arn:aws:iam::123456789012:mfa/user");
        let key4 = mfa_cache_key("AKIAEXAMPLE2", "arn:aws:iam::123456789012:mfa/user");
        assert_ne!(key3, key4);
    }

    #[test]
    fn mfa_cache_key_uses_hex_format() {
        let key = mfa_cache_key("AKIAEXAMPLE", "arn:aws:iam::123456789012:mfa/user");
        assert!(key.starts_with("v1-session-AKIAEXAMPLE-"));
        // Should contain hex-encoded mfa_serial
        assert!(key.contains("61726e3a6177733a69616d"));
    }

    fn make_user_profile(name: &str) -> Profile {
        Profile {
            name: name.to_string(),
            aws_access_key_id: Some("AKIATEST".to_string()),
            aws_secret_access_key: Some("secret".to_string()),
            ..Default::default()
        }
    }

    fn make_role_profile(name: &str, role_arn: &str, source_profile: &str) -> Profile {
        Profile {
            name: name.to_string(),
            role_arn: Some(role_arn.to_string()),
            source_profile: Some(source_profile.to_string()),
            ..Default::default()
        }
    }

    fn default_config() -> AwswitConfig {
        AwswitConfig::default()
    }

    #[test]
    fn role_chain_single_hop() {
        let mut profiles = HashMap::new();
        profiles.insert("base".to_string(), make_user_profile("base"));
        profiles.insert(
            "dev".to_string(),
            make_role_profile("dev", "arn:aws:iam::111:role/Dev", "base"),
        );

        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let chain = resolver.get_role_chain("dev").unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].name, "dev");
    }

    #[test]
    fn role_chain_multi_hop() {
        let mut profiles = HashMap::new();
        profiles.insert("base".to_string(), make_user_profile("base"));
        profiles.insert(
            "hop1".to_string(),
            make_role_profile("hop1", "arn:aws:iam::111:role/Hop1", "base"),
        );
        profiles.insert(
            "hop2".to_string(),
            make_role_profile("hop2", "arn:aws:iam::222:role/Hop2", "hop1"),
        );

        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let chain = resolver.get_role_chain("hop2").unwrap();
        // Chain should be [hop1, hop2] (reversed, source first)
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].name, "hop1");
        assert_eq!(chain[1].name, "hop2");
    }

    #[test]
    fn role_chain_cycle_detection() {
        let mut profiles = HashMap::new();
        profiles.insert(
            "a".to_string(),
            make_role_profile("a", "arn:aws:iam::111:role/A", "b"),
        );
        profiles.insert(
            "b".to_string(),
            make_role_profile("b", "arn:aws:iam::222:role/B", "a"),
        );

        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let result = resolver.get_role_chain("a");
        assert!(result.is_err());
        match result.unwrap_err() {
            AwswitError::RoleChainCycle { .. } => {}
            e => panic!("Expected RoleChainCycle, got: {:?}", e),
        }
    }

    #[test]
    fn role_chain_missing_profile() {
        let mut profiles = HashMap::new();
        profiles.insert(
            "orphan".to_string(),
            make_role_profile("orphan", "arn:aws:iam::111:role/X", "nonexistent"),
        );
        // nonexistent source doesn't exist, but get_role_chain stops when source is not a role profile
        // It should still return the chain with just orphan
        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let chain = resolver.get_role_chain("orphan").unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].name, "orphan");
    }

    #[test]
    fn role_chain_stops_at_user_profile() {
        let mut profiles = HashMap::new();
        profiles.insert("user".to_string(), make_user_profile("user"));
        profiles.insert(
            "role".to_string(),
            make_role_profile("role", "arn:aws:iam::111:role/R", "user"),
        );

        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let chain = resolver.get_role_chain("role").unwrap();
        // Should contain only the role profile, not the user source
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].name, "role");
    }

    #[test]
    fn get_mfa_serial_from_source_profile() {
        let mut profiles = HashMap::new();
        let mut user = make_user_profile("user");
        user.mfa_serial = Some("arn:aws:iam::111:mfa/user".to_string());
        profiles.insert("user".to_string(), user);
        profiles.insert(
            "role".to_string(),
            make_role_profile("role", "arn:aws:iam::111:role/R", "user"),
        );

        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let chain = resolver.get_role_chain("role").unwrap();
        let mfa = resolver.get_mfa_serial_for_chain(&chain);
        assert_eq!(mfa, Some("arn:aws:iam::111:mfa/user".to_string()));
    }

    #[test]
    fn get_mfa_serial_from_grandparent_profile() {
        // grandparent -> parent (user, has MFA) -> child (role)
        // The chain for "child" is just [child], and child.source_profile = "parent".
        // parent is a user profile with MFA, whose source_profile = "grandparent".
        // grandparent has the MFA serial.
        let mut profiles = HashMap::new();
        let mut grandparent = make_user_profile("grandparent");
        grandparent.mfa_serial = Some("arn:aws:iam::111:mfa/gp".to_string());
        profiles.insert("grandparent".to_string(), grandparent);

        // parent is a user profile that chains to grandparent
        let mut parent = make_user_profile("parent");
        parent.source_profile = Some("grandparent".to_string());
        profiles.insert("parent".to_string(), parent);

        // child is a role profile that chains to parent
        profiles.insert(
            "child".to_string(),
            make_role_profile("child", "arn:aws:iam::111:role/C", "parent"),
        );

        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let chain = resolver.get_role_chain("child").unwrap();
        let mfa = resolver.get_mfa_serial_for_chain(&chain);
        assert_eq!(mfa, Some("arn:aws:iam::111:mfa/gp".to_string()));
    }

    #[test]
    fn validate_mfa_token_rejects_spaces() {
        assert!(validate_mfa_token("12 345").is_err());
    }

    #[test]
    fn validate_mfa_token_rejects_fullwidth_digits() {
        // Full-width digits (U+FF10-FF19) should be rejected
        assert!(validate_mfa_token("\u{FF11}\u{FF12}\u{FF13}\u{FF14}\u{FF15}\u{FF16}").is_err());
    }

    #[test]
    fn credential_process_output_parses_full() {
        let json = r#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret","SessionToken":"tok","Expiration":"2099-01-01T00:00:00Z"}"#;
        let output: super::CredentialProcessOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.access_key_id, "AKIA");
        assert_eq!(output.secret_access_key, "secret");
        assert_eq!(output.session_token, Some("tok".to_string()));
        assert_eq!(output.expiration, Some("2099-01-01T00:00:00Z".to_string()));
    }

    #[test]
    fn credential_process_output_parses_minimal() {
        let json = r#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret"}"#;
        let output: super::CredentialProcessOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.access_key_id, "AKIA");
        assert!(output.session_token.is_none());
        assert!(output.expiration.is_none());
    }

    #[test]
    fn credential_process_output_parses_with_version_1() {
        let json = r#"{"Version":1,"AccessKeyId":"AKIA","SecretAccessKey":"secret"}"#;
        let output: super::CredentialProcessOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.version, 1);
        assert_eq!(output.access_key_id, "AKIA");
    }

    #[test]
    fn credential_process_output_defaults_version_to_1() {
        let json = r#"{"AccessKeyId":"AKIA","SecretAccessKey":"secret"}"#;
        let output: super::CredentialProcessOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.version, 1);
    }

    #[test]
    fn credential_process_output_parses_version_2() {
        // Version 2 should parse but be rejected at validation time, not parse time
        let json = r#"{"Version":2,"AccessKeyId":"AKIA","SecretAccessKey":"secret"}"#;
        let output: super::CredentialProcessOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.version, 2);
    }

    #[test]
    fn credential_process_output_rejects_missing_access_key() {
        let json = r#"{"SecretAccessKey":"secret"}"#;
        let result = serde_json::from_str::<super::CredentialProcessOutput>(json);
        assert!(result.is_err());
    }

    #[test]
    fn credential_process_output_rejects_invalid_json() {
        let result = serde_json::from_str::<super::CredentialProcessOutput>("not json");
        assert!(result.is_err());
    }

    #[test]
    fn validate_mfa_token_accepts_6_digits() {
        assert!(validate_mfa_token("123456").is_ok());
    }

    #[test]
    fn validate_mfa_token_accepts_8_digits() {
        assert!(validate_mfa_token("12345678").is_ok());
    }

    #[test]
    fn validate_mfa_token_rejects_5_digits() {
        assert!(validate_mfa_token("12345").is_err());
    }

    #[test]
    fn validate_mfa_token_rejects_9_digits() {
        assert!(validate_mfa_token("123456789").is_err());
    }

    #[test]
    fn validate_mfa_token_rejects_letters() {
        assert!(validate_mfa_token("abc456").is_err());
    }

    // --- MockStsClient for trait-based testing ---

    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Debug, Clone)]
    struct AssumeRoleCall {
        role_arn: String,
        session_name: String,
        external_id: Option<String>,
        region: Option<String>,
        duration_seconds: Option<i32>,
    }

    struct MockStsClient {
        assume_role_calls: Mutex<Vec<AssumeRoleCall>>,
    }

    impl MockStsClient {
        fn new() -> Self {
            Self {
                assume_role_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl StsOperations for MockStsClient {
        async fn assume_role(
            &self,
            _source_credentials: Option<&Credentials>,
            role_arn: &str,
            session_name: &str,
            external_id: Option<&str>,
            region: Option<&str>,
            duration_seconds: Option<i32>,
            _mfa_serial: Option<&str>,
            _mfa_token: Option<&str>,
        ) -> Result<Credentials, AwswitError> {
            self.assume_role_calls.lock().unwrap().push(AssumeRoleCall {
                role_arn: role_arn.to_string(),
                session_name: session_name.to_string(),
                external_id: external_id.map(String::from),
                region: region.map(String::from),
                duration_seconds,
            });
            Ok(Credentials {
                access_key_id: "AKIAMOCK".to_string(),
                secret_access_key: "mock_secret".to_string(),
                session_token: Some("mock_token".to_string()),
                expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                region: region.map(String::from),
            })
        }

        async fn get_session_token(
            &self,
            _source_credentials: &Credentials,
            _mfa_serial: Option<&str>,
            _mfa_token: Option<&str>,
            _duration_seconds: Option<i32>,
        ) -> Result<Credentials, AwswitError> {
            Ok(Credentials {
                access_key_id: "AKIAMOCK_SESSION".to_string(),
                secret_access_key: "mock_session_secret".to_string(),
                session_token: Some("mock_session_token".to_string()),
                expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                region: None,
            })
        }
    }

    #[tokio::test]
    async fn mock_single_hop_role_chain() {
        let mut profiles = HashMap::new();
        profiles.insert("base".to_string(), make_user_profile("base"));
        profiles.insert(
            "dev".to_string(),
            make_role_profile("dev", "arn:aws:iam::111:role/Dev", "base"),
        );

        let config = default_config();
        let resolver = ProfileResolver::new(&profiles, &config);
        let mock_sts = MockStsClient::new();
        let cache_manager = CacheManager::new().unwrap();
        let args = Args::default();

        let creds = resolver
            .resolve_credentials("dev", &args, &mock_sts, &cache_manager)
            .await
            .unwrap();

        assert_eq!(creds.access_key_id, "AKIAMOCK");

        let calls = mock_sts.assume_role_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].role_arn, "arn:aws:iam::111:role/Dev");
        assert_eq!(calls[0].session_name, "dev");
        assert_eq!(calls[0].external_id, None);
        assert_eq!(calls[0].region, None);
        assert_eq!(calls[0].duration_seconds, None);
    }
}
