use crate::config::profile::Profile;
use crate::config::SsoDefaults;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct AccountConfig {
    defaults: Option<SsoDefaults>,
    profile: Option<Profile>,
    roles: Vec<String>,
    regions: Vec<String>,
}

impl Default for AccountConfig {
    fn default() -> Self {
        AccountConfig {
            defaults: None,
            profile: None,
            roles: vec![String::from("AdministratorAccess")],
            regions: vec![String::from("us-east-1"), String::from("eu-west-2")],
        }
    }
}

impl AccountConfig {
    pub fn get_list_of_regions(&self) -> Vec<String> {
        self.regions.clone()
    }

    pub fn get_list_of_roles(&self) -> Vec<String> {
        self.roles.clone()
    }

    pub fn get_sso_defaults(&self) -> Option<&SsoDefaults> {
        self.defaults.as_ref()
    }

    pub fn get_current_profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    pub fn set_profile(&mut self, profile: Profile) -> Result<()> {
        self.profile = Some(profile);
        Ok(())
    }

    pub fn set_defaults(&mut self, defaults: SsoDefaults) -> Result<()> {
        self.defaults = Some(defaults);
        Ok(())
    }

    pub fn is_defaults_set(&self) -> bool {
        self.defaults.is_some()
    }
}
