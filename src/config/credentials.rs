use crate::config::constants::APP_NAME;
use aws_sdk_sso::config::Credentials as AWSCredentials;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Deserialize, Serialize)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub expires_at: Option<SystemTime>,
    pub provider_name: String,
}

impl Credentials {
    /// Checks if the credentials are set or not.
    ///
    /// Returns `true` if the `access_key_id`, `secret_access_key`, and `session_token` (if it is `Some`)
    /// are set to non-default values and are not empty.
    /// Also checks if the `expires_at` has a value.
    pub fn is_set(&self) -> bool {
        // Check if `access_key_id` and `secret_access_key` are not empty strings
        // and if `session_token` is Some with a non-empty value
        let access_key_set = !self.access_key_id.is_empty();
        let secret_key_set = !self.secret_access_key.is_empty();
        let token_set = match &self.session_token {
            Some(token) => !token.is_empty(),
            None => false,
        };

        // Check if `expires_at` has a value
        let expires_at_set = self.expires_at.is_some();

        // If all of the above are set, return true
        access_key_set && secret_key_set && token_set && expires_at_set
    }

    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    pub fn secret_access_key(&self) -> &str {
        &self.secret_access_key
    }

    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }
}

impl Default for Credentials {
    fn default() -> Self {
        Credentials {
            access_key_id: "".to_string(),     // Default access key
            secret_access_key: "".to_string(), // Default secret access key
            session_token: None,               // Default session token
            expires_at: None,                  // Default expiration time
            provider_name: APP_NAME.to_string(),
        }
    }
}

impl From<AWSCredentials> for Credentials {
    fn from(aws_credentials: AWSCredentials) -> Self {
        Credentials {
            access_key_id: aws_credentials.access_key_id().to_string(),
            secret_access_key: aws_credentials.secret_access_key().to_string(),
            session_token: aws_credentials.session_token().map(|s| s.to_string()),
            expires_at: aws_credentials.expiry(),
            provider_name: APP_NAME.to_string(),
        }
    }
}

impl From<Credentials> for AWSCredentials {
    fn from(credentials: Credentials) -> Self {
        AWSCredentials::new(
            credentials.access_key_id,
            credentials.secret_access_key,
            credentials.session_token,
            credentials.expires_at,
            APP_NAME,
        )
    }
}
