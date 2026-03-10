use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::aws::{Credentials, StsClient};
use crate::cache::CacheManager;
use crate::cli::Args;
use crate::config::AwswitConfig;
use crate::error::AwswitError;
use crate::profile::Profile;

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
        sts_client: &StsClient,
        cache_manager: &CacheManager,
    ) -> Result<Credentials, AwswitError> {
        tracing::info!("Resolving credentials for profile: {}", profile_name);

        // Check if we're using --role-arn directly
        if let Some(ref role_arn) = args.resolve_role_arn() {
            return self.assume_role_from_cli(role_arn, args, sts_client).await;
        }

        // Get the target profile
        let target_profile = self.profiles.get(profile_name)
            .ok_or_else(|| AwswitError::ProfileNotFound(profile_name.to_string()))?;

        // Handle credential_process
        if target_profile.uses_credential_process() {
            return self.get_credentials_from_process(target_profile).await;
        }

        // If it's a role profile, resolve the chain
        if target_profile.is_role_profile() {
            return self.resolve_role_chain(profile_name, args, sts_client, cache_manager).await;
        }

        // User profile - get session token if MFA required
        if target_profile.requires_mfa() {
            return self.get_session_token_credentials(
                target_profile,
                args,
                sts_client,
                cache_manager,
            ).await;
        }

        // Simple user profile - return the credentials directly
        self.profile_to_credentials(target_profile)
    }

    /// Assume role directly from CLI arguments
    async fn assume_role_from_cli(
        &self,
        role_arn: &str,
        args: &Args,
        sts_client: &StsClient,
    ) -> Result<Credentials, AwswitError> {
        tracing::info!("Assuming role from CLI: {}", role_arn);

        // Get source credentials
        let source_credentials = if let Some(ref source_profile_name) = args.source_profile {
            let source_profile = self.profiles.get(source_profile_name)
                .ok_or_else(|| AwswitError::SourceProfileNotFound(source_profile_name.clone()))?;
            
            // Check if source requires MFA
            if source_profile.requires_mfa() {
                // This is more complex - would need to get session token first
                // For now, use the profile's credentials directly
                tracing::warn!("Source profile requires MFA - this may not work correctly");
            }
            
            Some(self.profile_to_credentials(source_profile)?)
        } else {
            None
        };

        let session_name = args.get_session_name("awswit-cli-role");
        let role_duration = args.role_duration
            .or_else(|| {
                let rd = self.config.role_duration;
                if rd > 0 { Some(rd) } else { None }
            });
        let region = args.region.clone().or_else(|| self.config.region.clone());

        sts_client.assume_role(
            source_credentials.as_ref(),
            role_arn,
            &session_name,
            args.external_id.as_deref(),
            region.as_deref(),
            role_duration,
            None, // mfa_serial
            None, // mfa_token
        ).await
    }

    /// Resolve a role chain and get final credentials
    async fn resolve_role_chain(
        &self,
        profile_name: &str,
        args: &Args,
        sts_client: &StsClient,
        cache_manager: &CacheManager,
    ) -> Result<Credentials, AwswitError> {
        // Get the role chain
        let chain = self.get_role_chain(profile_name)?;
        tracing::debug!("Role chain: {:?}", chain.iter().map(|p| &p.name).collect::<Vec<_>>());

        // The first profile in the chain should be the source (user profile or credential_source)
        // We work backwards: get source credentials, then assume each role in sequence

        let target_profile = self.profiles.get(profile_name)
            .ok_or_else(|| AwswitError::ProfileNotFound(profile_name.to_string()))?;

        // Get source credentials
        let source_credentials = self.get_source_credentials(
            &chain,
            args,
            sts_client,
            cache_manager,
        ).await?;

        // Determine role duration
        let role_duration = args.role_duration
            .or(target_profile.duration_seconds)
            .or_else(|| {
                let rd = self.config.role_duration;
                if rd > 0 { Some(rd) } else { None }
            });

        // Check for MFA requirement
        let mfa_serial = self.get_mfa_serial_for_chain(&chain);
        
        // If MFA is required and role_duration > 3600, we need special handling
        if mfa_serial.is_some() && role_duration.map(|d| d > 3600).unwrap_or(false) {
            // Cannot use temp creds for custom role duration > 1 hour
            return self.assume_role_with_mfa_large_duration(
                target_profile,
                &source_credentials,
                args,
                sts_client,
                role_duration,
            ).await;
        }

        // If MFA required, get session token first
        let assume_source = if let Some(ref mfa_serial) = mfa_serial {
            self.get_session_token_with_mfa(
                &source_credentials,
                mfa_serial,
                args,
                sts_client,
                cache_manager,
            ).await?
        } else {
            source_credentials
        };

        // Assume the role
        let role_arn = target_profile.role_arn.as_ref()
            .ok_or_else(|| AwswitError::invalid_profile(profile_name, "missing role_arn"))?;
        
        let session_name = args.session_name.clone()
            .or(target_profile.role_session_name.clone())
            .or(self.config.role_session_name.clone())
            .unwrap_or_else(|| profile_name.to_string());

        let region = args.region.clone()
            .or(target_profile.region.clone())
            .or(self.config.region.clone());

        let external_id = args.external_id.clone()
            .or(target_profile.external_id.clone());

        sts_client.assume_role(
            Some(&assume_source),
            role_arn,
            &session_name,
            external_id.as_deref(),
            region.as_deref(),
            role_duration,
            None,
            None,
        ).await
    }

    /// Get the role chain for a profile
    fn get_role_chain(&self, profile_name: &str) -> Result<Vec<&Profile>, AwswitError> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        let mut current_name = profile_name.to_string();

        loop {
            // Check for cycles
            if visited.contains(&current_name) {
                return Err(AwswitError::RoleChainCycle(current_name));
            }
            visited.insert(current_name.clone());

            let profile = self.profiles.get(&current_name)
                .ok_or_else(|| AwswitError::ProfileNotFound(current_name.clone()))?;

            chain.push(profile);

            // If this profile has a source_profile that's also a role, continue the chain
            if let Some(ref source_name) = profile.source_profile {
                if let Some(source_profile) = self.profiles.get(source_name) {
                    if source_profile.is_role_profile() {
                        current_name = source_name.clone();
                        continue;
                    }
                }
            }

            // End of chain
            break;
        }

        // Reverse so first element is the source
        chain.reverse();
        Ok(chain)
    }

    /// Get source credentials for a role chain
    async fn get_source_credentials(
        &self,
        chain: &[&Profile],
        args: &Args,
        sts_client: &StsClient,
        cache_manager: &CacheManager,
    ) -> Result<Credentials, AwswitError> {
        // Get the first profile in the chain (source)
        let source_profile = chain.first()
            .ok_or_else(|| AwswitError::ValidationError("Empty role chain".to_string()))?;

        // Check credential_source
        if let Some(ref cred_source) = source_profile.credential_source {
            return self.get_credentials_from_source(cred_source).await;
        }

        // Check for source_profile
        if let Some(ref source_name) = source_profile.source_profile {
            let user_profile = self.profiles.get(source_name)
                .ok_or_else(|| AwswitError::SourceProfileNotFound(source_name.clone()))?;
            
            return self.profile_to_credentials(user_profile);
        }

        // Use the profile's own credentials
        self.profile_to_credentials(source_profile)
    }

    /// Get MFA serial for a role chain
    fn get_mfa_serial_for_chain(&self, chain: &[&Profile]) -> Option<String> {
        // Check source profiles for mfa_serial
        for profile in chain {
            if let Some(ref mfa_serial) = profile.mfa_serial {
                return Some(mfa_serial.clone());
            }
            if let Some(ref source_name) = profile.source_profile {
                if let Some(source_profile) = self.profiles.get(source_name) {
                    if let Some(ref mfa_serial) = source_profile.mfa_serial {
                        return Some(mfa_serial.clone());
                    }
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
        sts_client: &StsClient,
        cache_manager: &CacheManager,
    ) -> Result<Credentials, AwswitError> {
        // Check cache first (unless force refresh)
        if !args.force_refresh {
            let cache_key = format!("session-{}", source_credentials.access_key_id);
            if let Some(cached) = cache_manager.get(&cache_key)? {
                if !cached.is_expired() {
                    tracing::info!("Using cached MFA session credentials");
                    return Ok(cached);
                }
            }
        }

        // Need to get new session token
        let mfa_token = self.get_mfa_token(args)?;

        let session = sts_client.get_session_token(
            source_credentials,
            Some(mfa_serial),
            Some(&mfa_token),
            self.config.debug.session_token_duration,
        ).await?;

        // Cache the session
        let cache_key = format!("session-{}", source_credentials.access_key_id);
        cache_manager.set(&cache_key, &session)?;

        Ok(session)
    }

    /// Get MFA token from args or prompt user
    fn get_mfa_token(&self, args: &Args) -> Result<String, AwswitError> {
        if let Some(ref token) = args.mfa_token {
            return Ok(token.clone());
        }

        // Prompt user
        use dialoguer::Input;
        let token: String = Input::new()
            .with_prompt("Enter MFA token")
            .interact_text()
            .map_err(|_| AwswitError::MfaTokenRequired)?;

        Ok(token)
    }

    /// Get session token credentials for a user profile with MFA
    async fn get_session_token_credentials(
        &self,
        profile: &Profile,
        args: &Args,
        sts_client: &StsClient,
        cache_manager: &CacheManager,
    ) -> Result<Credentials, AwswitError> {
        let source_credentials = self.profile_to_credentials(profile)?;
        let mfa_serial = profile.mfa_serial.as_ref()
            .ok_or(AwswitError::MfaTokenRequired)?;

        self.get_session_token_with_mfa(
            &source_credentials,
            mfa_serial,
            args,
            sts_client,
            cache_manager,
        ).await
    }

    /// Assume role with MFA when duration > 1 hour (skip get_session_token)
    async fn assume_role_with_mfa_large_duration(
        &self,
        profile: &Profile,
        source_credentials: &Credentials,
        args: &Args,
        sts_client: &StsClient,
        role_duration: Option<i32>,
    ) -> Result<Credentials, AwswitError> {
        if args.auto_refresh && role_duration.map(|d| d > 3600).unwrap_or(false) {
            return Err(AwswitError::AutoRefreshDurationLimit);
        }

        let mfa_serial = self.get_mfa_serial_for_chain(&[profile]);
        let mfa_token = if mfa_serial.is_some() {
            Some(self.get_mfa_token(args)?)
        } else {
            None
        };

        let role_arn = profile.role_arn.as_ref()
            .ok_or_else(|| AwswitError::invalid_profile(&profile.name, "missing role_arn"))?;

        let session_name = args.session_name.clone()
            .or(profile.role_session_name.clone())
            .or(self.config.role_session_name.clone())
            .unwrap_or_else(|| profile.name.clone());

        let region = args.region.clone()
            .or(profile.region.clone())
            .or(self.config.region.clone());

        sts_client.assume_role(
            Some(source_credentials),
            role_arn,
            &session_name,
            profile.external_id.as_deref(),
            region.as_deref(),
            role_duration,
            mfa_serial.as_deref(),
            mfa_token.as_deref(),
        ).await
    }

    /// Get credentials from credential_source
    async fn get_credentials_from_source(&self, source: &str) -> Result<Credentials, AwswitError> {
        match source {
            "Environment" => {
                // Get from environment variables
                let access_key = std::env::var("AWS_ACCESS_KEY_ID")
                    .map_err(|_| AwswitError::EnvError("AWS_ACCESS_KEY_ID not set".to_string()))?;
                let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY")
                    .map_err(|_| AwswitError::EnvError("AWS_SECRET_ACCESS_KEY not set".to_string()))?;
                let session_token = std::env::var("AWS_SESSION_TOKEN").ok();

                Ok(Credentials {
                    access_key_id: access_key,
                    secret_access_key: secret_key,
                    session_token,
                    expiration: None,
                    region: std::env::var("AWS_REGION").ok(),
                })
            }
            "Ec2InstanceMetadata" | "EcsContainer" => {
                // Use default credential chain which handles these
                Err(AwswitError::InvalidCredentialSource(format!(
                    "{} should be handled by AWS SDK default chain", source
                )))
            }
            _ => {
                Err(AwswitError::InvalidCredentialSource(source.to_string()))
            }
        }
    }

    /// Get credentials from credential_process
    async fn get_credentials_from_process(&self, profile: &Profile) -> Result<Credentials, AwswitError> {
        let command = profile.credential_process.as_ref()
            .ok_or_else(|| AwswitError::invalid_profile(&profile.name, "missing credential_process"))?;

        tracing::info!("Running credential_process: {}", command);

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| AwswitError::CredentialProcessFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AwswitError::CredentialProcessFailed(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let creds: CredentialProcessOutput = serde_json::from_str(&stdout)?;

        Ok(Credentials {
            access_key_id: creds.access_key_id,
            secret_access_key: creds.secret_access_key,
            session_token: creds.session_token,
            expiration: creds.expiration.and_then(|e| {
                chrono::DateTime::parse_from_rfc3339(&e)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .ok()
            }),
            region: profile.region.clone(),
        })
    }

    /// Convert a profile to credentials
    fn profile_to_credentials(&self, profile: &Profile) -> Result<Credentials, AwswitError> {
        let access_key = profile.aws_access_key_id.clone()
            .ok_or_else(|| AwswitError::missing_key(&profile.name, "aws_access_key_id"))?;
        let secret_key = profile.aws_secret_access_key.clone()
            .ok_or_else(|| AwswitError::missing_key(&profile.name, "aws_secret_access_key"))?;

        Ok(Credentials {
            access_key_id: access_key,
            secret_access_key: secret_key,
            session_token: profile.aws_session_token.clone(),
            expiration: None,
            region: profile.region.clone(),
        })
    }
}

/// Output format for credential_process
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CredentialProcessOutput {
    #[serde(alias = "AccessKeyId")]
    access_key_id: String,
    #[serde(alias = "SecretAccessKey")]
    secret_access_key: String,
    #[serde(alias = "SessionToken")]
    session_token: Option<String>,
    #[serde(alias = "Expiration")]
    expiration: Option<String>,
}
