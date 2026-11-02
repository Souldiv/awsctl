use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

use crate::config::{Profile, SsoProfile};

const DEFAULT_MARKER: &str = "*";
pub const FALLBACK_REGION: &str = "ap-southeast-2";

pub fn last_region_for(
    account_id: &str,
    current: Option<&Profile>,
    written: &[SsoProfile],
) -> String {
    current
        .filter(|current| current.get_account_id() == account_id)
        .map(|current| current.get_region().to_string())
        .or_else(|| {
            written
                .iter()
                .find(|profile| profile.account_id == account_id)
                .map(|profile| profile.region.clone())
        })
        .unwrap_or_else(|| String::from(FALLBACK_REGION))
}

pub fn find_profile<'a>(profiles: &'a [SsoProfile], profile_name: &str) -> Result<&'a SsoProfile> {
    profiles
        .iter()
        .find(|profile| profile.profile_name == profile_name)
        .ok_or_else(|| {
            let known: Vec<&str> = profiles
                .iter()
                .map(|profile| profile.profile_name.as_str())
                .collect();
            anyhow!(
                "Unknown profile '{}'. Known profiles: {}",
                profile_name,
                known.join(", ")
            )
        })
}

pub fn to_current_profile(
    profile: &SsoProfile,
    configured_names: &BTreeMap<String, String>,
) -> Profile {
    let account_name = account_name_configured_for(profile, configured_names)
        .unwrap_or_else(|| profile.profile_name.clone());

    Profile::new(
        account_name,
        profile.account_id.clone(),
        profile.role_name.clone(),
        profile.region.clone(),
    )
}

fn account_name_configured_for(
    profile: &SsoProfile,
    configured_names: &BTreeMap<String, String>,
) -> Option<String> {
    configured_names
        .iter()
        .filter(|(key, _)| **key != profile.account_id)
        .find(|(_, name)| **name == profile.profile_name)
        .map(|(account_name, _)| account_name.clone())
}

pub fn format_listing(profiles: &[SsoProfile], current: Option<&Profile>) -> Vec<String> {
    let name_width = profiles
        .iter()
        .map(|profile| profile.profile_name.len())
        .max()
        .unwrap_or(0);

    profiles
        .iter()
        .map(|profile| {
            let marker = if is_current(profile, current) {
                DEFAULT_MARKER
            } else {
                " "
            };
            format!(
                "{} {:<width$}  {}  {}  {}",
                marker,
                profile.profile_name,
                profile.account_id,
                profile.role_name,
                profile.region,
                width = name_width
            )
        })
        .collect()
}

fn is_current(profile: &SsoProfile, current: Option<&Profile>) -> bool {
    current.is_some_and(|current| {
        current.get_account_id() == profile.account_id
            && current.get_role_name() == profile.role_name
    })
}

#[cfg(test)]
mod tests {
    use super::{find_profile, format_listing, last_region_for, to_current_profile};
    use crate::config::{Profile, SsoProfile};
    use std::collections::BTreeMap;

    fn profile(profile_name: &str, account_id: &str) -> SsoProfile {
        SsoProfile {
            profile_name: String::from(profile_name),
            account_id: String::from(account_id),
            role_name: String::from("AdministratorAccess"),
            region: String::from("eu-west-2"),
        }
    }

    fn configured(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn resolves_a_name_to_its_profile() {
        let profiles = [
            profile("dev", "111111111111"),
            profile("au", "222222222222"),
        ];

        let found = find_profile(&profiles, "au").expect("finds");

        assert_eq!(found.account_id, "222222222222");
        assert_eq!(found.role_name, "AdministratorAccess");
        assert_eq!(found.region, "eu-west-2");
    }

    #[test]
    fn rejects_an_unknown_name_and_lists_the_known_ones() {
        let profiles = [
            profile("dev", "111111111111"),
            profile("au", "222222222222"),
        ];

        let error = find_profile(&profiles, "prod").unwrap_err();

        let message = error.to_string();
        assert!(message.contains("prod"));
        assert!(message.contains("dev, au"));
    }

    #[test]
    fn takes_the_account_name_from_the_configured_names() {
        let current = to_current_profile(
            &profile("dev", "111111111111"),
            &configured(&[("labflow-dev-customer", "dev"), ("labflow-saas-au", "au")]),
        );

        assert_eq!(current.get_account_name(), "labflow-dev-customer");
        assert_eq!(current.get_account_id(), "111111111111");
        assert_eq!(current.get_role_name(), "AdministratorAccess");
        assert_eq!(current.get_region(), "eu-west-2");
    }

    #[test]
    fn keeps_the_profile_name_when_the_name_is_keyed_by_account_id() {
        let current = to_current_profile(
            &profile("dev", "111111111111"),
            &configured(&[("111111111111", "dev")]),
        );

        assert_eq!(current.get_account_name(), "dev");
    }

    #[test]
    fn keeps_the_profile_name_when_nothing_is_configured() {
        let current = to_current_profile(&profile("dev", "111111111111"), &configured(&[]));

        assert_eq!(current.get_account_name(), "dev");
    }

    #[test]
    fn lists_every_profile_and_marks_the_current_one() {
        let profiles = [
            profile("dev", "111111111111"),
            profile("au", "222222222222"),
            profile("prod-eu", "333333333333"),
        ];
        let current = Profile::new(
            String::from("labflow-saas-au"),
            String::from("222222222222"),
            String::from("AdministratorAccess"),
            String::from("eu-west-2"),
        );

        let lines = format_listing(&profiles, Some(&current));

        assert_eq!(
            lines,
            [
                "  dev      111111111111  AdministratorAccess  eu-west-2",
                "* au       222222222222  AdministratorAccess  eu-west-2",
                "  prod-eu  333333333333  AdministratorAccess  eu-west-2",
            ]
        );
    }

    #[test]
    fn marks_nothing_when_no_profile_is_current() {
        let lines = format_listing(&[profile("dev", "111111111111")], None);

        assert_eq!(
            lines,
            ["  dev  111111111111  AdministratorAccess  eu-west-2"]
        );
    }

    fn current(account_id: &str, region: &str) -> Profile {
        Profile::new(
            String::from("labflow-saas-au"),
            String::from(account_id),
            String::from("AdministratorAccess"),
            String::from(region),
        )
    }

    #[test]
    fn keeps_the_region_of_the_current_profile_for_its_account() {
        let region = last_region_for(
            "222222222222",
            Some(&current("222222222222", "ap-southeast-4")),
            &[profile("au", "222222222222")],
        );

        assert_eq!(region, "ap-southeast-4");
    }

    #[test]
    fn takes_the_region_written_for_the_account_when_it_is_not_current() {
        let region = last_region_for(
            "222222222222",
            Some(&current("111111111111", "us-west-2")),
            &[
                profile("dev", "111111111111"),
                profile("au", "222222222222"),
            ],
        );

        assert_eq!(region, "eu-west-2");
    }

    #[test]
    fn falls_back_to_ap_southeast_2_for_an_account_never_set_up() {
        let region = last_region_for("333333333333", None, &[profile("dev", "111111111111")]);

        assert_eq!(region, "ap-southeast-2");
    }
}
