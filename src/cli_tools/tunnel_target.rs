use anyhow::{anyhow, Result};
use std::collections::BTreeSet;

use crate::cli_tools::tunnel_kubeconfig::validate_cluster_name;
use crate::config::tunnel_group::{TunnelGroupEntry, TunnelGroupTable};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetSpec {
    pub cluster: String,
    pub profile: Option<String>,
    pub region: Option<String>,
    pub local_port: Option<u16>,
    pub bastion: Option<String>,
    pub account_label: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TargetDefaults {
    pub profile: Option<String>,
    pub region: Option<String>,
    pub local_port: Option<u16>,
    pub bastion: Option<String>,
    pub account_label: Option<String>,
}

pub fn parse_target(value: &str) -> Result<TargetSpec> {
    let (profile, cluster) = match value.rsplit_once(':') {
        Some((profile, _)) if profile.trim().is_empty() => {
            return Err(anyhow!(
            "'{}' names no profile before the ':'. Write <profile>:<cluster> or just <cluster>.",
            value
        ))
        }
        Some((profile, cluster)) => (Some(profile.trim().to_string()), cluster.trim()),
        None => (None, value.trim()),
    };
    validate_cluster_name(cluster)?;

    Ok(TargetSpec {
        cluster: cluster.to_string(),
        profile,
        ..TargetSpec::default()
    })
}

pub fn resolve_targets(
    cli_targets: &[String],
    group: &[TunnelGroupEntry],
    defaults: &TargetDefaults,
) -> Result<Vec<TargetSpec>> {
    let from_group = group.iter().map(from_group_entry);
    let from_cli = cli_targets.iter().map(|value| parse_target(value));
    let named: Vec<TargetSpec> = from_group.chain(from_cli).collect::<Result<_>>()?;

    let single_tunnel_flags = defaults.local_port.is_some() || defaults.bastion.is_some();
    if named.len() > 1 && single_tunnel_flags {
        return Err(anyhow!(
            "--local-port and --bastion apply to one tunnel. Set local_port and bastion \
             per cluster in a [tunnel_groups] table instead."
        ));
    }
    reject_duplicate_clusters(&named)?;

    Ok(named
        .into_iter()
        .map(|target| with_defaults(target, defaults))
        .collect())
}

pub fn with_defaults(target: TargetSpec, defaults: &TargetDefaults) -> TargetSpec {
    TargetSpec {
        profile: target.profile.or_else(|| defaults.profile.clone()),
        region: target.region.or_else(|| defaults.region.clone()),
        local_port: target.local_port.or(defaults.local_port),
        bastion: target.bastion.or_else(|| defaults.bastion.clone()),
        account_label: target
            .account_label
            .or_else(|| defaults.account_label.clone()),
        ..target
    }
}

fn from_group_entry(entry: &TunnelGroupEntry) -> Result<TargetSpec> {
    match entry {
        TunnelGroupEntry::Target(value) => parse_target(value),
        TunnelGroupEntry::Detailed(table) => from_group_table(table),
    }
}

fn from_group_table(table: &TunnelGroupTable) -> Result<TargetSpec> {
    validate_cluster_name(&table.cluster)?;

    Ok(TargetSpec {
        cluster: table.cluster.clone(),
        profile: table.profile.clone(),
        region: table.region.clone(),
        local_port: table.local_port,
        bastion: table.bastion.clone(),
        account_label: table.account_label.clone(),
    })
}

fn reject_duplicate_clusters(targets: &[TargetSpec]) -> Result<()> {
    let mut seen = BTreeSet::new();
    match targets
        .iter()
        .find(|target| !seen.insert(target.cluster.as_str()))
    {
        Some(duplicate) => Err(anyhow!(
            "Cluster '{}' is named twice. Its context and kubeconfig are named after the cluster, \
             so open a tunnel to the second one from a separate awsctl with --kubeconfig.",
            duplicate.cluster
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_target, resolve_targets, TargetDefaults, TargetSpec};
    use crate::config::tunnel_group::{TunnelGroupEntry, TunnelGroupTable};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_a_bare_cluster() {
        let target = parse_target("example-eks").expect("parses");

        assert_eq!(target.cluster, "example-eks");
        assert_eq!(target.profile, None);
    }

    #[test]
    fn parses_a_profile_before_the_cluster() {
        let target = parse_target("dev:example-eks").expect("parses");

        assert_eq!(target.cluster, "example-eks");
        assert_eq!(target.profile.as_deref(), Some("dev"));
    }

    #[test]
    fn splits_on_the_last_colon() {
        let target = parse_target("team:dev:example-eks").expect("parses");

        assert_eq!(target.profile.as_deref(), Some("team:dev"));
        assert_eq!(target.cluster, "example-eks");
    }

    #[test]
    fn rejects_an_empty_profile_or_cluster() {
        assert!(parse_target(":example-eks").is_err());
        assert!(parse_target("dev:").is_err());
        assert!(parse_target("").is_err());
        assert!(parse_target("dev:../evil").is_err());
    }

    #[test]
    fn fills_in_the_command_line_profile_and_region_where_a_target_has_none() {
        let defaults = TargetDefaults {
            profile: Some(String::from("dev")),
            region: Some(String::from("eu-west-2")),
            ..TargetDefaults::default()
        };

        let targets = resolve_targets(
            &strings(&["example-eks", "prod:example-k8s"]),
            &[],
            &defaults,
        )
        .expect("resolves");

        assert_eq!(targets[0].profile.as_deref(), Some("dev"));
        assert_eq!(targets[1].profile.as_deref(), Some("prod"));
        assert!(targets
            .iter()
            .all(|target| target.region.as_deref() == Some("eu-west-2")));
    }

    #[test]
    fn puts_group_entries_before_command_line_clusters() {
        let group = [
            TunnelGroupEntry::Target(String::from("dev:example-eks")),
            TunnelGroupEntry::Detailed(TunnelGroupTable {
                cluster: String::from("example-k8s"),
                profile: Some(String::from("prod")),
                region: Some(String::from("us-east-1")),
                local_port: Some(21443),
                bastion: Some(String::from("i-0123456789abcdef0")),
                account_label: None,
            }),
        ];

        let targets = resolve_targets(&strings(&["other-eks"]), &group, &TargetDefaults::default())
            .expect("resolves");

        let clusters: Vec<&str> = targets.iter().map(|t| t.cluster.as_str()).collect();
        assert_eq!(clusters, ["example-eks", "example-k8s", "other-eks"]);
        assert_eq!(
            targets[1],
            TargetSpec {
                cluster: String::from("example-k8s"),
                profile: Some(String::from("prod")),
                region: Some(String::from("us-east-1")),
                local_port: Some(21443),
                bastion: Some(String::from("i-0123456789abcdef0")),
                account_label: None,
            }
        );
    }

    #[test]
    fn applies_local_port_and_bastion_to_a_single_target() {
        let defaults = TargetDefaults {
            local_port: Some(21443),
            bastion: Some(String::from("i-0123456789abcdef0")),
            ..TargetDefaults::default()
        };

        let targets =
            resolve_targets(&strings(&["example-eks"]), &[], &defaults).expect("resolves");

        assert_eq!(targets[0].local_port, Some(21443));
        assert_eq!(targets[0].bastion.as_deref(), Some("i-0123456789abcdef0"));
    }

    #[test]
    fn refuses_local_port_or_bastion_for_several_targets() {
        let defaults = TargetDefaults {
            local_port: Some(21443),
            ..TargetDefaults::default()
        };

        let error = resolve_targets(&strings(&["example-eks", "example-k8s"]), &[], &defaults)
            .expect_err("refuses");

        assert!(error.to_string().contains("--local-port"));
    }

    #[test]
    fn refuses_the_same_cluster_twice() {
        let error = resolve_targets(
            &strings(&["dev:example-eks", "prod:example-eks"]),
            &[],
            &TargetDefaults::default(),
        )
        .expect_err("refuses");

        assert!(error.to_string().contains("named twice"));
    }

    #[test]
    fn resolves_nothing_when_no_cluster_is_named() {
        let targets = resolve_targets(&[], &[], &TargetDefaults::default()).expect("resolves");

        assert!(targets.is_empty());
    }
}
