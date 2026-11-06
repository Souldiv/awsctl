use anyhow::{anyhow, Context, Result};
use aws_config::SdkConfig;
use aws_sdk_eks::client::Client;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterEndpoint {
    pub endpoint_host: String,
    pub certificate_authority_data: String,
    pub vpc_id: String,
    pub private_access: bool,
}

pub struct EksProvider {
    client: Client,
}

impl EksProvider {
    pub fn new(sdk_config: &SdkConfig) -> Self {
        EksProvider {
            client: Client::new(sdk_config),
        }
    }

    pub async fn list_clusters(&self) -> Result<Vec<String>> {
        let clusters = self.client.list_clusters().max_results(20).send().await?;

        let mut list_of_clusters = vec![];
        list_of_clusters.extend_from_slice(clusters.clusters());

        if clusters.next_token.is_some() {
            let mut next_token = clusters.next_token;
            while let Some(token) = next_token {
                let output = self
                    .client
                    .list_clusters()
                    .max_results(20)
                    .next_token(&token)
                    .send()
                    .await?;
                list_of_clusters.extend_from_slice(output.clusters());
                next_token = output.next_token;
            }
        }

        Ok(list_of_clusters)
    }

    pub async fn describe_cluster_endpoint(&self, cluster_name: &str) -> Result<ClusterEndpoint> {
        let output = self
            .client
            .describe_cluster()
            .name(cluster_name)
            .send()
            .await
            .with_context(|| format!("Unable to describe the EKS cluster '{}'.", cluster_name))?;
        let cluster = output
            .cluster()
            .ok_or_else(|| anyhow!("EKS returned no details for cluster '{}'.", cluster_name))?;

        let endpoint = cluster.endpoint().ok_or_else(|| {
            anyhow!(
                "Cluster '{}' has no API endpoint yet; is it still being created?",
                cluster_name
            )
        })?;
        let certificate_authority_data = cluster
            .certificate_authority()
            .and_then(|authority| authority.data())
            .ok_or_else(|| anyhow!("Cluster '{}' has no certificate authority.", cluster_name))?;
        let vpc_config = cluster
            .resources_vpc_config()
            .ok_or_else(|| anyhow!("Cluster '{}' has no VPC configuration.", cluster_name))?;
        let vpc_id = vpc_config
            .vpc_id()
            .ok_or_else(|| anyhow!("Cluster '{}' has no VPC id.", cluster_name))?;

        Ok(ClusterEndpoint {
            endpoint_host: endpoint_host(endpoint)?,
            certificate_authority_data: certificate_authority_data.to_string(),
            vpc_id: vpc_id.to_string(),
            private_access: vpc_config.endpoint_private_access(),
        })
    }
}

fn endpoint_host(endpoint: &str) -> Result<String> {
    let host = endpoint
        .strip_prefix("https://")
        .unwrap_or(endpoint)
        .trim_end_matches('/');

    if host.is_empty() || host.contains(['/', ':', '@']) {
        return Err(anyhow!("Unexpected EKS endpoint '{}'.", endpoint));
    }

    Ok(host.to_string())
}

#[cfg(test)]
mod tests {
    use super::endpoint_host;

    #[test]
    fn strips_the_scheme_from_the_endpoint() {
        assert_eq!(
            endpoint_host("https://EXAMPLE.gr7.eu-west-2.eks.amazonaws.com").expect("host"),
            "EXAMPLE.gr7.eu-west-2.eks.amazonaws.com"
        );
    }

    #[test]
    fn rejects_an_endpoint_with_a_path_or_port() {
        assert!(endpoint_host("https://example.com:8443").is_err());
        assert!(endpoint_host("https://example.com/path").is_err());
        assert!(endpoint_host("https://").is_err());
    }
}
