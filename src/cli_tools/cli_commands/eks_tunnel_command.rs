use anyhow::{anyhow, Context, Result};
use aws_config::SdkConfig;
use aws_sdk_ec2::types::Filter;
use inquire::Select;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::aws::ec2::TaggedInstance;
use crate::aws::ssm::ensure_session_manager_plugin;
use crate::aws::{EC2Provider, EksProvider, SsmProvider};
use crate::cli_tools::bastion::{best_bastions, fallback_name_tag_patterns, BASTION_COMPONENT};
use crate::cli_tools::cli_commands::eks_command::select_cluster;
use crate::cli_tools::sdk_config::profile_sdk_config;
use crate::cli_tools::tunnel_credentials::{classify, FailureKind};
use crate::cli_tools::tunnel_keepalive::ApiProbe;
use crate::cli_tools::tunnel_kubeconfig::{
    context_name, default_kubeconfig_path, default_merged_kubeconfig_path, read_kubeconfig,
    with_tunnel, with_tunnels, write_kubeconfig, TunnelTarget,
};
use crate::cli_tools::tunnel_port::{assign_ports, ensure_port_free};
use crate::cli_tools::tunnel_status::{
    process_is_alive, remove_stale, render_statuses, scan_statuses, status_dir, unix_now,
};
use crate::cli_tools::tunnel_supervisor::{supervise, PreparedTunnel, TunnelAccount, EKS_API_PORT};
use crate::cli_tools::tunnel_target::{resolve_targets, with_defaults, TargetDefaults, TargetSpec};
use crate::config::tunnel_group::TunnelGroupEntry;

pub struct TunnelOptions {
    pub profile: Option<String>,
    pub local_port: Option<u16>,
    pub bastion: Option<String>,
    pub account_label: Option<String>,
    pub kubeconfig: Option<PathBuf>,
    pub group: Option<String>,
}

pub struct StoredRoleAccount {
    pub region: Option<String>,
    pub profile_name: String,
}

struct ResolvedAccount {
    account: TunnelAccount,
    aws_profile: String,
    region: String,
    config: SdkConfig,
}

pub async fn open_tunnels(
    clusters: &[String],
    region: Option<String>,
    options: TunnelOptions,
    groups: &BTreeMap<String, Vec<TunnelGroupEntry>>,
    stored_role: StoredRoleAccount,
) -> Result<()> {
    ensure_session_manager_plugin()?;

    let defaults = TargetDefaults {
        profile: options.profile.clone(),
        region,
        local_port: options.local_port,
        bastion: options.bastion.clone(),
        account_label: options.account_label.clone(),
    };
    let group = tunnel_group(groups, options.group.as_deref())?;
    let named = resolve_targets(clusters, group, &defaults)?;
    let targets = if named.is_empty() {
        vec![select_target(&defaults, &stored_role).await?]
    } else {
        named
    };

    let requests: Vec<(&str, Option<u16>)> = targets
        .iter()
        .map(|target| (target.cluster.as_str(), target.local_port))
        .collect();
    let ports = assign_ports(&requests)?;
    ports.iter().try_for_each(|port| ensure_port_free(*port))?;

    let single = targets.len() == 1;
    let mut prepared = Vec::with_capacity(targets.len());
    for (target, port) in targets.iter().zip(ports) {
        let kubeconfig = match (&options.kubeconfig, single) {
            (Some(path), true) => path.clone(),
            _ => default_kubeconfig_path(&target.cluster)?,
        };
        let tunnel = prepare_tunnel(target, port, kubeconfig, &stored_role)
            .await
            .map_err(|error| explain_failure(error, target))?;
        prepared.push(tunnel);
    }

    let merged = if single {
        None
    } else {
        let path = match &options.kubeconfig {
            Some(path) => path.clone(),
            None => default_merged_kubeconfig_path()?,
        };
        write_merged_kubeconfig(&path, &prepared)?;
        Some(path)
    };

    print_summary(&prepared, merged.as_deref());
    supervise(prepared).await
}

pub fn show_status() -> Result<()> {
    let dir = status_dir()?;
    let scan = scan_statuses(&dir, process_is_alive)?;
    remove_stale(&scan.stale);

    print!("{}", render_statuses(&scan.live, unix_now()));
    if !scan.stale.is_empty() {
        println!(
            "Removed {} status file(s) left by an awsctl that is no longer running.",
            scan.stale.len()
        );
    }
    Ok(())
}

