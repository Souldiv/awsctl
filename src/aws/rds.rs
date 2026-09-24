use anyhow::Result;
use aws_config::SdkConfig;
use aws_sdk_rds::types::DbInstance;
use aws_sdk_rds::Client;
use std::fmt::Display;

pub struct RdsProvider {
    client: Client,
}

pub struct RdsInstance {
    pub name: String,
    pub endpoint: String,
    pub port: i32,
}

impl Display for RdsInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl RdsProvider {
    pub fn new(sdk_config: &SdkConfig) -> Self {
        RdsProvider {
            client: Client::new(sdk_config),
        }
    }

    pub async fn list_rds_instances(&self) -> Result<Vec<RdsInstance>> {
        let response = self.client.describe_db_instances().send().await?;

        let mut list_of_db_instances: Vec<DbInstance> = vec![];
        list_of_db_instances.extend_from_slice(response.db_instances());

        if response.marker.is_some() {
            let mut next_token = response.marker;
            while let Some(token) = next_token {
                let output = self
                    .client
                    .describe_db_instances()
                    .marker(&token)
                    .send()
                    .await?;
                list_of_db_instances.extend_from_slice(output.db_instances());
                next_token = output.marker;
            }
        }

        let list_of_rds_instances: Vec<RdsInstance> = list_of_db_instances
            .iter()
            .map(|db_instance| RdsInstance {
                name: db_instance.db_name().unwrap().to_string(),
                endpoint: db_instance
                    .endpoint()
                    .unwrap()
                    .address()
                    .unwrap()
                    .to_string(),
                port: db_instance.endpoint().unwrap().port().unwrap(),
            })
            .collect();

        Ok(list_of_rds_instances)
    }
}
