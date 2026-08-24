//! Credential-provider preflight.
//!
//! Only variable names cross this boundary.  Values are neither retained nor
//! exposed, so diagnostics cannot accidentally disclose credentials.

use std::collections::BTreeSet;

use crate::activation::CredentialVariable;
use crate::catalog::EnvironmentSource;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialEnvironment {
    present: BTreeSet<CredentialVariable>,
}

impl CredentialEnvironment {
    pub(crate) fn capture() -> Self {
        let present = CredentialVariable::ALL
            .into_iter()
            .filter(|variable| std::env::var_os(variable.name()).is_some())
            .collect();
        Self { present }
    }

    #[cfg(test)]
    fn from_present(present: impl IntoIterator<Item = CredentialVariable>) -> Self {
        Self {
            present: present.into_iter().collect(),
        }
    }

    pub(crate) fn present(&self) -> &BTreeSet<CredentialVariable> {
        &self.present
    }

    pub(crate) fn assess(&self, intended_source: EnvironmentSource) -> OverrideAssessment {
        let profile_uses_ecs_container = intended_source == EnvironmentSource::UsesEcsContainer;
        let profile_uses_ec2_metadata =
            intended_source == EnvironmentSource::UsesEc2InstanceMetadata;
        let profile_uses_login = intended_source == EnvironmentSource::UsesLoginSession;
        let ambiguous_container_endpoint = self
            .present
            .contains(&CredentialVariable::ContainerCredentialsRelativeUri)
            && self
                .present
                .contains(&CredentialVariable::ContainerCredentialsFullUri);
        let ambiguous_container_auth = self
            .present
            .contains(&CredentialVariable::ContainerAuthorizationToken)
            && self
                .present
                .contains(&CredentialVariable::ContainerAuthorizationTokenFile);

        let conflicts = self
            .present
            .iter()
            .copied()
            .filter(|variable| {
                let intended_container = profile_uses_ecs_container
                    && !ambiguous_container_endpoint
                    && !ambiguous_container_auth
                    && variable.is_container();
                let intended_ec2_metadata = profile_uses_ec2_metadata && variable.is_ec2_metadata();
                let intended_login_cache = profile_uses_login && variable.is_login_cache();
                !intended_container && !intended_ec2_metadata && !intended_login_cache
            })
            .collect();

        OverrideAssessment { conflicts }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OverrideAssessment {
    conflicts: BTreeSet<CredentialVariable>,
}

impl OverrideAssessment {
    pub(crate) fn conflicts(&self) -> &BTreeSet<CredentialVariable> {
        &self.conflicts
    }

    pub(crate) fn is_safe(&self) -> bool {
        self.conflicts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_profile_rejects_every_present_override() {
        let environment = CredentialEnvironment::from_present(CredentialVariable::ALL);
        let assessment = environment.assess(EnvironmentSource::DoesNotUseEnvironment);
        assert_eq!(assessment.conflicts(), environment.present());
        assert!(!assessment.is_safe());
    }

    #[test]
    fn environment_source_never_exempts_direct_credentials() {
        for present in [
            vec![
                CredentialVariable::AccessKeyId,
                CredentialVariable::SecretAccessKey,
                CredentialVariable::SessionToken,
            ],
            vec![
                CredentialVariable::AccessKeyLegacy,
                CredentialVariable::SecretKeyLegacy,
                CredentialVariable::SecurityToken,
            ],
            vec![
                CredentialVariable::AmazonAccessKeyId,
                CredentialVariable::AmazonSecretAccessKey,
                CredentialVariable::AmazonSessionToken,
            ],
        ] {
            let environment = CredentialEnvironment::from_present(present);
            let assessment = environment.assess(EnvironmentSource::UsesEnvironment);
            assert_eq!(assessment.conflicts(), environment.present());
            assert!(!assessment.is_safe());
        }
    }

    #[test]
    fn partial_environment_tuple_is_a_clearable_named_conflict() {
        let environment = CredentialEnvironment::from_present([CredentialVariable::AccessKeyId]);
        let assessment = environment.assess(EnvironmentSource::UsesEnvironment);
        assert_eq!(
            assessment.conflicts(),
            &BTreeSet::from([CredentialVariable::AccessKeyId])
        );
    }

    #[test]
    fn environment_source_does_not_exempt_other_providers() {
        let environment = CredentialEnvironment::from_present([
            CredentialVariable::AccessKeyId,
            CredentialVariable::SecretAccessKey,
            CredentialVariable::ContainerCredentialsFullUri,
        ]);
        let assessment = environment.assess(EnvironmentSource::UsesEnvironment);
        assert_eq!(assessment.conflicts(), environment.present());
    }

    #[test]
    fn ecs_source_allows_only_container_provider_variables() {
        let environment = CredentialEnvironment::from_present([
            CredentialVariable::ContainerCredentialsFullUri,
            CredentialVariable::ContainerAuthorizationTokenFile,
            CredentialVariable::AccessKeyId,
        ]);
        let assessment = environment.assess(EnvironmentSource::UsesEcsContainer);
        assert_eq!(
            assessment.conflicts(),
            &BTreeSet::from([CredentialVariable::AccessKeyId])
        );
    }

    #[test]
    fn ecs_source_rejects_competing_endpoint_or_authorization_inputs() {
        let environment = CredentialEnvironment::from_present([
            CredentialVariable::ContainerCredentialsRelativeUri,
            CredentialVariable::ContainerCredentialsFullUri,
            CredentialVariable::ContainerAuthorizationToken,
            CredentialVariable::ContainerAuthorizationTokenFile,
        ]);
        let assessment = environment.assess(EnvironmentSource::UsesEcsContainer);
        assert_eq!(assessment.conflicts(), environment.present());
    }

    #[test]
    fn ec2_source_allows_only_its_explicit_metadata_endpoint() {
        let environment = CredentialEnvironment::from_present([
            CredentialVariable::Ec2MetadataServiceEndpoint,
            CredentialVariable::AccessKeyId,
            CredentialVariable::BedrockBearerToken,
        ]);
        let assessment = environment.assess(EnvironmentSource::UsesEc2InstanceMetadata);
        assert_eq!(
            assessment.conflicts(),
            &BTreeSet::from([
                CredentialVariable::AccessKeyId,
                CredentialVariable::BedrockBearerToken,
            ])
        );
    }

    #[test]
    fn ordinary_profiles_reject_an_ambient_metadata_endpoint() {
        let environment =
            CredentialEnvironment::from_present([CredentialVariable::Ec2MetadataServiceEndpoint]);
        let assessment = environment.assess(EnvironmentSource::DoesNotUseEnvironment);
        assert_eq!(assessment.conflicts(), environment.present());
    }

    #[test]
    fn login_source_allows_only_its_explicit_cache_directory() {
        let environment = CredentialEnvironment::from_present([
            CredentialVariable::LoginCacheDirectory,
            CredentialVariable::AccessKeyId,
        ]);
        let assessment = environment.assess(EnvironmentSource::UsesLoginSession);
        assert_eq!(
            assessment.conflicts(),
            &BTreeSet::from([CredentialVariable::AccessKeyId])
        );
    }
}
