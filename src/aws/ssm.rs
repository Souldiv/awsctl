use anyhow::Result;
use aws_config::SdkConfig;
use aws_sdk_ssm::types::{InstanceInformation, InventoryFilter, InventoryResultEntity};
use aws_sdk_ssm::Client;
use serde_json::json;
use std::collections::HashMap;
use std::process::Command;

pub struct SsmProvider {
    client: Client,
}

impl SsmProvider {
    pub fn new(sdk_config: &SdkConfig) -> Self {
        SsmProvider {
            client: Client::new(sdk_config),
        }
    }

    pub async fn create_ssm_document(
        &self,
        document_name: &str,
        document_content: &str,
    ) -> Result<()> {
        let output = self
            .client
            .create_document()
            .name(document_name)
            .content(document_content)
            .send()
            .await?;
        println!("{:#?}", output.document_description().unwrap());
        Ok(())
    }

    pub async fn list_ssm_managed_inventory(
        &self,
        filter: Option<InventoryFilter>,
    ) -> Result<Vec<InventoryResultEntity>> {
        let mut list_of_entities: Vec<InventoryResultEntity> = vec![];
        let client_request = self.client.get_inventory();

        let inventory_op = if filter.is_some() {
            client_request.filters(filter.unwrap()).send().await?
        } else {
            client_request.send().await?
        };

        list_of_entities.extend_from_slice(inventory_op.entities());
        if inventory_op.next_token.is_some() {
            let mut next_token = inventory_op.next_token;
            while let Some(token) = next_token {
                let output = self
                    .client
                    .get_inventory()
                    .next_token(&token)
                    .send()
                    .await?;
                list_of_entities.extend_from_slice(output.entities());
                next_token = output.next_token;
            }
        }

        Ok(list_of_entities)
    }

    pub async fn describe_ssm_document(
        &self,
        document_name: &str,
        document_version: Option<&str>,
        version_name: Option<&str>,
    ) -> Result<()> {
        let mut document_op = self.client.describe_document().name(document_name);

        if document_version.is_some() {
            document_op = document_op.document_version(document_version.unwrap());
        }

        if version_name.is_some() {
            document_op = document_op.version_name(version_name.unwrap());
        }

        let output = document_op.send().await?;

        println!("{:#?}", output.document().unwrap());

        Ok(())
    }

    pub async fn list_ssm_managed_instances(&self) -> Result<Vec<InstanceInformation>> {
        // let ec2_filter = InstanceInformationStringFilter::builder()
        //                     .key("ResourceType")
        //                     .values("EC2Instance")
        //                     .build()?;
        let mut list_of_instances: Vec<InstanceInformation> = vec![];
        let instance_op = self
            .client
            .describe_instance_information()
            // .filters(ec2_filter)
            .send()
            .await?;

        list_of_instances.extend_from_slice(instance_op.instance_information_list());

        if instance_op.next_token.is_some() {
            let mut next_token = instance_op.next_token;
            while let Some(token) = next_token {
                let output = self
                    .client
                    .describe_instance_information()
                    .next_token(&token)
                    .send()
                    .await?;
                list_of_instances.extend_from_slice(output.instance_information_list());
                next_token = output.next_token;
            }
        }

        Ok(list_of_instances)
    }

    pub async fn create_ssh_session(&self, instance_id: &str) -> Result<()> {
        let output = self
            .client
            .start_session()
            .target(instance_id)
            .send()
            .await?;

        // create ssm plugin json message
        let response = json!({
            "SessionId": output.session_id().unwrap().to_string(),
            "TokenValue": output.token_value().unwrap().to_string(),
            "StreamUrl": output.stream_url().unwrap().to_string()
        });

        let region = self.client.config().region().unwrap();

        // commenting code for reference
        // let template =r#"{"Target" : "{instance_id}","DocumentName": "AWS-StartPortForwardingSession","Parameters" : {"portNumber": [22],"localPortNumber": [3232] }}"#;
        // let ssm_plugin_document = json!(template.replace("{instance_id}", instance_id.into()));
        // let plugin_string = serde_json::to_string(&ssm_plugin_document)?;

        let response_string = serde_json::to_string(&response)?;
        let mut session_manager_plugin = Command::new("session-manager-plugin");
        let run_command_output = session_manager_plugin
            .args([
                response_string,
                region.as_ref().into(),
                "StartSession".into(),
                format!("https://ssm.{}.amazonaws.com/", region.as_ref()).into(),
            ])
            .spawn()?;
        let _ = run_command_output.wait_with_output()?;

        let _ = self
            .client
            .terminate_session()
            .session_id(output.session_id().unwrap())
            .send()
            .await?;
        println!("Session was successfully terminated");
        Ok(())
    }

    pub async fn create_port_forward_session(
        &self,
        instance_id: &str,
        rds_instance_endpoint: &str,
        local_port: i32,
        remote_port: i32,
    ) -> Result<()> {
        let params = HashMap::from([
            (
                String::from("host"),
                vec![rds_instance_endpoint.to_string()],
            ),
            (String::from("portNumber"), vec![remote_port.to_string()]),
            (
                String::from("localPortNumber"),
                vec![local_port.to_string()],
            ),
        ]);

        let output = self
            .client
            .start_session()
            .document_name("AWS-StartPortForwardingSessionToRemoteHost")
            .target(instance_id)
            .set_parameters(Some(params))
            .send()
            .await?;

        // create ssm plugin json message
        let response = json!({
            "SessionId": output.session_id().unwrap().to_string(),
            "TokenValue": output.token_value().unwrap().to_string(),
            "StreamUrl": output.stream_url().unwrap().to_string()
        });
        let region = self.client.config().region().unwrap();

        let response_string = serde_json::to_string(&response)?;
        let mut session_manager_plugin = Command::new("session-manager-plugin");
        let run_command_output = session_manager_plugin
            .args([
                response_string,
                region.as_ref().into(),
                "StartSession".into(),
                format!("https://ssm.{}.amazonaws.com/", region.as_ref()).into(),
            ])
            .spawn()?;

        let _ = run_command_output.wait_with_output()?;

        Ok(())
    }
}
