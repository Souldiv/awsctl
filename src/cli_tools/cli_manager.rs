use anyhow::{anyhow, Context, Result};
use aws_config::retry::RetryConfig;
use aws_config::SdkConfig;
use aws_sdk_ssm::config::AppName;
use aws_sdk_ssm::config::SharedCredentialsProvider;
use aws_sdk_sso::config::Credentials as AWSCredentials;
use aws_sdk_ssooidc as ssooidc;
use chrono::{DateTime, Utc};
use std::process::Command;
use std::time::SystemTime;

use crate::aws::AccountInfoProvider;
use crate::aws::EC2Provider;
use crate::aws::EksProvider;
use crate::aws::RdsProvider;
use crate::aws::SnapshotProvider;
use crate::aws::SsmProvider;
use crate::cli_tools::cli::Host;
use crate::cli_tools::cli::SnapshotType;
use crate::cli_tools::cli_commands::{eks_command, ssh_command, sso_command};
use crate::cli_tools::helper;
use crate::config::constants::APP_NAME;
use crate::config::{AccessToken, AccountConfig, Credentials};

pub struct SsoSelections {
    pub account: Option<String>,
    pub role: Option<String>,
    pub region: Option<String>,
}

pub struct SsoFlags {
    pub get_access_token: bool,
    pub setup_profile: bool,
    pub force: bool,
    pub keep_alive: bool,
    pub switch_default: Option<Option<String>>,
    pub list_profiles: bool,
}

pub struct CliManager {
    cfg: AccountConfig,
    access_token: AccessToken,
}

impl CliManager {
    pub fn new(cfg: AccountConfig, access_token: AccessToken) -> Self {
        Self { cfg, access_token }
    }

    // function for handling setup for most providers, region overrides the profile region
    fn create_config(&self, region: Option<&str>) -> Result<SdkConfig> {
        let role_credentials: Credentials = confy::load(APP_NAME, "role-credentials")
            .with_context(|| "There was an error reading the credentials.")?;

        if !role_credentials.is_set() {
            return Err(anyhow!("Generate credentials and try again."));
        }

        if self.cfg.get_current_profile().is_none() {
            return Err(anyhow!("Please set your profile and try again."));
        }

        let profile = self.cfg.get_current_profile().unwrap();
        let region = region.unwrap_or_else(|| profile.get_region());
        let role_credentials: AWSCredentials = role_credentials.into();

        let retry_config = RetryConfig::standard();
        let scp = SharedCredentialsProvider::new(role_credentials);
        let config = aws_config::SdkConfig::builder()
            .app_name(AppName::new(APP_NAME)?)
            .credentials_provider(scp)
            .behavior_version(aws_sdk_ssm::config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region.to_string()))
            .retry_config(retry_config)
            .build();

