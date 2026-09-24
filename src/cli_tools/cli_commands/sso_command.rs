use anyhow::{anyhow, Context, Result};
use tokio::time::{sleep, Duration};

use crate::aws::sso::SsoAccessTokenProvider;
use crate::aws::AccountInfoProvider;
use crate::cli_tools::helper;
use crate::config::constants::APP_NAME;
use crate::config::{AccessToken, AccountConfig, Credentials, Profile};

// used for requesting access token from aws
pub async fn get_access_token(
    access_token: AccessToken,
    start_url: String,
    sso_region: String,
    force: bool,
) -> Result<()> {
    let token = (access_token.is_set() && !force).then(|| access_token);
    let access_token = SsoAccessTokenProvider::new(start_url.clone(), sso_region, &token)?
        .get_access_token()
        .await?;

    confy::store(APP_NAME, "access-token", access_token)
        .with_context(|| "Unable to store access token.")?;
    println!("Access token was generated.");

    Ok(())
}

// used for setting up aws profile and generates role credentials
pub async fn setup_profile(
    access_token: &AccessToken,
    account_info_provider: &AccountInfoProvider,
    account_config: &mut AccountConfig,
    account: Option<String>,
    role: Option<String>,
) -> Result<()> {
    if !access_token.is_set() || access_token.is_expired() {
        return Err(anyhow!("Please authenticate client and try again."));
    }

    let mut list_of_acc = account_info_provider
        .get_account_list(&access_token)
        .await?;

    list_of_acc.sort_by(|a, b| a.account_name.cmp(&b.account_name));

    let profile = if let Some(account_filter) = account {
        let account_info = list_of_acc
            .into_iter()
            .find(|a| {
                a.account_id == account_filter
                    || a.account_name.to_lowercase() == account_filter.to_lowercase()
            })
            .ok_or_else(|| {
                anyhow!(
                    "Account '{}' not found. Use 'awsctl info -a' to list available accounts.",
                    account_filter
                )
            })?;

        let role_name = role.unwrap_or_else(|| {
            account_config
                .get_list_of_roles()
                .first()
                .cloned()
                .unwrap_or_else(|| String::from("AdministratorAccess"))
        });

        let region = account_config
            .get_list_of_regions()
            .first()
            .cloned()
            .unwrap_or_else(|| String::from("us-east-1"));

        println!(
            "Switching to account: {} ({}) with role: {} in region: {}",
            account_info.account_name, account_info.account_id, role_name, region
        );

        Profile::new(
            account_info.account_name,
            account_info.account_id,
            role_name,
            region,
        )
    } else {
        helper::prompt_for_profile(
            account_config.get_list_of_regions(),
            account_config.get_list_of_roles(),
            list_of_acc,
        )?
    };
    let _ = account_config.set_profile(profile)?;

    let creds: Credentials = account_info_provider
        .get_role_credentials(
            &access_token,
            account_config
                .get_current_profile()
                .unwrap()
                .get_account_id(),
            account_config
                .get_current_profile()
                .unwrap()
                .get_role_name(),
        )
        .await?
        .into();

    let _ = helper::update_credentials_file(
        creds.access_key_id(),
        creds.secret_access_key(),
        creds.session_token().unwrap(),
    )?;

    confy::store(APP_NAME, "role-credentials", creds)
        .with_context(|| "Unable to store credentials")?;

    println!("Profile was setup successfully");

    Ok(())
}

// starts a keep alive function that re-news the role credentials every hour
pub async fn start_keep_alive(
    access_token: &AccessToken,
    account_info_provider: &AccountInfoProvider,
    account_config: &AccountConfig,
) -> Result<()> {
    let creds: Credentials = account_info_provider
        .get_role_credentials(
            &access_token,
            account_config
                .get_current_profile()
                .unwrap()
                .get_account_id(),
            account_config
                .get_current_profile()
                .unwrap()
                .get_role_name(),
        )
        .await?
        .into();

    confy::store(APP_NAME, "role-credentials", creds)
        .with_context(|| "Unable to store credentials")?;
    println!("Role credentials were created successfully.");

    let time_to_sleep = Duration::from_secs(3600);
    println!("Sleeping for {:?} seconds", time_to_sleep.as_secs());
    let _ = sleep(time_to_sleep).await;

    Ok(())
}
