use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

use crate::config::profile_name::resolve_profile_names;
use crate::config::{Profile, SsoProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameClash {
    pub account_name: String,
    pub wanted_name: String,
    pub profile_name: String,
}

#[derive(Debug, Clone)]
pub struct ProfilePlan {
    /// The account whose role credentials go into `[default]`.
    pub default_profile: Profile,
    pub sso_profiles: Vec<SsoProfile>,
    pub name_clashes: Vec<NameClash>,
}

/// Plans the profiles for the selected accounts, in the order they were
/// selected.
///
/// The first account selected keeps `[default]`, so the order the caller
/// passes is the order the user chose and is never sorted away.
pub fn plan_profiles(
    profiles: &[Profile],
    configured_names: &BTreeMap<String, String>,
) -> Result<ProfilePlan> {
    let default_profile = profiles
        .first()
        .ok_or_else(|| anyhow!("No account was selected. Please try again."))?
        .clone();

    let resolved = resolve_profile_names(
        profiles
            .iter()
            .map(|profile| (profile.get_account_name(), profile.get_account_id())),
        configured_names,
    );

    let sso_profiles = resolved
        .iter()
        .zip(profiles)
        .map(|(resolved, profile)| SsoProfile {
            profile_name: resolved.profile_name.clone(),
            account_id: profile.get_account_id().to_string(),
            role_name: profile.get_role_name().to_string(),
            region: profile.get_region().to_string(),
        })
        .collect();

    let name_clashes = resolved
        .iter()
        .filter_map(|resolved| {
            resolved.wanted_name.clone().map(|wanted_name| NameClash {
                account_name: resolved.account_name.clone(),
                wanted_name,
                profile_name: resolved.profile_name.clone(),
            })
        })
        .collect();

    Ok(ProfilePlan {
        default_profile,
        sso_profiles,
        name_clashes,
    })
}

pub fn pick_role(preferred: &str, granted: &[String]) -> Option<String> {
    granted
        .iter()
        .find(|role_name| *role_name == preferred)
        .or_else(|| granted.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::{pick_role, plan_profiles};
    use crate::config::Profile;
    use std::collections::BTreeMap;

    fn profile(account_name: &str, account_id: &str) -> Profile {
        Profile::new(
            String::from(account_name),
            String::from(account_id),
            String::from("AdministratorAccess"),
            String::from("eu-west-2"),
        )
    }

    fn configured(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn gives_default_to_the_first_account_selected() {
        let plan = plan_profiles(
            &[
                profile("labflow-saas-au", "222222222222"),
                profile("labflow-dev-customer", "111111111111"),
            ],
            &configured(&[]),
        )
        .expect("plans");

        assert_eq!(plan.default_profile.get_account_id(), "222222222222");
    }

    #[test]
    fn keeps_the_selection_order_of_the_written_profiles() {
        let plan = plan_profiles(
            &[
                profile("labflow-saas-au", "222222222222"),
                profile("labflow-dev-customer", "111111111111"),
            ],
            &configured(&[("labflow-saas-au", "au"), ("labflow-dev-customer", "dev")]),
        )
        .expect("plans");

        let names: Vec<&str> = plan
            .sso_profiles
            .iter()
            .map(|sso_profile| sso_profile.profile_name.as_str())
            .collect();

        assert_eq!(names, ["au", "dev"]);
    }

    #[test]
    fn writes_a_profile_for_every_selected_account() {
        let plan = plan_profiles(
            &[
                profile("labflow-dev-customer", "111111111111"),
                profile("labflow-saas-au", "222222222222"),
            ],
            &configured(&[]),
        )
        .expect("plans");

        assert_eq!(plan.sso_profiles.len(), 2);
        assert_eq!(plan.sso_profiles[0].account_id, "111111111111");
        assert_eq!(plan.sso_profiles[0].role_name, "AdministratorAccess");
        assert_eq!(plan.sso_profiles[0].region, "eu-west-2");
    }

    #[test]
    fn keeps_a_single_selection_on_the_name_it_resolves_to() {
        let plan = plan_profiles(
            &[profile("labflow-dev-customer", "111111111111")],
            &configured(&[("labflow-dev-customer", "dev")]),
        )
        .expect("plans");

        assert_eq!(plan.default_profile.get_account_id(), "111111111111");
        assert_eq!(plan.sso_profiles.len(), 1);
        assert_eq!(plan.sso_profiles[0].profile_name, "dev");
        assert!(plan.name_clashes.is_empty());
    }

    #[test]
    fn reports_two_accounts_that_want_one_name() {
        let plan = plan_profiles(
            &[
                profile("labflow-dev-customer", "111111111111"),
                profile("labflow-saas-au", "222222222222"),
            ],
            &configured(&[("labflow-dev-customer", "dev"), ("labflow-saas-au", "dev")]),
        )
        .expect("plans");

        assert_eq!(plan.sso_profiles[0].profile_name, "dev");
        assert_eq!(plan.sso_profiles[1].profile_name, "dev-222222222222");
        assert_eq!(plan.name_clashes.len(), 1);
        assert_eq!(plan.name_clashes[0].account_name, "labflow-saas-au");
        assert_eq!(plan.name_clashes[0].wanted_name, "dev");
        assert_eq!(plan.name_clashes[0].profile_name, "dev-222222222222");
    }

    #[test]
    fn refuses_a_run_with_no_account() {
        let error = plan_profiles(&[], &configured(&[])).unwrap_err();

        assert!(error.to_string().contains("No account was selected"));
    }

    fn roles(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn picks_the_chosen_role_when_the_account_grants_it() {
        let role = pick_role(
            "AdministratorAccess",
            &roles(&["ReadOnlyAccess", "AdministratorAccess"]),
        );

        assert_eq!(role.as_deref(), Some("AdministratorAccess"));
    }

    #[test]
    fn falls_back_to_the_first_role_the_account_grants() {
        let role = pick_role(
            "AdministratorAccess",
            &roles(&["ReadOnlyAccess", "PowerUserAccess"]),
        );

        assert_eq!(role.as_deref(), Some("ReadOnlyAccess"));
    }

    #[test]
    fn picks_nothing_for_an_account_that_grants_no_role() {
        assert_eq!(pick_role("AdministratorAccess", &[]), None);
    }
}