        Ok(config)
    }

    pub async fn sso_command(mut self, flags: SsoFlags, selections: SsoSelections) -> Result<()> {
        if flags.list_profiles {
            return sso_command::list_profiles(&self.cfg);
        }

        if !self.cfg.is_defaults_set() {
            self.cfg.set_defaults(helper::prompt_for_defaults(
                self.cfg.get_list_of_regions(),
                selections.region,
            )?)?;
        }
        let sso_defaults = self.cfg.get_sso_defaults().unwrap();

        let config = aws_config::SdkConfig::builder()
            .region(aws_config::Region::new(
                sso_defaults.get_sso_region().to_string(),
            ))
            .behavior_version(ssooidc::config::BehaviorVersion::latest())
            .build();

        let acc_info_provider = AccountInfoProvider::new(&config);

        match (
            flags.get_access_token,
            flags.setup_profile,
            flags.keep_alive,
            flags.switch_default,
        ) {
            (false, false, false, Some(profile_name)) => {
                sso_command::switch_default(
                    &self.access_token,
                    &acc_info_provider,
                    &mut self.cfg,
                    profile_name,
                )
                .await?;
            }
            // get access token
            (true, false, ..) => {
                sso_command::get_access_token(
                    self.access_token,
                    sso_defaults.get_start_url().to_string(),
                    sso_defaults.get_sso_region().to_string(),
                    flags.force,
                )
                .await?;
            }
            // setup profile
            (_, true, false, _) => {
                sso_command::setup_profile(
                    &self.access_token,
                    &acc_info_provider,
                    &mut self.cfg,
                    selections.account,
                    selections.role,
                )
                .await?;
            }
            // start keep alive
            (_, false, true, _) => loop {
                sso_command::start_keep_alive(&self.access_token, &acc_info_provider, &self.cfg)
                    .await?;
            },
            _ => {}
        }
        confy::store(APP_NAME, None, self.cfg)
            .with_context(|| "There was an error storing the config...")?;

        Ok(())
    }

    pub async fn ssh_command(&self, host: Host, local_port: i32) -> Result<()> {
        let config = self.create_config(None)?;

        let ssm = SsmProvider::new(&config);
        let ec2 = EC2Provider::new(&config);
        match host {
            Host::Ec2 => {
                ssh_command::ssh_into_ec2_instance(&ssm, &ec2).await?;
            }
            Host::Rds => {
                let rds = RdsProvider::new(&config);
                ssh_command::ssh_into_rds_instance(&ssm, &ec2, &rds, local_port).await?;
            }
        }
        Ok(())
    }

    // debug command
    pub async fn info_command(&self, accounts: bool, roles: bool, test: bool) -> Result<()> {
        let config = self.create_config(None)?;

        let ssm = SsmProvider::new(&config);

        match (accounts, roles, test) {
            (true, ..) => {
                let inventory = ssm.list_ssm_managed_inventory(None).await?;
                println!("{:#?}", inventory);
            }
            (false, true, ..) => {}
            (false, .., true) => {
                let current_time = SystemTime::now();
                let datetime_utc: DateTime<Utc> = current_time.into();
                println!("datetime: {:#?}", datetime_utc);
            }
            _ => {}
        }

        Ok(())
    }

    pub async fn eks_command(
        &self,
        update: bool,
        list: bool,
        region: Option<String>,
        cluster: Option<String>,
    ) -> Result<()> {
        let config = self.create_config(region.as_deref())?;
        let eks = EksProvider::new(&config);

        match (update, list) {
            (true, ..) => {
                println!("Updating kubeconfig...");
                let region = region.unwrap_or_else(|| {
                    self.cfg
                        .get_current_profile()
                        .unwrap()
                        .get_region()
                        .to_string()
                });
                eks_command::update_kubeconfig(&eks, &region, cluster).await?;
            }
            (.., true) => {
                println!("Listing clusters...");
                let clusters = eks.list_clusters().await?;
                println!("{:#?}", clusters);
            }
            _ => {}
        }

        Ok(())
    }

    pub fn config_command(
        &self,
        location: bool,
        show: bool,
        file_path: Option<String>,
    ) -> Result<()> {
        match (location, show) {
            (true, ..) => {
                let config_path = confy::get_configuration_file_path(APP_NAME, None)
                    .with_context(|| "Unable to locate config...")?;
                println!("{}", config_path.to_str().unwrap());
            }
            (.., true) => {
                let config_toml = toml::to_string(&self.cfg).with_context(|| {
                    "Unable to parse config file. Please remove and generate config again."
                })?;
                println!("{}", config_toml);
            }
            _ => {
                let _ = helper::create_cfg_from_file(file_path.as_ref().unwrap())?;
                let _ = helper::remove_access_key_file()?;
            }
        }

        Ok(())
    }

    pub fn config_edit_command(&self) -> Result<()> {
        let config_path = confy::get_configuration_file_path(APP_NAME, None)
            .with_context(|| "Unable to locate config...")?;

        let editor = std::env::var("EDITOR").unwrap_or_else(|_| String::from("vim"));
        let status = Command::new(&editor)
            .arg(&config_path)
            .status()
            .with_context(|| format!("There was an error launching '{}'.", editor))?;

        if !status.success() {
            return Err(anyhow!("'{}' exited with an error.", editor));
        }

        Ok(())
    }

    pub async fn ssm_command(
        &self,
        describe_document: bool,
        document_name: Option<String>,
        document_version: Option<String>,
        version_name: Option<String>,
        list_inventory: bool,
        create_document: bool,
        document_path: String,
    ) -> Result<()> {
        let config = self.create_config(None)?;
        let ssm_provider = SsmProvider::new(&config);

        match (describe_document, list_inventory, create_document) {
            (true, .., false) => {
                if document_name.is_none() {
                    anyhow::bail!("Please provide a document name.");
                }
                let _ = ssm_provider
                    .describe_ssm_document(
                        &document_name.unwrap(),
                        document_version.as_deref(),
                        version_name.as_deref(),
                    )
                    .await?;
            }
            (.., true, false) => {
                let inventory = ssm_provider.list_ssm_managed_inventory(None).await?;
                println!("{:#?}", inventory);
            }
            (.., false, true) => {
                let document_content: String = std::fs::read_to_string(&document_path)?;
                let document_name = match document_name {
                    Some(name) => name,
                    None => document_path.split("/").last().unwrap().to_string(),
                };
                let _ = ssm_provider
                    .create_ssm_document(&document_name, &document_content)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn snapshot_command(
        &self,
        snapshot_type: SnapshotType,
        namespace: String,
        before_date: DateTime<Utc>,
        delete_all: bool,
        yes: bool,
        count: bool,
    ) -> Result<()> {
        let config = self.create_config(None)?;
        let snapshot_provider = SnapshotProvider::new(&config);

        let snapshots = match snapshot_type {
            SnapshotType::Aws => {
                println!("Processing AWS snapshots...");
                snapshot_provider.list_snapshots(namespace).await?
            }
            SnapshotType::Eks => {
                println!("Processing EKS snapshots...");
                snapshot_provider
                    .list_volume_snapshots(namespace.clone())
                    .await?
            }
            SnapshotType::Cnpg => {
                println!("Processing Cnpg backups...");
                snapshot_provider.list_cnpg_backups(namespace).await?
            }
        };

        let mut filtered_snapshots = snapshots
            .into_iter()
            .filter(|snapshot| snapshot.created_at <= before_date)
            .collect::<Vec<_>>();

        // order by start time
        filtered_snapshots.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        if count {
            println!("{}", filtered_snapshots.len());
        } else {
            println!("{:#?}", filtered_snapshots);
        }
        if delete_all {
            match snapshot_type {
                SnapshotType::Aws => {
                    snapshot_provider
                        .delete_aws_snapshots(filtered_snapshots, yes)
                        .await?;
                }
                SnapshotType::Eks => {
                    snapshot_provider
                        .delete_eks_snapshots(filtered_snapshots, yes)
                        .await?;
                }
                SnapshotType::Cnpg => {
                    snapshot_provider
                        .delete_cnpg_backups(filtered_snapshots, yes)
                        .await?;
                }
            }
        }

        Ok(())
    }
}
