use crate::utils::serde::toml_date_format;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceClient {
    pub client_id: String,
    pub client_secret: String,
    #[serde(with = "toml_date_format")]
    pub registration_expires_at: DateTime<Utc>,
}

// impl DeviceClient {
//     pub fn is_expired(&self) -> bool {
//         self.registration_expires_at < Utc::now()
//     }
// }
