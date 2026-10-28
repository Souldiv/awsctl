use crate::config::profile::Profile;
use crate::config::profile_name::ResolvedProfileName;
use crate::config::SsoDefaults;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize, Debug)]
pub struct AccountConfig {
    defaults: Option<SsoDefaults>,
    profile: Option<Profile>,
    roles: Vec<String>,
    regions: Vec<String>,
    #[serde(default)]
    profile_names: BTreeMap<String, String>,
}

impl Default for AccountConfig {
    fn default() -> Self {
        AccountConfig {
            defaults: None,
            profile: None,
            roles: vec![String::from("AdministratorAccess")],
            regions: vec![String::from("us-east-1"), String::from("eu-west-2")],
            profile_names: BTreeMap::new(),
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

    pub fn get_profile_names(&self) -> &BTreeMap<String, String> {
        &self.profile_names
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

    /// Lists every account awsctl has seen against an editable profile name,
    /// so the names can be shortened in one place. Names already in the config
    /// are left alone; only accounts missing from it are added.
    pub fn seed_profile_names(&mut self, resolved: &[ResolvedProfileName]) {
        for entry in resolved {
            if self.profile_names.contains_key(&entry.account_id) {
                continue;
            }
            self.profile_names
                .entry(entry.account_name.clone())
                .or_insert_with(|| entry.profile_name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AccountConfig;
    use crate::config::profile_name::ResolvedProfileName;

    fn resolved(account_name: &str, account_id: &str, profile_name: &str) -> ResolvedProfileName {
        ResolvedProfileName {
            account_id: String::from(account_id),
            account_name: String::from(account_name),
            profile_name: String::from(profile_name),
            wanted_name: None,
        }
    }

    #[test]
    fn seeds_every_account_it_has_not_seen() {
        let mut config = AccountConfig::default();

        config.seed_profile_names(&[
            resolved(
                "labflow-dev-customer",
                "111111111111",
                "labflow-dev-customer",
            ),
            resolved("labflow-saas-au", "222222222222", "labflow-saas-au"),
        ]);

        assert_eq!(
            config.get_profile_names().get("labflow-dev-customer"),
            Some(&String::from("labflow-dev-customer"))
        );
        assert_eq!(
            config.get_profile_names().get("labflow-saas-au"),
            Some(&String::from("labflow-saas-au"))
        );
    }

    #[test]
    fn leaves_a_name_the_user_shortened() {
        let mut config = AccountConfig::default();
        config.seed_profile_names(&[resolved(
            "labflow-dev-customer",
            "111111111111",
            "labflow-dev-customer",
        )]);

        config
            .profile_names
            .insert(String::from("labflow-dev-customer"), String::from("dev"));
        config.seed_profile_names(&[resolved(
            "labflow-dev-customer",
            "111111111111",
            "labflow-dev-customer",
        )]);

        assert_eq!(
            config.get_profile_names().get("labflow-dev-customer"),
            Some(&String::from("dev"))
        );
    }

    #[test]
    fn leaves_a_name_keyed_by_account_id() {
        let mut config = AccountConfig::default();
        config
            .profile_names
            .insert(String::from("111111111111"), String::from("dev"));

        config.seed_profile_names(&[resolved(
            "labflow-dev-customer",
            "111111111111",
            "labflow-dev-customer",
        )]);

        assert_eq!(config.get_profile_names().len(), 1);
        assert_eq!(
            config.get_profile_names().get("111111111111"),
            Some(&String::from("dev"))
        );
    }

    #[test]
    fn reads_the_config_template_shipped_with_awsctl() {
        let template = include_str!("../../Config.toml");

        let config: AccountConfig = toml::from_str(template).expect("template parses");

        assert!(config.is_defaults_set());
        assert!(config.get_profile_names().is_empty());
    }

    #[test]
    fn reads_a_config_written_before_profile_names_existed() {
        let stored = r#"
            roles = ["AdministratorAccess"]
            regions = ["us-east-1"]
        "#;

        let config: AccountConfig = toml::from_str(stored).expect("config parses");

        assert!(config.get_profile_names().is_empty());
    }
}
