use crate::config::{AccessToken, DeviceClient};
use anyhow::{anyhow, Context, Result};
use aws_sdk_ssooidc as ssooidc;
use chrono::{Duration, TimeZone, Utc};

pub struct SsoAccessTokenProvider {
    client: ssooidc::Client,
    cached_token: Option<AccessToken>,
    start_url: String,
}

impl SsoAccessTokenProvider {
    const DEVICE_GRANT_TYPE: &'static str = "urn:ietf:params:oauth:grant-type:device_code";

    pub fn new(
        start_url: String,
        region: String,
        access_token: &Option<AccessToken>,
    ) -> anyhow::Result<Self> {
        let sso_config = ssooidc::config::Builder::new()
            .behavior_version(ssooidc::config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region))
            .build();

        Ok(Self {
            client: ssooidc::client::Client::from_conf(sso_config),
            cached_token: access_token.clone(),
            start_url: start_url.to_string(),
        })
    }

    pub async fn get_access_token(&self) -> Result<AccessToken> {
        match &self.cached_token {
            Some(access_token) => {
                if access_token.is_expired() {
                    self.get_new_token().await
                } else {
                    // refresh_token(cached_token).await
                    println!("Valid access token already exists.");
                    return Ok(access_token.clone());
                }
            }
            None => self.get_new_token().await,
        }
    }

    async fn get_new_token(&self) -> Result<AccessToken> {
        let device_client = self.sso_register_device_client().await?;
        self.sso_authenticate(device_client).await
    }

    async fn sso_register_device_client(&self) -> Result<DeviceClient> {
        let register_output = self
            .client
            .register_client()
            .client_name("awsctl")
            .client_type("public")
            .send()
            .await
            .with_context(|| "Unable to register client")?;

        let client_id = register_output.client_id().unwrap();
        let client_secret = register_output.client_secret().unwrap();
        let registration_expires_at = Utc
            .timestamp_opt(register_output.client_secret_expires_at(), 0)
            .unwrap();
        let device_client = DeviceClient {
            client_id: String::from(client_id),
            client_secret: String::from(client_secret),
            registration_expires_at,
        };
        Ok(device_client)
    }

    async fn sso_authenticate(&self, device_client: DeviceClient) -> Result<AccessToken> {
        let auth_output = self
            .client
            .start_device_authorization()
            .client_id(device_client.client_id.as_str())
            .client_secret(device_client.client_secret.as_str())
            .start_url(&self.start_url)
            .send()
            .await
            .with_context(|| "Unable authorize your device")?;

        if webbrowser::open(&auth_output.verification_uri_complete().unwrap()).is_err() {
            println!("WARNING: Unable to open browser");
        }
        println!(
            "Verify here: {}",
            auth_output.verification_uri_complete().unwrap()
        );

        let interval = auth_output.interval();
        loop {
            let token_output = self
                .client
                .create_token()
                .client_id(device_client.client_id.as_str())
                .client_secret(device_client.client_secret.as_str())
                .grant_type(Self::DEVICE_GRANT_TYPE)
                .device_code(auth_output.device_code().unwrap())
                .send()
                .await;

            match token_output {
                Ok(out) => {
                    let access_token = out.access_token().unwrap();
                    // let refresh_token = out.refresh_token().unwrap();
                    // println!("{:?}", out.refresh_token());
                    let expires_at = Utc::now() + Duration::seconds(out.expires_in() as i64);

                    let access_token = AccessToken {
                        access_token: String::from(access_token),
                        device_client: Some(device_client),
                        expires_at,
                        refresh_token: None,
                    };

                    break Ok(access_token);
                }
                Err(e) => {
                    // eprintln!("There was an error {:#?}", e);
                    let service_error = e.into_service_error();
                    if service_error.is_access_denied_exception() {
                        break Err(anyhow!("Access request rejected"));
                    }
                    let millis = Duration::seconds(interval as i64);
                    std::thread::sleep(millis.to_std()?);
                }
            }
        }
    }
}

// TODO: create struct manager for sso
// pub fn creaet_client_from_p(credentials_provider: sso::credentials::SsoCredentialsProvider) -> Result<ConfigLoader, Box< dyn Error>> {
//     Ok(ConfigLoader::credentials_provider(credentials_provider))
// }
