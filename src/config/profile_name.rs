use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfileName {
    pub account_id: String,
    pub account_name: String,
    pub profile_name: String,
    pub wanted_name: Option<String>,
}

/// Derives the profile name used when the config maps no name to an account.
///
/// The result is lowercase and limited to `a-z`, `0-9`, `-` and `_`, so it
/// needs no quoting as `AWS_PROFILE=<name>`. Runs of other characters collapse
/// into a single `-`.
pub fn derive_profile_name(account_name: &str) -> String {
    let mut name = String::with_capacity(account_name.len());

    for character in account_name.chars() {
        if character.is_ascii_alphanumeric() {
            name.push(character.to_ascii_lowercase());
        } else if character == '-' || character == '_' {
            name.push(character);
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }

    name.trim_matches('-').to_string()
}

/// Resolves one profile name per account, in the order the accounts arrive.
///
/// A name configured against the account name, or against its id, wins.
/// Otherwise the name is derived from the account name. Accounts that resolve
/// to a name already taken keep that name suffixed with their account id, so
/// no account loses its profile to another; `wanted_name` records the clash for
/// the caller to report.
pub fn resolve_profile_names<'a>(
    accounts: impl IntoIterator<Item = (&'a str, &'a str)>,
    configured_names: &BTreeMap<String, String>,
) -> Vec<ResolvedProfileName> {
    let mut taken: BTreeSet<String> = BTreeSet::new();

    accounts
        .into_iter()
        .map(|(account_name, account_id)| {
            let wanted = wanted_profile_name(account_name, account_id, configured_names);
            let profile_name = free_name(&wanted, account_id, &taken);
            let wanted_name = (profile_name != wanted).then(|| wanted.clone());

            taken.insert(profile_name.clone());

            ResolvedProfileName {
                account_id: account_id.to_string(),
                account_name: account_name.to_string(),
                profile_name,
                wanted_name,
            }
        })
        .collect()
}

fn wanted_profile_name(
    account_name: &str,
    account_id: &str,
    configured_names: &BTreeMap<String, String>,
) -> String {
    let configured = configured_names
        .get(account_name)
        .or_else(|| configured_names.get(account_id))
        .map(|name| name.trim())
        .filter(|name| !name.is_empty());

    match configured {
        Some(name) => name.to_string(),
        None => match derive_profile_name(account_name) {
            derived if derived.is_empty() => account_id.to_string(),
            derived => derived,
        },
    }
}

fn free_name(wanted: &str, account_id: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(wanted) {
        return wanted.to_string();
    }

    let suffixed = format!("{}-{}", wanted, account_id);
    if !taken.contains(&suffixed) {
        return suffixed;
    }

    (2..)
        .map(|attempt| format!("{}-{}", suffixed, attempt))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or(suffixed)
}

#[cfg(test)]
mod tests {
    use super::{derive_profile_name, resolve_profile_names};
    use std::collections::BTreeMap;

    fn configured(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn keeps_an_already_shell_safe_account_name() {
        assert_eq!(
            derive_profile_name("labflow-dev-customer"),
            "labflow-dev-customer"
        );
    }

    #[test]
    fn lowercases_and_replaces_characters_that_need_quoting() {
        assert_eq!(derive_profile_name("LabFlow SaaS (AU)"), "labflow-saas-au");
    }

    #[test]
    fn collapses_runs_of_replaced_characters() {
        assert_eq!(derive_profile_name("a  //  b"), "a-b");
    }

    #[test]
    fn falls_back_to_the_account_id_when_nothing_survives_derivation() {
        let resolved = resolve_profile_names([("!!!", "123456789012")], &configured(&[]));
        assert_eq!(resolved[0].profile_name, "123456789012");
    }

    #[test]
    fn prefers_the_name_configured_against_the_account_name() {
        let resolved = resolve_profile_names(
            [("labflow-dev-customer", "123456789012")],
            &configured(&[("labflow-dev-customer", "dev")]),
        );
        assert_eq!(resolved[0].profile_name, "dev");
        assert_eq!(resolved[0].wanted_name, None);
    }

    #[test]
    fn accepts_a_name_configured_against_the_account_id() {
        let resolved = resolve_profile_names(
            [("labflow-saas-au", "123456789012")],
            &configured(&[("123456789012", "au")]),
        );
        assert_eq!(resolved[0].profile_name, "au");
    }

    #[test]
    fn ignores_a_blank_configured_name() {
        let resolved = resolve_profile_names(
            [("labflow-saas-au", "123456789012")],
            &configured(&[("labflow-saas-au", "   ")]),
        );
        assert_eq!(resolved[0].profile_name, "labflow-saas-au");
    }

    #[test]
    fn suffixes_the_second_account_that_wants_a_name_already_taken() {
        let resolved = resolve_profile_names(
            [("Prod EU", "111111111111"), ("prod-eu", "222222222222")],
            &configured(&[]),
        );

        assert_eq!(resolved[0].profile_name, "prod-eu");
        assert_eq!(resolved[0].wanted_name, None);
        assert_eq!(resolved[1].profile_name, "prod-eu-222222222222");
        assert_eq!(resolved[1].wanted_name.as_deref(), Some("prod-eu"));
    }

    #[test]
    fn suffixes_accounts_the_config_maps_onto_one_name() {
        let resolved = resolve_profile_names(
            [
                ("labflow-dev-customer", "111111111111"),
                ("labflow-saas-au", "222222222222"),
            ],
            &configured(&[("labflow-dev-customer", "dev"), ("labflow-saas-au", "dev")]),
        );

        assert_eq!(resolved[0].profile_name, "dev");
        assert_eq!(resolved[1].profile_name, "dev-222222222222");
        assert_eq!(resolved[1].wanted_name.as_deref(), Some("dev"));
    }

    #[test]
    fn resolves_in_the_order_the_accounts_arrive() {
        let resolved = resolve_profile_names(
            [("prod-eu", "222222222222"), ("Prod EU", "111111111111")],
            &configured(&[]),
        );

        assert_eq!(resolved[0].profile_name, "prod-eu");
        assert_eq!(resolved[0].account_id, "222222222222");
        assert_eq!(resolved[1].profile_name, "prod-eu-111111111111");
    }

    #[test]
    fn gives_every_account_a_distinct_name_when_even_the_suffix_clashes() {
        let resolved = resolve_profile_names(
            [
                ("one", "111111111111"),
                ("two", "222222222222"),
                ("three", "333333333333"),
            ],
            &configured(&[
                ("one", "dev"),
                ("two", "dev-333333333333"),
                ("three", "dev"),
            ]),
        );

        let names: Vec<&str> = resolved
            .iter()
            .map(|entry| entry.profile_name.as_str())
            .collect();

        assert_eq!(names, ["dev", "dev-333333333333", "dev-333333333333-2"]);
    }
}
