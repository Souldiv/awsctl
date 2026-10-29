use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};

fn parse_datetime(s: &str) -> Result<DateTime<Utc>, String> {
    s.parse::<DateTime<Utc>>().map_err(|_| {
        format!(
            "Invalid date format for '{}'. Please use the format 'YYYY-MM-DDTHH:MM:SSZ'.",
            s
        )
    })
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Host {
    Ec2,
    Rds,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum SnapshotType {
    Aws,
    Eks,
    Cnpg,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Edit the config file using $EDITOR (defaults to vim)
    Edit,
}

#[derive(Subcommand)]
#[group(required = true, multiple = false)]
pub enum Commands {
    /// Setup sso configuration or authorize access to AWS
    #[group(required = true, multiple = true)]
    Sso {
        /// Setup aws profile for accessing resources
        #[arg(short, long, group = "sso")]
        setup_profile: bool,

        /// Generates an access token or refreshes the token
        #[arg(short, long, group = "sso")]
        get_access_token: bool,

        /// flag for forcing re-creation of access token
        #[arg(short, long, default_value_t = false, requires = "sso")]
        force: bool, // /// path to aws config
        // #[arg(short, long, default_value_t=String::from("default_path"), requires="sso")]
        // path: String
        /// starts a renewal session
        #[arg(short, long, default_value_t = false, group = "sso")]
        keep_alive: bool,

        /// point [default] at a profile written by --setup-profile (prompts when no name is given)
        #[arg(
            short = 'd',
            long,
            group = "sso",
            value_name = "PROFILE",
            conflicts_with_all = ["setup_profile", "get_access_token", "keep_alive", "list_profiles"]
        )]
        switch_default: Option<Option<String>>,

        /// list the profiles written by --setup-profile, marking the one behind [default]
        #[arg(
            short = 'l',
            long,
            group = "sso",
            conflicts_with_all = ["setup_profile", "get_access_token", "keep_alive"]
        )]
        list_profiles: bool,

        /// account name or account id to authenticate directly (skips interactive selection)
        #[arg(short, long, requires = "sso")]
        account: Option<String>,

        /// role name to use (skips interactive selection)
        #[arg(short = 'o', long, requires = "sso")]
        role: Option<String>,

        /// sso region to use (skips interactive selection)
        #[arg(short = 'r', long, requires = "sso")]
        region: Option<String>,
    },
    /// Display account related information
    #[group(required = true, multiple = false)]
    Info {
        /// list accounts
        #[arg(short, long)]
        accounts: bool,

        /// list roles for authenticated account
        #[arg(short, long)]
        roles: bool,

        /// function test get role credentials
        #[arg(short, long)]
        test: bool,
    },
    /// SSH into an ec2 instance or port forward to an RDS instance
    Ssh {
        /// host to ssh into or port forward to
        #[arg(value_enum)]
        host: Host,

        /// port to forward to
        #[arg(short, long, default_value_t = 3333)]
        port: i32,
    },
    /// ssm functions
    #[group(required = true, multiple = false)]
    Ssm {
        /// describe ssm document
        #[arg(short, long, group = "describe")]
        describe_document: bool,

        /// document name
        #[arg(short = 'n', long, requires = "describe")]
        document_name: Option<String>,

        /// document version
        #[arg(short = 'r', long, requires = "describe")]
        document_version: Option<String>,

        /// document version name
        #[arg(short, long, requires = "describe")]
        version_name: Option<String>,

        /// list inventory
        #[arg(short, long, group = "list")]
        list_inventory: bool,

        /// create ssm document
        #[arg(short, long, group = "create")]
        create_document: bool,

        /// document file path
        #[arg(short, long, requires = "create")]
        document_path: String,
    },
    /// manage eks clusters
    #[group(required = true, multiple = true)]
    Eks {
        /// update kubeconfig
        #[arg(short, long, group = "eks")]
        update: bool,

        /// list clusters
        #[arg(short, long, group = "eks")]
        list: bool,

        /// aws region to use (defaults to the current profile region)
        #[arg(short, long, requires = "eks")]
        region: Option<String>,

        /// cluster name to update kubeconfig for (skips interactive selection)
        #[arg(short, long, requires = "update", conflicts_with = "list")]
        cluster: Option<String>,
    },
    /// Configuration settings
    #[group(required = true, multiple = false)]
    #[command(subcommand_negates_reqs = true)]
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommands>,

        /// Display config location
        #[arg(short, long)]
        location: bool,

        /// Display config file contents
        #[arg(short, long)]
        show: bool,

        /// Create a new config from a file
        #[arg(short, long)]
        update: Option<String>,
    },
    /// Manage snapshots
    Snapshot {
        /// snapshot type
        #[arg(value_enum)]
        snapshot_type: SnapshotType,

        /// eks namespace to filter by
        #[arg(short, long)]
        namespace: String,

        /// date filter in UTC
        #[arg(short, long, value_parser = parse_datetime)]
        before_date: DateTime<Utc>,

        /// delete snapshots
        #[arg(short, long)]
        delete_all: bool,

        /// yes to confirm deletion
        #[arg(short, long)]
        yes: bool,

        /// count of snapshots
        #[arg(short, long)]
        count: bool,
    },
}
