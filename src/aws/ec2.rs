use aws_config::SdkConfig;
use aws_sdk_ec2::types::{Filter, Instance, TagDescription};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedInstance {
    pub instance_id: String,
    pub name: Option<String>,
    pub component: Option<String>,
}

impl Display for TaggedInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{} ({})", name, self.instance_id),
            None => write!(f, "{}", self.instance_id),
        }
    }
}

impl From<&Instance> for TaggedInstance {
    fn from(instance: &Instance) -> Self {
        let tag_value = |key: &str| {
            instance
                .tags()
                .iter()
                .find(|tag| tag.key() == Some(key))
                .and_then(|tag| tag.value())
                .map(str::to_string)
        };

        TaggedInstance {
            instance_id: instance.instance_id().unwrap_or_default().to_string(),
            name: tag_value("Name"),
            component: tag_value("Component"),
        }
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

    pub async fn list_running_instances_in_vpc(
        &self,
        vpc_id: &str,
        tag_filter: Filter,
    ) -> Result<Vec<TaggedInstance>> {
        let filters = vec![
            Filter::builder().name("vpc-id").values(vpc_id).build(),
            Filter::builder()
                .name("instance-state-name")
                .values("running")
                .build(),
            tag_filter,
        ];

        let mut instances: Vec<TaggedInstance> = vec![];
        let mut next_token: Option<String> = None;
        loop {
            let output = self
                .client
                .describe_instances()
                .set_filters(Some(filters.clone()))
                .set_next_token(next_token)
                .send()
                .await?;
            instances.extend(
                output
                    .reservations()
                    .iter()
                    .flat_map(|reservation| reservation.instances())
                    .map(TaggedInstance::from),
            );
            next_token = output.next_token;
            if next_token.is_none() {
                break;
            }
        }

        Ok(instances)
    }
}
