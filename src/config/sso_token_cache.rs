use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};
use sha1::{Digest, Sha1};
use std::fmt;
use std::fs::{DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::config::{AccessToken, SsoDefaults};

const AWS_CLI_TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";
const EXPIRY_DISPLAY_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";
const CACHE_FILE_MODE: u32 = 0o600;
const CACHE_DIR_MODE: u32 = 0o700;
const AWS_CLI_RENEWAL_WINDOW_SECONDS: i64 = 15 * 60;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AwsCliCachedToken {
    access_token: String,
    #[serde(serialize_with = "serialize_timestamp")]
    expires_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_timestamp"
    )]
    registration_expires_at: Option<DateTime<Utc>>,
    region: String,
    start_url: String,
}

impl AwsCliCachedToken {
    fn new(access_token: &AccessToken, sso_defaults: &SsoDefaults) -> Self {
        let device_client = access_token.device_client.as_ref();

        AwsCliCachedToken {
            access_token: access_token.access_token.clone(),
            expires_at: access_token.expires_at,
            refresh_token: access_token.refresh_token.clone(),
            client_id: device_client.map(|client| client.client_id.clone()),
            client_secret: device_client.map(|client| client.client_secret.clone()),
            registration_expires_at: device_client.map(|client| client.registration_expires_at),
            region: sso_defaults.get_sso_region().to_string(),
            start_url: sso_defaults.get_start_url().to_string(),
        }
    }
}

impl fmt::Debug for AwsCliCachedToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AwsCliCachedToken")
            .field("access_token", &"** redacted **")
            .field("expires_at", &self.expires_at)
            .field("refresh_token", &"** redacted **")
            .field("client_id", &"** redacted **")
            .field("client_secret", &"** redacted **")
            .field("registration_expires_at", &self.registration_expires_at)
            .field("region", &self.region)
            .field("start_url", &self.start_url)
            .finish()
    }
}

fn serialize_timestamp<S>(timestamp: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&timestamp.format(AWS_CLI_TIMESTAMP_FORMAT).to_string())
}

fn serialize_optional_timestamp<S>(
    timestamp: &Option<DateTime<Utc>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match timestamp {
        Some(timestamp) => serialize_timestamp(timestamp, serializer),
        None => serializer.serialize_none(),
    }
}

pub fn cache_file_name(session_name: &str) -> String {
    let digest = Sha1::digest(session_name.as_bytes());

    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        encoded.push(hex_digit(byte >> 4));
        encoded.push(hex_digit(byte & 0x0f));
    }

    format!("{}.json", encoded)
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).unwrap_or('0')
}

pub fn cache_path(session_name: &str) -> Result<PathBuf> {
    let cache_dir = directories::UserDirs::new()
        .context("Unable to get home directory")?
        .home_dir()
        .join(".aws")
        .join("sso")
        .join("cache");

    Ok(cache_dir.join(cache_file_name(session_name)))
}

pub fn write_sso_token(
    cache_path: &Path,
    access_token: &AccessToken,
    sso_defaults: &SsoDefaults,
) -> Result<()> {
    if access_token.access_token.is_empty() {
        return Err(anyhow!(
            "There is no SSO token to share with the AWS CLI. Run 'awsctl sso -g' and try again."
        ));
    }

    if access_token.is_expired() {
        return Err(anyhow!(
            "The SSO token expired at {}, so the AWS CLI would reject it. \
             Run 'awsctl sso -g -f' and try again.",
            access_token.expires_at.format(EXPIRY_DISPLAY_FORMAT)
        ));
    }

    let token = AwsCliCachedToken::new(access_token, sso_defaults);
    let contents = serde_json::to_string(&token)
        .with_context(|| "Unable to serialise the SSO token for the AWS CLI.")?;

    write_private_file(cache_path, &contents)
}

pub fn renewal_warning(access_token: &AccessToken) -> Option<String> {
    let remaining = access_token.expires_at - Utc::now();
    if remaining.num_seconds() >= AWS_CLI_RENEWAL_WINDOW_SECONDS {
        return None;
    }

    Some(format!(
        "Warning: this SSO token expires at {}, and awsctl has no refresh token to renew it with. \
         Run 'awsctl sso -g -f' followed by 'awsctl sso -s' to give the AWS CLI a fresh one.",
        access_token.expires_at.format(EXPIRY_DISPLAY_FORMAT)
    ))
}

