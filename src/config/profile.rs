use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Profile {
    sso_account_name: String,
    sso_account_id: String,
    sso_role_name: String,
    region: String,
}

impl Profile {
    pub fn new(
        sso_account_name: String,
        sso_account_id: String,
        sso_role_name: String,
        region: String,
    ) -> Self {
        Profile {
            sso_account_name,
            sso_account_id,
            sso_role_name,
            region,
        }
    }

    pub fn get_account_name(&self) -> &str {
        &self.sso_account_name
    }

    pub fn get_account_id(&self) -> &str {
        &self.sso_account_id
    }

    pub fn get_role_name(&self) -> &str {
        &self.sso_role_name
    }

    pub fn get_region(&self) -> &str {
        &self.region
    }
}
