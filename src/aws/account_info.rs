use aws_config::SdkConfig;
use aws_sdk_sso::config::Credentials;
use aws_sdk_sso::Client;
// use aws_sdk_ssm::config::{SharedCredentialsProvider, Credentials };
// use aws_sdk_ssm::config::AppName;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

use std::time::{Duration, UNIX_EPOCH};

use crate::config::AccessToken;

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountInfo {
    pub account_name: String,
    pub account_id: String,
}

impl Display for AccountInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} - {}", self.account_name, self.account_id)
    }
}

pub struct AccountInfoProvider {
    client: Client,
}

impl AccountInfoProvider {
    pub fn new(sdk_config: &SdkConfig) -> Self {
        AccountInfoProvider {
            client: Client::new(sdk_config),
        }
    }

    pub async fn get_account_list(&self, access_token: &AccessToken) -> Result<Vec<AccountInfo>> {
        let mut paginator = self
            .client
            .list_accounts()
            .access_token(access_token.access_token.as_str())
            .max_results(100)
            .into_paginator()
            .send();

        let mut account_infos = Vec::new();
        while let Some(page) = paginator.next().await {
            for account in page?.account_list() {
                account_infos.push(AccountInfo {
                    account_id: String::from(account.account_id().unwrap()),
                    account_name: String::from(account.account_name().unwrap_or("unknown")),
                });
            }
        }

        Ok(account_infos)
    }

    pub async fn get_role_credentials(
        &self,
        access_token: &AccessToken,
        account_id: &str,
        role_name: &str,
    ) -> Result<Credentials> {
        let get_rc_output = self
            .client
            .get_role_credentials()
            .access_token(access_token.access_token.as_str())
            .role_name(role_name)
            .account_id(account_id)
            .send()
            .await?;

        let role_credentials = get_rc_output.role_credentials().unwrap();
        let expiration_secs = role_credentials.expiration();
        let expiration = if expiration_secs != 0 {
            Some(UNIX_EPOCH + Duration::from_secs(expiration_secs as u64))
        } else {
            None
        };

        let creds = Credentials::new(
            role_credentials.access_key_id().unwrap(),
            role_credentials.secret_access_key().unwrap(),
            Some(role_credentials.session_token().unwrap().to_string()),
            expiration,
            "awsctl",
        );

        Ok(creds)
    }
}
