use crate::aws::Credentials;
use crate::error::AwswitError;

pub trait CredentialStore {
    fn get(&self, key: &str) -> Result<Option<Credentials>, AwswitError>;
    fn set(&self, key: &str, credentials: &Credentials) -> Result<(), AwswitError>;
    fn remove(&self, key: &str) -> Result<(), AwswitError>;
}
