use anyhow::{anyhow, Context, Result};
use aws_config::retry::RetryConfig;
use aws_config::SdkConfig;
use aws_sdk_ssm::config::AppName;
use aws_sdk_ssm::config::SharedCredentialsProvider;
use aws_sdk_sso::config::Credentials as AWSCredentials;

use crate::config::constants::APP_NAME;
use crate::config::Credentials;

pub fn stored_role_sdk_config(region: &str) -> Result<SdkConfig> {
    let role_credentials: Credentials = confy::load(APP_NAME, "role-credentials")
        .with_context(|| "There was an error reading the credentials.")?;

    if !role_credentials.is_set() {
        return Err(anyhow!("Generate credentials and try again."));
    }

    let role_credentials: AWSCredentials = role_credentials.into();
    let config = aws_config::SdkConfig::builder()
        .app_name(AppName::new(APP_NAME)?)
        .credentials_provider(SharedCredentialsProvider::new(role_credentials))
        .behavior_version(aws_sdk_ssm::config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .retry_config(RetryConfig::standard())
        .build();

    Ok(config)
}

pub async fn profile_sdk_config(profile: &str, region: Option<&str>) -> SdkConfig {
    let loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .app_name(AppName::new(APP_NAME).expect("APP_NAME is a valid app name"))
        .profile_name(profile)
        .retry_config(RetryConfig::standard());

    match region {
        Some(region) => loader.region(aws_config::Region::new(region.to_string())),
        None => loader,
    }
    .load()
    .await
}
