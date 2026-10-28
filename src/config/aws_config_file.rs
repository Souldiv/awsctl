use anyhow::{Context, Result};
use ini::Ini;
use std::path::{Path, PathBuf};

use crate::config::constants::APP_NAME;
use crate::config::SsoDefaults;

/// Name of the `[sso-session]` block awsctl owns in `~/.aws/config`.
pub const SSO_SESSION_NAME: &str = APP_NAME;

const REGISTRATION_SCOPES: &str = "sso:account:access";

/// One `[profile <name>]` block pointing at an account and role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsoProfile {
    pub profile_name: String,
    pub account_id: String,
    pub role_name: String,
    pub region: String,
}

pub fn aws_config_path() -> Result<PathBuf> {
    let home_dir = directories::UserDirs::new()
        .context("Unable to get home directory")?
        .home_dir()
        .join(".aws");

    Ok(home_dir.join("config"))
}

/// Writes the SSO session and one profile per account into an AWS config file.
///
/// Sections and keys awsctl does not own are left as they are; the blocks it
/// does own are replaced, so repeated runs do not stack up stale keys.
pub fn write_sso_profiles(
    config_path: &Path,
    session_name: &str,
    sso_defaults: &SsoDefaults,
    profiles: &[SsoProfile],
) -> Result<()> {
    let mut config = load_or_create(config_path)?;

    let session_section = format!("sso-session {}", session_name);
    config.delete(Some(session_section.as_str()));
    config
        .with_section(Some(session_section.as_str()))
        .set("sso_start_url", sso_defaults.get_start_url())
        .set("sso_region", sso_defaults.get_sso_region())
        .set("sso_registration_scopes", REGISTRATION_SCOPES);

    for profile in profiles {
        let profile_section = format!("profile {}", profile.profile_name);
        config.delete(Some(profile_section.as_str()));
        config
            .with_section(Some(profile_section.as_str()))
            .set("sso_session", session_name)
            .set("sso_account_id", profile.account_id.as_str())
            .set("sso_role_name", profile.role_name.as_str())
            .set("region", profile.region.as_str());
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| "Unable to create the .aws directory.")?;
    }

    config
        .write_to_file(config_path)
        .with_context(|| "Unable to write the AWS config file.")?;

    Ok(())
}

