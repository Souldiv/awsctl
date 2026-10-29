use anyhow::{anyhow, Context, Result};
use tokio::time::{sleep, Duration};

use crate::aws::sso::SsoAccessTokenProvider;
use crate::aws::AccountInfoProvider;
use crate::cli_tools::helper;
use crate::cli_tools::profile_plan::{self, ProfilePlan};
use crate::cli_tools::written_profiles;
use crate::config::aws_config_file;
use crate::config::constants::APP_NAME;
use crate::config::profile_name::resolve_profile_names;
use crate::config::{
    AccessToken, AccountConfig, Credentials, Profile, SsoProfile, SSO_SESSION_NAME,
};

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

    let sso_defaults = account_config
        .get_sso_defaults()
        .cloned()
        .ok_or_else(|| anyhow!("Please set your sso defaults and try again."))?;

    let mut list_of_acc = account_info_provider
        .get_account_list(&access_token)
        .await?;

    list_of_acc.sort_by(|a, b| a.account_name.cmp(&b.account_name));

    let discovered = resolve_profile_names(
        list_of_acc.iter().map(|account_info| {
            (
                account_info.account_name.as_str(),
                account_info.account_id.as_str(),
            )
        }),
        account_config.get_profile_names(),
    );
    account_config.seed_profile_names(&discovered);

    let profiles = if let Some(account_filter) = account {
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

        vec![Profile::new(
            account_info.account_name,
            account_info.account_id,
            role_name,
            region,
        )]
    } else {
        helper::prompt_for_profiles(
            account_config.get_list_of_regions(),
            account_config.get_list_of_roles(),
            list_of_acc,
        )?
    };

    let plan = profile_plan::plan_profiles(&profiles, account_config.get_profile_names())?;
    report_name_clashes(&plan);

    let _ = account_config.set_profile(plan.default_profile.clone())?;

    helper::update_aws_config_file(&sso_defaults, &plan.sso_profiles)?;
    helper::update_sso_token_cache(access_token, &sso_defaults)?;

    let creds: Credentials = account_info_provider
        .get_role_credentials(
            &access_token,
            plan.default_profile.get_account_id(),
            plan.default_profile.get_role_name(),
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

fn report_name_clashes(plan: &ProfilePlan) {
    for clash in &plan.name_clashes {
        println!(
            "Warning: profile name '{}' was already taken, so '{}' was written as '{}'. \
             Give each account its own name under [profile_names] in your awsctl config.",
            clash.wanted_name, clash.account_name, clash.profile_name
        );
    }
}

pub async fn switch_default(
    access_token: &AccessToken,
    account_info_provider: &AccountInfoProvider,
    account_config: &mut AccountConfig,
    profile_name: Option<String>,
) -> Result<()> {
    if !access_token.is_set() || access_token.is_expired() {
        return Err(anyhow!(
            "The access token is missing or expired. Run 'awsctl sso --get-access-token' and try again."
        ));
    }

    let profiles = read_written_profiles()?;
    let chosen = match profile_name {
        Some(profile_name) => written_profiles::find_profile(&profiles, &profile_name)?.clone(),
        None => helper::prompt_for_written_profile(profiles)?,
    };
    let profile = written_profiles::to_current_profile(&chosen, account_config.get_profile_names());

    let creds: Credentials = account_info_provider
        .get_role_credentials(access_token, &chosen.account_id, &chosen.role_name)
        .await?
        .into();
    let session_token = creds
        .session_token()
        .ok_or_else(|| anyhow!("AWS returned role credentials without a session token."))?;

    helper::update_credentials_file(
        creds.access_key_id(),
        creds.secret_access_key(),
        session_token,
    )?;
    account_config.set_profile(profile.clone())?;
    confy::store(APP_NAME, "role-credentials", creds)
        .with_context(|| "Unable to store credentials")?;

    println!(
        "[default] now points at account {} ({}) with role {}.",
        profile.get_account_name(),
        profile.get_account_id(),
        profile.get_role_name()
    );

    Ok(())
}

pub fn list_profiles(account_config: &AccountConfig) -> Result<()> {
    let profiles = read_written_profiles()?;

    for line in written_profiles::format_listing(&profiles, account_config.get_current_profile()) {
        println!("{}", line);
    }

    Ok(())
}

fn read_written_profiles() -> Result<Vec<SsoProfile>> {
    let config_path = aws_config_file::aws_config_path()?;
    let profiles = aws_config_file::read_sso_profiles(&config_path, SSO_SESSION_NAME)?;

    if profiles.is_empty() {
        return Err(anyhow!(
            "No profiles written by awsctl were found in {}. Run 'awsctl sso --setup-profile' first.",
            config_path.display()
        ));
    }

    Ok(profiles)
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
