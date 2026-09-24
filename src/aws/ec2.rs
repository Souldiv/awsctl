use aws_config::SdkConfig;
use aws_sdk_ec2::types::{Filter, TagDescription};
use aws_sdk_ec2::Client;

use anyhow::Result;
use std::fmt::Display;

pub struct EC2Provider {
    client: Client,
}
#[derive(Debug, Clone)]
pub struct InstanceNameTag {
    pub instance_id: String,
    pub instance_name: String,
}

impl Display for InstanceNameTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.instance_name)
    }
}

impl EC2Provider {
    pub fn new(sdk_config: &SdkConfig) -> Self {
        EC2Provider {
            client: Client::new(sdk_config),
        }
    }

    pub async fn list_name_tags_of_instances(
        &self,
        instance_ids: Option<Vec<String>>,
    ) -> Result<Vec<InstanceNameTag>> {
        let id_filter = Filter::builder()
            .name("resource-id")
            .set_values(instance_ids)
            .build();
        let key_filter = Filter::builder().name("key").values("Name").build();
        let tag_output = self
            .client
            .describe_tags()
            .set_filters(Some(vec![id_filter, key_filter]))
            .send()
            .await?;
        let mut list_of_tags: Vec<TagDescription> = vec![];
        list_of_tags.extend_from_slice(tag_output.tags());

        if tag_output.next_token.is_some() {
            let mut next_token = tag_output.next_token;
            while let Some(token) = next_token {
                let output = self
                    .client
                    .describe_tags()
                    .next_token(&token)
                    .send()
                    .await?;
                list_of_tags.extend_from_slice(output.tags());
                next_token = output.next_token;
            }
        }

        let instance_name_list = list_of_tags
            .iter()
            .map(|tag| InstanceNameTag {
                instance_id: tag.resource_id().unwrap().to_string(),
                instance_name: tag.value().unwrap().to_string(),
            })
            .collect();

        Ok(instance_name_list)
    }
}