fn write_private_file(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        anyhow!(
            "The SSO cache path {} has no parent directory.",
            path.display()
        )
    })?;

    DirBuilder::new()
        .recursive(true)
        .mode(CACHE_DIR_MODE)
        .create(parent)
        .with_context(|| {
            format!(
                "Unable to create the SSO cache directory {}.",
                parent.display()
            )
        })?;

    let temporary_path = temporary_path(path);
    let written = fill_private_file(&temporary_path, contents).and_then(|()| {
        std::fs::rename(&temporary_path, path).with_context(|| {
            format!(
                "Unable to move the SSO token into place at {}.",
                path.display()
            )
        })
    });

    if written.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }

    written
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("token"));

    path.with_file_name(format!(".{}.{}.tmp", file_name, std::process::id()))
}

fn fill_private_file(path: &Path, contents: &str) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Unable to clear the temporary file {}.", path.display()))
        }
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(CACHE_FILE_MODE)
        .open(path)
        .with_context(|| format!("Unable to create {}.", path.display()))?;

    file.write_all(contents.as_bytes())
        .with_context(|| format!("Unable to write {}.", path.display()))?;
    file.sync_all()
        .with_context(|| format!("Unable to flush {}.", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{cache_file_name, renewal_warning, write_sso_token};
    use crate::config::{AccessToken, DeviceClient, SsoDefaults};
    use chrono::{Duration, Utc};
    use serde_json::Value;
    use std::os::unix::fs::PermissionsExt;
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

        fn cache_file(&self) -> PathBuf {
            self.path.join("cache").join("session.json")
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

    fn token() -> AccessToken {
        AccessToken {
            access_token: String::from("not-a-real-access-token"),
            expires_at: Utc::now() + Duration::hours(8),
            refresh_token: None,
            device_client: Some(DeviceClient {
                client_id: String::from("not-a-real-client-id"),
                client_secret: String::from("not-a-real-client-secret"),
                registration_expires_at: Utc::now() + Duration::days(90),
            }),
        }
    }

    fn written(cache_path: &Path) -> Value {
        let contents = std::fs::read_to_string(cache_path).expect("cache file is readable");
        serde_json::from_str(&contents).expect("cache file holds one JSON object")
    }

    #[test]
    fn names_the_file_after_the_session_name_the_way_the_aws_cli_does() {
        assert_eq!(
            cache_file_name("awsctl"),
            "7dafa79e18f5b79295b2f7ee9a3a757f30094fa6.json"
        );
        assert_eq!(
            cache_file_name("admin"),
            "d033e22ae348aeb5660fc2140aec35850c4da997.json"
        );
        assert_eq!(
            cache_file_name("dev-scopes"),
            "75e4d41276d8bd17f85986fc6cccef29fd725ce3.json"
        );
    }

    #[test]
    fn names_a_different_file_for_a_session_name_that_is_not_the_default() {
        assert_ne!(cache_file_name("awsctl"), cache_file_name("labflow"));
        assert_eq!(
            cache_file_name("labflow"),
            "657b07113a2dea8df3affc01fbca5c97fbb2cb09.json"
        );
    }

    #[test]
    fn writes_the_fields_the_aws_cli_reads() {
        let dir = TempDir::new("fields");
        let cache_path = dir.cache_file();
        let token = token();

        write_sso_token(&cache_path, &token, &defaults()).expect("writes");

        let cached = written(&cache_path);
        assert_eq!(
            cached["accessToken"],
            Value::from("not-a-real-access-token")
        );
        assert_eq!(
            cached["expiresAt"],
            Value::from(token.expires_at.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        );
        assert_eq!(cached["clientId"], Value::from("not-a-real-client-id"));
        assert_eq!(
            cached["clientSecret"],
            Value::from("not-a-real-client-secret")
        );
        assert_eq!(cached["region"], Value::from("us-east-1"));
        assert_eq!(
            cached["startUrl"],
            Value::from("https://labflow.awsapps.com/start")
        );
        assert!(cached["registrationExpiresAt"].is_string());
    }

    #[test]
    fn leaves_out_the_refresh_token_when_awsctl_does_not_have_one() {
        let dir = TempDir::new("no-refresh");
        let cache_path = dir.cache_file();

        write_sso_token(&cache_path, &token(), &defaults()).expect("writes");

        assert_eq!(written(&cache_path).get("refreshToken"), None);
    }

    #[test]
    fn writes_the_refresh_token_when_awsctl_has_one() {
        let dir = TempDir::new("refresh");
        let cache_path = dir.cache_file();
        let token = AccessToken {
            refresh_token: Some(String::from("not-a-real-refresh-token")),
            ..token()
        };

        write_sso_token(&cache_path, &token, &defaults()).expect("writes");

        assert_eq!(
            written(&cache_path)["refreshToken"],
            Value::from("not-a-real-refresh-token")
        );
    }

    #[test]
    fn leaves_out_the_registration_when_awsctl_does_not_have_one() {
        let dir = TempDir::new("no-registration");
        let cache_path = dir.cache_file();
        let token = AccessToken {
            device_client: None,
            ..token()
        };

        write_sso_token(&cache_path, &token, &defaults()).expect("writes");

        let cached = written(&cache_path);
        assert_eq!(cached.get("clientId"), None);
        assert_eq!(cached.get("clientSecret"), None);
        assert_eq!(cached.get("registrationExpiresAt"), None);
        assert!(cached["accessToken"].is_string());
    }

    #[test]
    fn writes_a_file_and_a_directory_only_their_owner_can_read() {
        let dir = TempDir::new("permissions");
        let cache_path = dir.cache_file();

        write_sso_token(&cache_path, &token(), &defaults()).expect("writes");

        let file_mode = std::fs::metadata(&cache_path)
            .expect("cache file exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);

        let dir_mode = std::fs::metadata(cache_path.parent().expect("cache directory"))
            .expect("cache directory exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn replaces_a_cache_file_from_an_earlier_run_instead_of_appending_to_it() {
        let dir = TempDir::new("replaces");
        let cache_path = dir.cache_file();

        write_sso_token(&cache_path, &token(), &defaults()).expect("first write");

        let renewed = AccessToken {
            access_token: String::from("not-a-real-second-access-token"),
            ..token()
        };
        write_sso_token(&cache_path, &renewed, &defaults()).expect("second write");

        let contents = std::fs::read_to_string(&cache_path).expect("cache file is readable");
        assert_eq!(contents.matches("\"accessToken\"").count(), 1);
        assert_eq!(
            written(&cache_path)["accessToken"],
            Value::from("not-a-real-second-access-token")
        );
    }

    #[test]
    fn replaces_a_cache_file_the_aws_cli_wrote_itself() {
        let dir = TempDir::new("replaces-cli");
        let cache_path = dir.cache_file();
        std::fs::create_dir_all(cache_path.parent().expect("cache directory"))
            .expect("cache directory");
        std::fs::write(
            &cache_path,
            r#"{"accessToken":"stale","expiresAt":"2020-01-01T00:00:00Z"}"#,
        )
        .expect("seed cache file");

        write_sso_token(&cache_path, &token(), &defaults()).expect("writes");

        assert_eq!(
            written(&cache_path)["accessToken"],
            Value::from("not-a-real-access-token")
        );
    }

    #[test]
    fn refuses_to_write_a_token_that_has_already_expired() {
        let dir = TempDir::new("expired");
        let cache_path = dir.cache_file();
        let expired = AccessToken {
            expires_at: Utc::now() - Duration::hours(1),
            ..token()
        };

        let error = write_sso_token(&cache_path, &expired, &defaults()).expect_err("refuses");

        assert!(error.to_string().contains("expired"));
        assert!(!error.to_string().contains("not-a-real-access-token"));
        assert!(!cache_path.exists());
    }

    #[test]
    fn refuses_to_write_a_token_that_was_never_set() {
        let dir = TempDir::new("unset");
        let cache_path = dir.cache_file();

        let error = write_sso_token(&cache_path, &AccessToken::default(), &defaults())
            .expect_err("refuses");

        assert!(error.to_string().contains("no SSO token"));
        assert!(!cache_path.exists());
    }

    #[test]
    fn warns_when_the_token_is_too_close_to_expiry_for_the_aws_cli() {
        let expiring = AccessToken {
            expires_at: Utc::now() + Duration::minutes(5),
            ..token()
        };

        let warning = renewal_warning(&expiring).expect("warns");

        assert!(warning.contains("expires at"));
        assert!(!warning.contains("not-a-real-access-token"));
    }

    #[test]
    fn stays_quiet_when_the_token_has_hours_left() {
        assert_eq!(renewal_warning(&token()), None);
    }
}
