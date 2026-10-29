use anyhow::{anyhow, Context, Result};
use ini::Ini;
use inquire::validator::MinLengthValidator;
use inquire::{MultiSelect, Select, Text};
use std::fs::File;
use std::path::PathBuf;

use crate::aws::AccountInfo;
use crate::config::aws_config_file;
use crate::config::constants::APP_NAME;
use crate::config::sso_token_cache;
use crate::config::{
    AccessToken, AccountConfig, Profile, SsoDefaults, SsoProfile, SSO_SESSION_NAME,
};

/// Prompts for one or more accounts, then for the region and role they share.
///
/// The returned profiles lead with the account that takes the `[default]`
/// credentials.
pub fn prompt_for_profiles(
    list_of_regions: Vec<String>,
    list_of_roles: Vec<String>,
    list_of_account_info: Vec<AccountInfo>,
) -> Result<Vec<Profile>> {
    let accounts = prompt_for_accounts(list_of_account_info)?;
    let region = Select::new("Select Profile Region: ", list_of_regions)
        .prompt()
        .with_context(|| "There was an error displaying regions. Please try again.")?;
    let role_name = Select::new("Select Profile Role: ", list_of_roles)
        .prompt()
        .with_context(|| "There was an error displaying roles. Please try again.")?;

    Ok(accounts
        .into_iter()
        .map(|account_info| {
            Profile::new(
                account_info.account_name,
                account_info.account_id,
                role_name.clone(),
                region.clone(),
            )
        })
        .collect())
}

fn prompt_for_accounts(list_of_account_info: Vec<AccountInfo>) -> Result<Vec<AccountInfo>> {
    let accounts = MultiSelect::new("Select Profile Accounts: ", list_of_account_info)
        .with_validator(MinLengthValidator::new(1).with_message("Select at least one account."))
        .prompt()
        .with_context(|| "There was an error displaying account info. Please try again.")?;

    default_account_first(accounts)
}

// inquire reports ticked options in list order rather than the order they were
// ticked, so the account that takes [default] is asked for instead of guessed.
fn default_account_first(accounts: Vec<AccountInfo>) -> Result<Vec<AccountInfo>> {
    if accounts.len() < 2 {
        return Ok(accounts);
    }

    let account_names: Vec<String> = accounts.iter().map(AccountInfo::to_string).collect();
    let chosen = Select::new("Select the account to keep as [default]: ", account_names)
        .raw_prompt()
        .with_context(|| {
            "There was an error displaying the selected accounts. Please try again."
        })?;

    let mut ordered = accounts;
    let default_account = ordered.remove(chosen.index);
    ordered.insert(0, default_account);

    Ok(ordered)
}

pub fn prompt_for_defaults(
    list_of_regions: Vec<String>,
    region: Option<String>,
) -> Result<SsoDefaults> {
    let region = region
        .map(|region| validate_region(region, &list_of_regions))
        .transpose()?;
    let start_url = Text::new("Enter SSO Start Url: ")
        .prompt()
        .with_context(|| "There was an error reading sso start url.")?;
    let region = match region {
        Some(region) => region,
        None => Select::new("Select SSO Region: ", list_of_regions)
            .prompt()
            .with_context(|| "There was an error displaying regions. Please try again.")?,
    };
    Ok(SsoDefaults::new(start_url, region))
}

fn validate_region(region: String, list_of_regions: &[String]) -> Result<String> {
    if list_of_regions.iter().any(|known| known == &region) {
        return Ok(region);
    }
    Err(anyhow!(
        "Unknown region '{}'. Valid regions are: {}",
        region,
        list_of_regions.join(", ")
    ))
}

pub fn create_cfg_from_file(file_path: &str) -> Result<()> {
    let file_path = file_path.to_string();
    let config_string = std::fs::read_to_string(PathBuf::from(file_path))
        .with_context(|| "Please check config file path and try again.")?;
    let custom_cfg: AccountConfig = toml::from_str(&config_string)
        .with_context(|| "Unable to read contents of config file.")?;

    confy::store(APP_NAME, None, custom_cfg)
        .with_context(|| "There was an error storing the config...")?;

    let config_file_path = confy::get_configuration_file_path(APP_NAME, None)
        .with_context(|| "Unable to locate config...")?;

    println!(
        "Successfully created config at: {}",
        config_file_path.to_str().unwrap()
    );

    Ok(())
}

pub fn remove_access_key_file() -> Result<()> {
    let file_path = confy::get_configuration_file_path(APP_NAME, "access-token")
        .with_context(|| "There was an error locating access token...")?;
    std::fs::remove_file(&file_path)
        .with_context(|| "There was an error removing access token...")?;
    Ok(())
}

// helper function to update ~/.aws/credentials
pub fn update_credentials_file(
    access_key_id: &str,
    secret_access_key: &str,
    session_token: &str,
) -> Result<()> {
    let home_dir = directories::UserDirs::new()
        .context("Unable to get home directory")?
        .home_dir()
        .join(".aws");

    let aws_credentials_file_path = home_dir.join("credentials").to_path_buf();
    if !aws_credentials_file_path.try_exists()? {
        File::create(&aws_credentials_file_path)?;
    }

    let mut credentials = Ini::load_from_file(&aws_credentials_file_path)?;

    credentials.set_to(
        Some(String::from("default")),
        String::from("aws_access_key_id"),
        String::from(access_key_id),
    );
    credentials.set_to(
        Some(String::from("default")),
        String::from("aws_secret_access_key"),
        String::from(secret_access_key),
    );
    credentials.set_to(
        Some(String::from("default")),
        String::from("aws_session_token"),
        String::from(session_token),
    );

    credentials.write_to_file(aws_credentials_file_path)?;

    Ok(())
}

pub fn update_sso_token_cache(
    access_token: &AccessToken,
    sso_defaults: &SsoDefaults,
) -> Result<()> {
    let cache_path = sso_token_cache::cache_path(SSO_SESSION_NAME)?;
    sso_token_cache::write_sso_token(&cache_path, access_token, sso_defaults)?;

    println!(
        "Wrote the SSO token for session '{}' to {}",
        SSO_SESSION_NAME,
        cache_path.display()
    );

    if let Some(warning) = sso_token_cache::renewal_warning(access_token) {
        println!("{}", warning);
    }

    Ok(())
}

pub fn update_aws_config_file(sso_defaults: &SsoDefaults, profiles: &[SsoProfile]) -> Result<()> {
    let config_path = aws_config_file::aws_config_path()?;
    aws_config_file::write_sso_profiles(&config_path, SSO_SESSION_NAME, sso_defaults, profiles)?;

    println!(
        "Wrote {} profile(s) to {}",
        profiles.len(),
        config_path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_region;

    fn regions() -> Vec<String> {
        vec![String::from("us-east-1"), String::from("eu-west-1")]
    }

    #[test]
    fn accepts_a_known_region() {
        let region = validate_region(String::from("eu-west-1"), &regions()).unwrap();
        assert_eq!(region, "eu-west-1");
    }

    #[test]
    fn rejects_an_unknown_region_and_names_the_valid_ones() {
        let error = validate_region(String::from("mars-north-1"), &regions()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("mars-north-1"));
        assert!(message.contains("us-east-1, eu-west-1"));
    }
}
