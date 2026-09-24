pub mod access_token;
pub mod account_config;
pub mod constants;
pub mod credentials;
pub mod device_client;
pub mod profile;
pub mod sso_defaults;

pub use access_token::AccessToken;
pub use account_config::AccountConfig;
pub use credentials::Credentials;
pub use device_client::DeviceClient;
pub use profile::Profile;
pub use sso_defaults::SsoDefaults;

// #[derive(Deserialize, Serialize, Debug)]
// pub struct AWSConfig {
//     path: PathBuf,
//     config: BTreeMap<String, Profile>
// }

// pub trait Config {
//     type ConfigType: DeserializeOwned + Serialize;

//     fn new(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>>
//     where Self: Sized
//     {
//         let config = Self::load_config(&path)?;
//         Ok(Self::create(path, config))
//     }

//     fn load_config(path: &PathBuf) -> Result<Self::ConfigType, Box<dyn std::error::Error>> {
//         let contents = fs::read_to_string(path)?;
//         Self::deserialize_config(&contents)
//     }

//     fn save_config(&self) -> Result<(), Box<dyn std::error::Error>> {
//         let config = self.serialize_config()?;
//         fs::write(self.get_path(), config.as_bytes())?;
//         Ok(())
//     }

//     fn create(path: PathBuf, config: Self::ConfigType) -> Self
//     where
//         Self: Sized;

//     fn serialize_config(&self) -> Result<String, Box<dyn std::error::Error>>;

//     fn deserialize_config(contents: &str) ->  Result<Self::ConfigType, Box< dyn std::error::Error>>;

//     fn get_path(&self) -> &PathBuf;
// }

// impl Config for AWSConfig {
//     type ConfigType = BTreeMap<String, Profile>;
//     fn create(path: PathBuf, config: BTreeMap<String, Profile>) -> Self {
//         AWSConfig { path: path, config: config  }
//     }

//     fn serialize_config(&self) -> Result<String, Box<dyn std::error::Error>> {
//         Ok(serde_ini::to_string(&self.config)?)
//     }

//     fn deserialize_config(contents: &str) -> Result<Self::ConfigType, Box< dyn std::error::Error>> {
//         let config_file: Self::ConfigType = serde_ini::from_str(contents)?;
//         Ok(config_file)
//     }

//     fn get_path(&self) -> &PathBuf  {
//         &self.path
//     }
// }

// impl Config for AccountConfig {
//     type ConfigType = BTreeMap<String, Value>;
//     fn create(path: std::path::PathBuf, config: BTreeMap<String, Value>) -> Self {
//         AccountConfig { path: path, config: config }
//     }

//     fn serialize_config(&self) -> Result<String, Box<dyn std::error::Error>> {
//         Ok(toml::to_string(&self.config)?)
//     }

//     fn deserialize_config(contents: &str) -> Result<Self::ConfigType, Box< dyn std::error::Error>> {
//         let toml: Self::ConfigType = toml::from_str(contents)?;
//         Ok(toml)
//     }

//     fn get_path(&self) -> &PathBuf {
//         &self.path
//     }
// }
