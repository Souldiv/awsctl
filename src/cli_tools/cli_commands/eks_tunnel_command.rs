use anyhow::{anyhow, Context, Result};
use aws_config::SdkConfig;
use aws_sdk_ec2::types::Filter;
use inquire::Select;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::aws::ec2::TaggedInstance;
use crate::aws::eks::ClusterEndpoint;
use crate::aws::ssm::ensure_session_manager_plugin;
use crate::aws::{EC2Provider, EksProvider, SsmProvider};
use crate::cli_tools::bastion::{best_bastions, fallback_name_tag_patterns, BASTION_COMPONENT};
use crate::cli_tools::cli_commands::eks_command::select_cluster;
use crate::cli_tools::sdk_config::profile_sdk_config;
use crate::cli_tools::tunnel_credentials::{classify, FailureKind};
use crate::cli_tools::tunnel_keepalive::ApiProbe;
use crate::cli_tools::tunnel_kubeconfig::{
    context_name, home_dir, kubectl_kubeconfig_path, locked, read_kubeconfig,
    tunnel_kubeconfig_path, with_tunnels, without_tunnels, write_kubeconfig, TunnelTarget,
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

struct WrittenTunnel {
    cluster_name: String,
    region: String,
    aws_profile: String,
    endpoint: ClusterEndpoint,
    local_port: u16,
}

impl WrittenTunnel {
    fn of(tunnel: &PreparedTunnel) -> Self {
        WrittenTunnel {
            cluster_name: tunnel.cluster_name.clone(),
            region: tunnel.region.clone(),
            aws_profile: tunnel.aws_profile.clone(),
            endpoint: tunnel.endpoint.clone(),
            local_port: tunnel.local_port,
        }
    }

    fn target(&self) -> TunnelTarget<'_> {
        TunnelTarget {
            cluster_name: &self.cluster_name,
            region: &self.region,
            aws_profile: &self.aws_profile,
            endpoint: &self.endpoint,
            local_port: self.local_port,
        }
    }
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
    configured_kubeconfig: Option<&Path>,
    stored_role: StoredRoleAccount,
) -> Result<()> {
    ensure_session_manager_plugin()?;

    let home = home_dir()?;
    let kubectl_default = kubectl_kubeconfig_path(std::env::var_os("KUBECONFIG").as_deref(), &home);
    let kubeconfig = tunnel_kubeconfig_path(
        options.kubeconfig.as_deref(),
        configured_kubeconfig,
        &kubectl_default,
        &home,
    );
    let kubectl_reads_it = kubeconfig == kubectl_default;

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

    let mut prepared = Vec::with_capacity(targets.len());
    for (target, port) in targets.iter().zip(ports) {
        let tunnel = prepare_tunnel(target, port, &stored_role)
            .await
            .map_err(|error| explain_failure(error, target))?;
        prepared.push(tunnel);
    }

    let written: Vec<WrittenTunnel> = prepared.iter().map(WrittenTunnel::of).collect();
    write_tunnels(&kubeconfig, &written)?;

    let outcome = async {
        let current_context = read_kubeconfig(&kubeconfig)?.current_context;
        print_summary(
            &prepared,
            &kubeconfig,
            kubectl_reads_it,
            current_context.as_deref(),
        );
        supervise(prepared).await
    }
    .await;

    remove_written_tunnels(&kubeconfig, &written);
    outcome
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

    let probe = ApiProbe::new(&endpoint, local_port)?;

    Ok(PreparedTunnel {
        account: resolved.account,
        cluster_name: target.cluster.clone(),
        aws_profile: resolved.aws_profile,
        region: resolved.region,
        endpoint,
        bastion_id,
        local_port,
        probe,
    })
}

fn write_tunnels(path: &Path, tunnels: &[WrittenTunnel]) -> Result<()> {
    let targets: Vec<TunnelTarget> = tunnels.iter().map(WrittenTunnel::target).collect();

    locked(path, || {
        write_kubeconfig(path, &with_tunnels(read_kubeconfig(path)?, &targets))
    })
}

fn remove_written_tunnels(path: &Path, tunnels: &[WrittenTunnel]) {
    let targets: Vec<TunnelTarget> = tunnels.iter().map(WrittenTunnel::target).collect();
    let removal = locked(path, || {
        let existing = read_kubeconfig(path)?;
        let cleaned = without_tunnels(existing.clone(), &targets);
        let removed: Vec<String> = existing
            .contexts
            .into_iter()
            .map(|context| context.name)
            .filter(|name| !cleaned.contexts.iter().any(|kept| kept.name == *name))
            .collect();
        if !removed.is_empty() {
            write_kubeconfig(path, &cleaned)?;
        }
        Ok(removed)
    });

    match removal {
        Ok(removed) if removed.is_empty() => {}
        Ok(removed) => println!("Removed {} from {}.", removed.join(", "), path.display()),
        Err(error) => println!(
            "Warning: unable to remove the tunnel contexts from {}: {:#}",
            path.display(),
            error
        ),
    }
}

