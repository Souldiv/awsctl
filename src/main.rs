mod aws;
mod cli_tools;
mod config;
mod utils;

use crate::cli_tools::cli::{Cli, Commands, ConfigCommands};
use crate::cli_tools::cli_manager::{CliManager, SsoSelections};
use crate::config::constants::APP_NAME;
use crate::config::{AccessToken, AccountConfig};
use anyhow::{Context, Result};
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("Failed to install the default rustls crypto provider"))?;

    migrate_legacy_config()?;

    let cfg: AccountConfig = confy::load(APP_NAME, None)
        .with_context(|| "There was an error creating or reading the config file")?;
    let access_token: AccessToken = confy::load(APP_NAME, "access-token")
        .with_context(|| "There was an error reading the access token.")?;

    let cli_manager = CliManager::new(cfg, access_token);
    let cli = Cli::parse();

    match &cli.command {
        Commands::Sso {
            get_access_token,
            setup_profile,
            force,
            keep_alive,
            account,
            role,
            region,
        } => {
            cli_manager
                .sso_command(
                    *get_access_token,
                    *setup_profile,
                    *force,
                    *keep_alive,
                    SsoSelections {
                        account: account.clone(),
                        role: role.clone(),
                        region: region.clone(),
                    },
                )
                .await?;
        }
        Commands::Ssh { host, port } => {
            cli_manager.ssh_command(*host, *port).await?;
        }
        Commands::Info {
            accounts,
            roles,
            test,
        } => {
            cli_manager.info_command(*accounts, *roles, *test).await?;
        }
        Commands::Eks {
            update,
            list,
            region,
            cluster,
        } => {
            cli_manager
                .eks_command(*update, *list, region.clone(), cluster.clone())
                .await?;
        }
        Commands::Config {
            command,
            location,
            show,
            update,
        } => match command {
            Some(ConfigCommands::Edit) => {
                cli_manager.config_edit_command()?;
            }
            None => {
                cli_manager.config_command(*location, *show, update.clone())?;
            }
        },
        Commands::Ssm {
            describe_document,
            document_name,
            document_version,
            version_name,
            list_inventory,
            create_document,
            document_path,
        } => {
            cli_manager
                .ssm_command(
                    *describe_document,
                    document_name.clone(),
                    document_version.clone(),
                    version_name.clone(),
                    *list_inventory,
                    *create_document,
                    document_path.clone(),
                )
                .await?;
        }
        Commands::Snapshot {
            snapshot_type,
            namespace,
            before_date,
            delete_all,
            yes,
            count,
        } => {
            cli_manager
                .snapshot_command(
                    *snapshot_type,
                    namespace.clone(),
                    *before_date,
                    *delete_all,
                    *yes,
                    *count,
                )
                .await?;
        }
    }

    Ok(())
}

/// Copies any config files written by confy <2.0 into confy 2.0's location.
///
/// confy 0.6 located config via the `directories` crate (on macOS that is
/// `~/Library/Application Support/rs.awsctl`); confy 2.0 defaults to the XDG
/// directory (`~/.config/awsctl`). For each config file, if the new location
/// is missing but the legacy one exists, it is copied over so upgrading users
/// keep their SSO session and settings. The legacy file is left untouched.
fn migrate_legacy_config() -> Result<()> {
    let Some(legacy) = directories::ProjectDirs::from("rs", "", APP_NAME) else {
        return Ok(());
    };
    let legacy_dir = legacy.config_dir();

    for config_name in [None, Some("access-token"), Some("role-credentials")] {
        let new_path = confy::get_configuration_file_path(APP_NAME, config_name)
            .with_context(|| "Unable to determine the config file path")?;
        if new_path.exists() {
            continue;
        }

        let Some(file_name) = new_path.file_name() else {
            continue;
        };
        let legacy_path = legacy_dir.join(file_name);
        if legacy_path == new_path || !legacy_path.exists() {
            continue;
        }

        if let Some(parent) = new_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| "Unable to create the config directory")?;
        }
        std::fs::copy(&legacy_path, &new_path)
            .with_context(|| "Unable to migrate the legacy config file")?;
    }

    Ok(())
}
