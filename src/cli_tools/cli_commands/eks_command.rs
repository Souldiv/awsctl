use crate::aws::eks::EksProvider;
use anyhow::{Context, Result};
use inquire::Select;
use std::process::Command;

pub async fn update_kubeconfig(
    eks: &EksProvider,
    region: &str,
    cluster: Option<String>,
) -> Result<()> {
    let selected_cluster = match cluster {
        Some(cluster) => cluster,
        None => {
            let mut clusters = eks.list_clusters().await?;

            clusters.sort_by(|a, b| a.cmp(&b));

            Select::new("Select Cluster: ", clusters)
                .prompt()
                .with_context(|| "There was an error displaying clusters. Please try again.")?
        }
    };

    let mut aws_eks_command = Command::new("aws");
    // temporary fix for the issue since update-kubeconfig method does not exist in aws-sdk-eks
    // aws eks --region $region update-kubeconfig --name $cluster_name
    // fix: https://stackoverflow.com/questions/54953190/amazon-eks-generate-update-kubeconfig-via-python-script

    let run_command_output = aws_eks_command
        .args([
            "eks",
            "update-kubeconfig",
            "--region",
            region,
            "--name",
            &selected_cluster,
        ])
        .spawn()?;

    let output = run_command_output.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "Failed to update kubeconfig for cluster '{}' in region '{}'.",
            selected_cluster,
            region
        );
    }
    println!("Cluster was successfully updated");
    Ok(())
}
