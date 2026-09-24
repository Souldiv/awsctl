use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SsoDefaults {
    sso_start_url: String,
    sso_region: String,
}

impl SsoDefaults {
    pub fn new(sso_start_url: String, sso_region: String) -> Self {
        SsoDefaults {
            sso_start_url,
            sso_region,
        }
    }

    pub fn get_start_url(&self) -> &str {
        &self.sso_start_url
    }

    pub fn get_sso_region(&self) -> &str {
        &self.sso_region
    }
}