fn explain_failure(error: anyhow::Error, target: &TargetSpec) -> anyhow::Error {
    match classify(&error) {
        FailureKind::NeedsLogin => {
            let account = match &target.profile {
                Some(profile) => TunnelAccount::Profile(profile.clone()),
                None => TunnelAccount::StoredRole,
            };
            error.context(format!(
                "The credentials for the tunnel to {} are expired or missing; {}.",
                target.cluster,
                account.login_hint()
            ))
        }
        FailureKind::Transient => error.context(format!(
            "Unable to prepare the tunnel to {}.",
            target.cluster
        )),
    }
}

fn tunnel_group<'a>(
    groups: &'a BTreeMap<String, Vec<TunnelGroupEntry>>,
    name: Option<&str>,
) -> Result<&'a [TunnelGroupEntry]> {
    let Some(name) = name else {
        return Ok(&[]);
    };
    match groups.get(name) {
        Some(entries) if !entries.is_empty() => Ok(entries),
        Some(_) => Err(anyhow!("Tunnel group '{}' lists no clusters.", name)),
        None => {
            let known: Vec<&str> = groups.keys().map(String::as_str).collect();
            Err(anyhow!(
                "No tunnel group '{}' is in [tunnel_groups] of awsctl's config (known: {}). \
                 Run 'awsctl config edit' to add one.",
                name,
                if known.is_empty() {
                    String::from("none")
                } else {
                    known.join(", ")
                }
            ))
        }
    }
}

async fn select_target(
    defaults: &TargetDefaults,
    stored_role: &StoredRoleAccount,
) -> Result<TargetSpec> {
    let resolved = resolve_account(
        defaults.profile.as_deref(),
        defaults.region.as_deref(),
        stored_role,
    )
    .await?;
    let cluster = select_cluster(&EksProvider::new(&resolved.config), None).await?;

    Ok(with_defaults(
        TargetSpec {
            cluster,
            ..TargetSpec::default()
        },
        defaults,
    ))
}

async fn resolve_account(
    profile: Option<&str>,
    region: Option<&str>,
    stored_role: &StoredRoleAccount,
) -> Result<ResolvedAccount> {
    let (account, aws_profile) = match profile {
        Some(profile) => (
            TunnelAccount::Profile(profile.to_string()),
            profile.to_string(),
        ),
        None => (TunnelAccount::StoredRole, stored_role.profile_name.clone()),
    };
    let region = match (region, &account) {
        (Some(region), _) => region.to_string(),
        (None, TunnelAccount::Profile(profile)) => profile_sdk_config(profile, None)
            .await
            .region()
            .map(|region| region.to_string())
            .ok_or_else(|| anyhow!("Profile '{}' sets no region. Pass --region.", profile))?,
        (None, TunnelAccount::StoredRole) => stored_role
            .region
            .clone()
            .ok_or_else(|| anyhow!("Please set your profile and try again."))?,
    };
    let config = account.load_sdk_config(&region).await?;

    Ok(ResolvedAccount {
        account,
        aws_profile,
        region,
        config,
    })
}

async fn prepare_tunnel(
    target: &TargetSpec,
    local_port: u16,
    kubeconfig: PathBuf,
    stored_role: &StoredRoleAccount,
) -> Result<PreparedTunnel> {
    let resolved = resolve_account(
        target.profile.as_deref(),
        target.region.as_deref(),
        stored_role,
    )
    .await?;
    let eks = EksProvider::new(&resolved.config);
    let ec2 = EC2Provider::new(&resolved.config);
    let ssm = SsmProvider::new(&resolved.config);

    let endpoint = eks.describe_cluster_endpoint(&target.cluster).await?;
    if !endpoint.private_access {
        println!(
            "Note: private endpoint access is off for {}, so the bastion reaches its public endpoint.",
            target.cluster
        );
    }

    let bastion_id = match &target.bastion {
        Some(instance_id) => instance_id.clone(),
        None => find_bastion(&ec2, &endpoint.vpc_id, target.account_label.as_deref()).await?,
    };
    ssm.ensure_instance_online(&bastion_id).await?;

    let tunnel_target = TunnelTarget {
        cluster_name: &target.cluster,
        region: &resolved.region,
        aws_profile: &resolved.aws_profile,
        endpoint: &endpoint,
        local_port,
    };
    let updated = with_tunnel(read_kubeconfig(&kubeconfig)?, &tunnel_target);
    write_kubeconfig(&kubeconfig, &updated)?;
    let probe = ApiProbe::new(&endpoint, local_port)?;

    Ok(PreparedTunnel {
        account: resolved.account,
        cluster_name: target.cluster.clone(),
        aws_profile: resolved.aws_profile,
        region: resolved.region,
        endpoint,
        bastion_id,
        local_port,
        kubeconfig,
        probe,
    })
}

