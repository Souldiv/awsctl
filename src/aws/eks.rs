use anyhow::Result;
use aws_config::SdkConfig;
use aws_sdk_eks::client::Client;

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
}
