use anyhow::{anyhow, Context, Result};
use aws_config::SdkConfig;
use aws_sdk_ssm::types::{
    InstanceInformation, InstanceInformationStringFilter, InventoryFilter, InventoryResultEntity,
    PingStatus,
};
use aws_sdk_ssm::Client;
use serde_json::json;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::process::Command;
use std::time::Duration;

const SESSION_MANAGER_PLUGIN: &str = "session-manager-plugin";
const PLUGIN_INSTALL_URL: &str = "https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html";
const PLUGIN_EXIT_GRACE: Duration = Duration::from_secs(5);

pub fn ensure_session_manager_plugin() -> Result<()> {
    match Command::new(SESSION_MANAGER_PLUGIN).arg("--version").output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(anyhow!(
            "'{} --version' failed: {}",
            SESSION_MANAGER_PLUGIN,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(anyhow!(
            "The Session Manager plugin ({}) is not on your PATH. Install it from {} and try again.",
            SESSION_MANAGER_PLUGIN,
            PLUGIN_INSTALL_URL
        )),
        Err(error) => Err(error)
            .with_context(|| format!("Unable to run {}.", SESSION_MANAGER_PLUGIN)),
    }
}

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

    pub async fn ensure_instance_online(&self, instance_id: &str) -> Result<()> {
        let id_filter = InstanceInformationStringFilter::builder()
            .key("InstanceIds")
            .values(instance_id)
            .build()?;
        let output = self
            .client
            .describe_instance_information()
            .filters(id_filter)
            .send()
            .await
            .with_context(|| format!("Unable to look up {} in Systems Manager.", instance_id))?;

        match output.instance_information_list().first() {
            None => Err(anyhow!(
                "Instance {} is not managed by Systems Manager, so no session can be opened through it.",
                instance_id
            )),
            Some(info) if info.ping_status() != Some(&PingStatus::Online) => Err(anyhow!(
                "The SSM agent on {} is not online (status: {}).",
                instance_id,
                info.ping_status().map(|status| status.as_str()).unwrap_or("unknown")
            )),
            Some(_) => Ok(()),
        }
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

        ensure_session_manager_plugin()?;

        let output = self
            .client
            .start_session()
            .document_name("AWS-StartPortForwardingSessionToRemoteHost")
            .target(instance_id)
            .set_parameters(Some(params))
            .send()
            .await?;
        let session_id = output
            .session_id()
            .ok_or_else(|| anyhow!("Systems Manager returned no session id."))?
            .to_string();

        // create ssm plugin json message
        let response = json!({
            "SessionId": session_id,
            "TokenValue": output.token_value().unwrap().to_string(),
            "StreamUrl": output.stream_url().unwrap().to_string()
        });
        let region = self.client.config().region().unwrap();

        let response_string = serde_json::to_string(&response)?;
        let forwarded = run_port_forward_plugin(response_string, region.as_ref().to_string()).await;

        if let Err(error) = self
            .client
            .terminate_session()
            .session_id(&session_id)
            .send()
            .await
        {
            eprintln!(
                "Warning: unable to terminate session {}: {}",
                session_id, error
            );
        }

        forwarded
    }
}

/// Ctrl-C reaches the plugin too, so it is given a moment to exit on its own
/// before it is killed.
async fn run_port_forward_plugin(response: String, region: String) -> Result<()> {
    let mut plugin = tokio::process::Command::new(SESSION_MANAGER_PLUGIN)
        .args([
            response,
            region.clone(),
            String::from("StartSession"),
            format!("https://ssm.{}.amazonaws.com/", region),
        ])
        .spawn()
        .with_context(|| format!("Unable to start {}.", SESSION_MANAGER_PLUGIN))?;

    let exit_status = tokio::select! {
        status = plugin.wait() => Some(status?),
        interrupted = tokio::signal::ctrl_c() => {
            interrupted.with_context(|| "Unable to listen for Ctrl-C.")?;
            None
        }
    };

    match exit_status {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(anyhow!(
            "{} exited with {}, so the port forward is not running.",
            SESSION_MANAGER_PLUGIN,
            status
        )),
        None => {
            if tokio::time::timeout(PLUGIN_EXIT_GRACE, plugin.wait())
                .await
                .is_err()
            {
                plugin.kill().await?;
            }
            Ok(())
        }
    }
}
