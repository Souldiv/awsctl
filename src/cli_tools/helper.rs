use anyhow::{anyhow, Context, Result};
use ini::Ini;
use inquire::{Select, Text};
use std::fs::File;
use std::path::PathBuf;

use crate::aws::AccountInfo;
use crate::config::constants::APP_NAME;
use crate::config::{AccountConfig, Profile, SsoDefaults};

pub fn prompt_for_profile(
    list_of_regions: Vec<String>,
    list_of_roles: Vec<String>,
    list_of_account_info: Vec<AccountInfo>,
) -> Result<Profile> {
    let account_info = Select::new("Select Profile Account: ", list_of_account_info)
        .prompt()
        .with_context(|| "There was an error displaying account info. Please try again.")?;
    let region = Select::new("Select Profile Region: ", list_of_regions)
        .prompt()
        .with_context(|| "There was an error displaying regions. Please try again.")?;
    let role_name = Select::new("Select Profile Role: ", list_of_roles)
        .prompt()
        .with_context(|| "There was an error displaying roles. Please try again.")?;
    Ok(Profile::new(
        account_info.account_name,
        account_info.account_id,
        role_name,
        region,
    ))
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