fn load_or_create(config_path: &Path) -> Result<Ini> {
    if !config_path.try_exists()? {
        return Ok(Ini::new());
    }

    Ini::load_from_file(config_path).with_context(|| {
        format!(
            "Unable to read {}. Please fix or move it and try again.",
            config_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{write_sso_profiles, SsoProfile};
    use crate::config::SsoDefaults;
    use ini::Ini;
    use std::path::{Path, PathBuf};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "awsctl-{}-{}-{:?}",
                label,
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            TempDir { path }
        }

        fn file(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn defaults() -> SsoDefaults {
        SsoDefaults::new(
            String::from("https://labflow.awsapps.com/start"),
            String::from("us-east-1"),
        )
    }

    fn profile(profile_name: &str, account_id: &str) -> SsoProfile {
        SsoProfile {
            profile_name: String::from(profile_name),
            account_id: String::from(account_id),
            role_name: String::from("AdministratorAccess"),
            region: String::from("eu-west-2"),
        }
    }

    fn value(config_path: &Path, section: &str, key: &str) -> Option<String> {
        Ini::load_from_file(config_path)
            .expect("config parses")
            .section(Some(section))
            .and_then(|properties| properties.get(key))
            .map(String::from)
    }

    #[test]
    fn writes_the_session_and_one_profile_per_account() {
        let dir = TempDir::new("writes");
        let config_path = dir.file("config");

        write_sso_profiles(
            &config_path,
            "awsctl",
            &defaults(),
            &[
                profile("dev", "111111111111"),
                profile("au", "222222222222"),
            ],
        )
        .expect("writes");

        assert_eq!(
            value(&config_path, "sso-session awsctl", "sso_start_url").as_deref(),
            Some("https://labflow.awsapps.com/start")
        );
        assert_eq!(
            value(&config_path, "sso-session awsctl", "sso_region").as_deref(),
            Some("us-east-1")
        );
        assert_eq!(
            value(
                &config_path,
                "sso-session awsctl",
                "sso_registration_scopes"
            )
            .as_deref(),
            Some("sso:account:access")
        );

        assert_eq!(
            value(&config_path, "profile dev", "sso_account_id").as_deref(),
            Some("111111111111")
        );
        assert_eq!(
            value(&config_path, "profile dev", "sso_session").as_deref(),
            Some("awsctl")
        );
        assert_eq!(
            value(&config_path, "profile dev", "sso_role_name").as_deref(),
            Some("AdministratorAccess")
        );
        assert_eq!(
            value(&config_path, "profile dev", "region").as_deref(),
            Some("eu-west-2")
        );
        assert_eq!(
            value(&config_path, "profile au", "sso_account_id").as_deref(),
            Some("222222222222")
        );
    }

    #[test]
    fn creates_the_file_when_it_is_missing() {
        let dir = TempDir::new("creates");
        let config_path = dir.file("nested/config");

        write_sso_profiles(
            &config_path,
            "awsctl",
            &defaults(),
            &[profile("dev", "111111111111")],
        )
        .expect("writes");

        assert!(config_path.exists());
    }

    #[test]
    fn preserves_sections_and_keys_it_does_not_own() {
        let dir = TempDir::new("preserves");
        let config_path = dir.file("config");
        std::fs::write(
            &config_path,
            "[default]\nregion = ap-southeast-2\noutput = json\n\n\
             [profile hand-written]\nrole_arn = arn:aws:iam::999999999999:role/ops\nsource_profile = default\n",
        )
        .expect("seed config");

        write_sso_profiles(
            &config_path,
            "awsctl",
            &defaults(),
            &[profile("dev", "111111111111")],
        )
        .expect("writes");

        assert_eq!(
            value(&config_path, "default", "region").as_deref(),
            Some("ap-southeast-2")
        );
        assert_eq!(
            value(&config_path, "default", "output").as_deref(),
            Some("json")
        );
        assert_eq!(
            value(&config_path, "profile hand-written", "role_arn").as_deref(),
            Some("arn:aws:iam::999999999999:role/ops")
        );
        assert_eq!(
            value(&config_path, "profile hand-written", "source_profile").as_deref(),
            Some("default")
        );
        assert_eq!(
            value(&config_path, "profile dev", "sso_account_id").as_deref(),
            Some("111111111111")
        );
    }

    #[test]
    fn replaces_its_own_blocks_instead_of_stacking_them() {
        let dir = TempDir::new("replaces");
        let config_path = dir.file("config");

        write_sso_profiles(
            &config_path,
            "awsctl",
            &defaults(),
            &[profile("dev", "111111111111")],
        )
        .expect("first write");

        let moved = SsoProfile {
            region: String::from("us-west-2"),
            ..profile("dev", "333333333333")
        };
        write_sso_profiles(&config_path, "awsctl", &defaults(), &[moved]).expect("second write");

        let config = Ini::load_from_file(&config_path).expect("config parses");
        assert_eq!(config.section_all(Some("profile dev")).count(), 1);
        assert_eq!(
            value(&config_path, "profile dev", "sso_account_id").as_deref(),
            Some("333333333333")
        );
        assert_eq!(
            value(&config_path, "profile dev", "region").as_deref(),
            Some("us-west-2")
        );
    }

    #[test]
    fn leaves_a_profile_it_wrote_earlier_but_was_not_asked_for_again() {
        let dir = TempDir::new("earlier");
        let config_path = dir.file("config");

        write_sso_profiles(
            &config_path,
            "awsctl",
            &defaults(),
            &[profile("au", "222222222222")],
        )
        .expect("first write");
        write_sso_profiles(
            &config_path,
            "awsctl",
            &defaults(),
            &[profile("dev", "111111111111")],
        )
        .expect("second write");

        assert_eq!(
            value(&config_path, "profile au", "sso_account_id").as_deref(),
            Some("222222222222")
        );
        assert_eq!(
            value(&config_path, "profile dev", "sso_account_id").as_deref(),
            Some("111111111111")
        );
    }
}
