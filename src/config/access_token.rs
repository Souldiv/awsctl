use crate::config::DeviceClient;
use crate::utils::serde::toml_date_format;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccessToken {
    pub access_token: String,
    #[serde(with = "toml_date_format")]
    pub expires_at: DateTime<Utc>,
    pub refresh_token: Option<String>,
    #[serde(flatten)]
    pub device_client: Option<DeviceClient>,
}

impl Default for AccessToken {
    fn default() -> Self {
        AccessToken {
            access_token: String::default(),
            expires_at: DateTime::default(),
            refresh_token: None,
            device_client: None,
        }
    }
}

impl AccessToken {
    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }

    pub fn is_set(&self) -> bool {
        !self.access_token.is_empty()
            || self.expires_at != DateTime::<Utc>::default()
            || self.refresh_token.is_some()
            || self.device_client.is_some()
    }
}
