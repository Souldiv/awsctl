use anyhow::{Context, Result};
use aws_sdk_ssm::types::InstanceInformation;
use inquire::Select;

use crate::aws::ec2::InstanceNameTag;
use crate::aws::rds::RdsInstance;
use crate::aws::EC2Provider;
use crate::aws::RdsProvider;
use crate::aws::SsmProvider;

pub async fn ssh_into_ec2_instance(ssm: &SsmProvider, ec2: &EC2Provider) -> Result<()> {
    let ssm_response: Vec<InstanceInformation> = ssm.list_ssm_managed_instances().await?;
    let instance_ids: Vec<String> = ssm_response
        .iter()
        .flat_map(|instance| instance.instance_id())
        .map(|id| id.to_string())
        .collect();
    let ec2_response: Vec<InstanceNameTag> =
        ec2.list_name_tags_of_instances(Some(instance_ids)).await?;

    if ec2_response.is_empty() {
        anyhow::bail!("No instances found");
    }

    let selected_instance = Select::new("Select Instance to SSH: ", ec2_response)
        .prompt()
        .with_context(|| "There was an error displaying instances. Please try again.")?;

    ssm.create_ssh_session(&selected_instance.instance_id)
        .await?;
    Ok(())
}

pub async fn ssh_into_rds_instance(
    ssm: &SsmProvider,
    ec2: &EC2Provider,
    rds: &RdsProvider,
    local_port: i32,
) -> Result<()> {
    let ssm_response: Vec<InstanceInformation> = ssm.list_ssm_managed_instances().await?;
    if ssm_response.is_empty() {
        anyhow::bail!("No instances found");
    }

    let instance_ids: Vec<String> = ssm_response
        .iter()
        .flat_map(|instance| instance.instance_id())
        .map(|id| id.to_string())
        .collect();

    let ec2_response: Vec<InstanceNameTag> =
        ec2.list_name_tags_of_instances(Some(instance_ids)).await?;

    let selected_instance = if ec2_response.len() > 1 {
        Select::new("Select Instance to connect to RDS: ", ec2_response)
            .prompt()
            .with_context(|| "There was an error displaying instances. Please try again.")?
    } else {
        ec2_response.get(0).unwrap().clone()
    };

    let rds_response: Vec<RdsInstance> = rds.list_rds_instances().await?;
    let selected_rds_instance = Select::new("Select RDS to connect to: ", rds_response)
        .prompt()
        .with_context(|| "There was an error displaying RDS instances. Please try again.")?;

    let _ = ssm
        .create_port_forward_session(
            &selected_instance.instance_id,
            &selected_rds_instance.endpoint,
            local_port,
            selected_rds_instance.port,
        )
        .await?;

    Ok(())
}