fn write_merged_kubeconfig(path: &Path, tunnels: &[PreparedTunnel]) -> Result<()> {
    let targets: Vec<TunnelTarget> = tunnels
        .iter()
        .map(|tunnel| TunnelTarget {
            cluster_name: &tunnel.cluster_name,
            region: &tunnel.region,
            aws_profile: &tunnel.aws_profile,
            endpoint: &tunnel.endpoint,
            local_port: tunnel.local_port,
        })
        .collect();

    write_kubeconfig(path, &with_tunnels(read_kubeconfig(path)?, &targets))
}

fn print_summary(tunnels: &[PreparedTunnel], merged: Option<&Path>) {
    for tunnel in tunnels {
        println!("{}", tunnel.cluster_name);
        println!("  bastion     {}", tunnel.bastion_id);
        println!(
            "  forwarding  127.0.0.1:{} -> {}:{}",
            tunnel.local_port, tunnel.endpoint.endpoint_host, EKS_API_PORT
        );
        println!(
            "  kubeconfig  {} (context {})",
            tunnel.kubeconfig.display(),
            context_name(&tunnel.cluster_name)
        );
    }

    let (kubeconfig, context) = match (merged, tunnels) {
        (Some(path), [first, ..]) => {
            println!("Every context is also in {}.", path.display());
            (path.to_path_buf(), context_name(&first.cluster_name))
        }
        (None, [only]) => (only.kubeconfig.clone(), context_name(&only.cluster_name)),
        _ => return,
    };
    println!(
        "In another terminal: KUBECONFIG={} kubectl --context {} get nodes",
        kubeconfig.display(),
        context
    );
    println!(
        "Tunnels reconnect on their own. Press Ctrl-C to close them; \
         'awsctl eks --tunnel-status' shows them from another terminal."
    );
}

async fn find_bastion(
    ec2: &EC2Provider,
    vpc_id: &str,
    account_label: Option<&str>,
) -> Result<String> {
    let by_component = ec2
        .list_running_instances_in_vpc(
            vpc_id,
            Filter::builder()
                .name("tag:Component")
                .values(BASTION_COMPONENT)
                .build(),
        )
        .await?;
    let found = if by_component.is_empty() {
        ec2.list_running_instances_in_vpc(
            vpc_id,
            Filter::builder()
                .name("tag:Name")
                .set_values(Some(fallback_name_tag_patterns()))
                .build(),
        )
        .await?
    } else {
        by_component
    };

    let candidates: Vec<TaggedInstance> = best_bastions(&found, account_label);
    match candidates.as_slice() {
        [] => Err(anyhow!(
            "No running bastion was found in {}. Tag one Component=bastion, \
             or pass --bastion <instance-id>.",
            vpc_id
        )),
        [only] => Ok(only.instance_id.clone()),
        _ => Select::new("Select the bastion to tunnel through: ", candidates)
            .prompt()
            .map(|selected| selected.instance_id)
            .with_context(|| "There was an error displaying bastions. Please try again."),
    }
}

#[cfg(test)]
mod tests {
    use super::{explain_failure, tunnel_group};
    use crate::cli_tools::tunnel_target::parse_target;
    use crate::config::tunnel_group::TunnelGroupEntry;
    use anyhow::anyhow;
    use std::collections::BTreeMap;

    #[test]
    fn tells_how_to_log_in_when_setup_meets_expired_credentials() {
        let target = parse_target("dev:example-eks").expect("parses");

        let error = explain_failure(anyhow!("the SSO token has expired"), &target);

        assert!(error.to_string().contains("aws sso login --profile dev"));
    }

    #[test]
    fn names_the_cluster_when_setup_fails_otherwise() {
        let target = parse_target("example-eks").expect("parses");

        let error = explain_failure(anyhow!("cluster not found"), &target);

        assert_eq!(
            error.to_string(),
            "Unable to prepare the tunnel to example-eks."
        );
    }

    fn groups() -> BTreeMap<String, Vec<TunnelGroupEntry>> {
        BTreeMap::from([
            (
                String::from("work"),
                vec![TunnelGroupEntry::Target(String::from("dev:example-eks"))],
            ),
            (String::from("empty"), vec![]),
        ])
    }

    #[test]
    fn finds_a_named_group() {
        assert_eq!(
            tunnel_group(&groups(), Some("work")).expect("found").len(),
            1
        );
        assert!(tunnel_group(&groups(), None).expect("none").is_empty());
    }

    #[test]
    fn names_the_known_groups_when_one_is_missing() {
        let error = tunnel_group(&groups(), Some("home")).expect_err("missing");

        assert!(error.to_string().contains("known: empty, work"));
    }

    #[test]
    fn refuses_an_empty_group() {
        assert!(tunnel_group(&groups(), Some("empty")).is_err());
    }
}