fn print_summary(
    tunnels: &[PreparedTunnel],
    kubeconfig: &Path,
    kubectl_reads_it: bool,
    current_context: Option<&str>,
) {
    for tunnel in tunnels {
        println!("{}", tunnel.cluster_name);
        println!("  bastion     {}", tunnel.bastion_id);
        println!(
            "  forwarding  127.0.0.1:{} -> {}:{}",
            tunnel.local_port, tunnel.endpoint.endpoint_host, EKS_API_PORT
        );
        println!("  context     {}", context_name(&tunnel.cluster_name));
    }

    let Some(first) = tunnels.first() else {
        return;
    };
    println!("Wrote the contexts into {}.", kubeconfig.display());
    println!(
        "In another terminal: {}",
        kubectl_hint(
            kubeconfig,
            kubectl_reads_it,
            &context_name(&first.cluster_name)
        )
    );
    let contexts: Vec<String> = tunnels
        .iter()
        .map(|tunnel| context_name(&tunnel.cluster_name))
        .collect();
    println!(
        "{}",
        current_context_note(kubeconfig, current_context, &contexts)
    );
    println!(
        "Tunnels reconnect on their own. Press Ctrl-C to close them; \
         'awsctl eks --tunnel-status' shows them from another terminal."
    );
}

fn kubectl_hint(kubeconfig: &Path, kubectl_reads_it: bool, context: &str) -> String {
    if kubectl_reads_it {
        format!("kubectl --context {} get nodes", context)
    } else {
        format!(
            "KUBECONFIG={} kubectl --context {} get nodes",
            kubeconfig.display(),
            context
        )
    }
}

fn current_context_note(
    kubeconfig: &Path,
    current_context: Option<&str>,
    tunnel_contexts: &[String],
) -> String {
    match current_context {
        Some(current) if tunnel_contexts.iter().any(|tunnel| tunnel == current) => format!(
            "The current-context of {} is {}; tools that take no --context use it.",
            kubeconfig.display(),
            current
        ),
        Some(current) => format!(
            "The current-context of {} stays {}; tools that take no --context, such as Radar, \
             use it, not a tunnel.",
            kubeconfig.display(),
            current
        ),
        None => format!(
            "{} has no current-context; tools that take no --context, such as Radar, need one.",
            kubeconfig.display()
        ),
    }
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
    use super::{current_context_note, explain_failure, kubectl_hint, tunnel_group};
    use crate::cli_tools::tunnel_target::parse_target;
    use crate::config::tunnel_group::TunnelGroupEntry;
    use anyhow::anyhow;
    use std::collections::BTreeMap;
    use std::path::Path;

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

    #[test]
    fn warns_when_the_current_context_is_not_a_tunnel() {
        let note = current_context_note(
            Path::new("/home/me/.kube/config"),
            Some("arn:aws:eks:eu-west-2:000000000000:cluster/example-eks"),
            &[String::from("ssm-example-eks")],
        );

        assert_eq!(
            note,
            "The current-context of /home/me/.kube/config stays \
             arn:aws:eks:eu-west-2:000000000000:cluster/example-eks; tools that take no \
             --context, such as Radar, use it, not a tunnel."
        );
    }

    #[test]
    fn needs_no_kubeconfig_prefix_for_the_file_kubectl_reads() {
        assert_eq!(
            kubectl_hint(Path::new("/home/me/.kube/config"), true, "ssm-example-eks"),
            "kubectl --context ssm-example-eks get nodes"
        );
    }

    #[test]
    fn names_any_other_kubeconfig_in_the_hint() {
        assert_eq!(
            kubectl_hint(
                Path::new("/home/me/.kube/tunnels.yaml"),
                false,
                "ssm-example-eks"
            ),
            "KUBECONFIG=/home/me/.kube/tunnels.yaml kubectl --context ssm-example-eks get nodes"
        );
    }

    #[test]
    fn says_when_the_current_context_is_a_tunnel() {
        let note = current_context_note(
            Path::new("/home/me/.kube/ssm-example-eks.yaml"),
            Some("ssm-example-eks"),
            &[String::from("ssm-example-eks")],
        );

        assert!(note.contains("is ssm-example-eks; tools that take no --context use it."));
    }
}
