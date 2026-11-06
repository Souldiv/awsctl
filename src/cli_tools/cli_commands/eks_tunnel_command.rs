use anyhow::{anyhow, Context, Result};
use aws_sdk_ec2::types::Filter;
use inquire::Select;
use std::path::PathBuf;

use crate::aws::ec2::TaggedInstance;
use crate::aws::ssm::ensure_session_manager_plugin;
use crate::aws::{EC2Provider, EksProvider, SsmProvider};
use crate::cli_tools::bastion::{best_bastions, fallback_name_tag_patterns, BASTION_COMPONENT};
use crate::cli_tools::cli_commands::eks_command::select_cluster;
use crate::cli_tools::tunnel_kubeconfig::{
    context_name, default_kubeconfig_path, read_kubeconfig, validate_cluster_name, with_tunnel,
    write_kubeconfig, TunnelTarget,
};
use crate::cli_tools::tunnel_port::{default_local_port, ensure_port_free};

const EKS_API_PORT: i32 = 443;

pub struct TunnelOptions {
    pub profile: Option<String>,
    pub local_port: Option<u16>,
    pub bastion: Option<String>,
    pub account_label: Option<String>,
    pub kubeconfig: Option<PathBuf>,
}

pub struct TunnelProviders<'a> {
    pub eks: &'a EksProvider,
    pub ec2: &'a EC2Provider,
    pub ssm: &'a SsmProvider,
}

pub async fn open_tunnel(
    providers: TunnelProviders<'_>,
    region: &str,
    aws_profile: &str,
    cluster: Option<String>,
    options: TunnelOptions,
) -> Result<()> {
    ensure_session_manager_plugin()?;

    let cluster_name = select_cluster(providers.eks, cluster).await?;
    validate_cluster_name(&cluster_name)?;

    let local_port = options
        .local_port
        .unwrap_or_else(|| default_local_port(&cluster_name));
    ensure_port_free(local_port)?;

    let endpoint = providers
        .eks
        .describe_cluster_endpoint(&cluster_name)
        .await?;
    if !endpoint.private_access {
        println!(
            "Note: private endpoint access is off for {}, so the bastion reaches its public endpoint.",
            cluster_name
        );
    }

    let bastion_id = match options.bastion {
        Some(instance_id) => instance_id,
        None => {
            find_bastion(
                providers.ec2,
                &endpoint.vpc_id,
                options.account_label.as_deref(),
            )
            .await?
        }
    };
    providers.ssm.ensure_instance_online(&bastion_id).await?;

    let kubeconfig_path = match options.kubeconfig {
        Some(path) => path,
        None => default_kubeconfig_path(&cluster_name)?,
    };
    let target = TunnelTarget {
        cluster_name: &cluster_name,
        region,
        aws_profile,
        endpoint: &endpoint,
        local_port,
    };
    let kubeconfig = with_tunnel(read_kubeconfig(&kubeconfig_path)?, &target);
    write_kubeconfig(&kubeconfig_path, &kubeconfig)?;

    println!("bastion     {}", bastion_id);
    println!(
        "forwarding  127.0.0.1:{} -> {}:{}",
        local_port, endpoint.endpoint_host, EKS_API_PORT
    );
    println!(
        "kubeconfig  {} (context {})",
        kubeconfig_path.display(),
        context_name(&cluster_name)
    );
    println!(
        "In another terminal: KUBECONFIG={} kubectl --context {} get nodes",
        kubeconfig_path.display(),
        context_name(&cluster_name)
    );
    println!("Press Ctrl-C to close the tunnel.");

    providers
        .ssm
        .create_port_forward_session(
            &bastion_id,
            &endpoint.endpoint_host,
            i32::from(local_port),
            EKS_API_PORT,
        )
        .await?;

    println!("Tunnel to {} closed.", cluster_name);
    Ok(())
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
