use anyhow::{anyhow, Context, Result};
use kube::config::{
    AuthInfo, Cluster, Context as KubeContext, ExecConfig, ExecInteractiveMode, Kubeconfig,
    NamedAuthInfo, NamedCluster, NamedContext,
};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::aws::eks::ClusterEndpoint;
use crate::utils::private_file::replace_private_file;

const EXEC_API_VERSION: &str = "client.authentication.k8s.io/v1beta1";
const MAX_CLUSTER_NAME_LENGTH: usize = 100;
const CONTEXT_PREFIX: &str = "ssm-";
const MERGED_KUBECONFIG_NAME: &str = "awsctl-tunnels.yaml";

pub struct TunnelTarget<'a> {
    pub cluster_name: &'a str,
    pub region: &'a str,
    pub aws_profile: &'a str,
    pub endpoint: &'a ClusterEndpoint,
    pub local_port: u16,
}

pub fn validate_cluster_name(cluster_name: &str) -> Result<()> {
    let mut characters = cluster_name.chars();
    let starts_well = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric());
    let rest_is_valid =
        characters.all(|character| character.is_ascii_alphanumeric() || "-_".contains(character));

    if starts_well && rest_is_valid && cluster_name.len() <= MAX_CLUSTER_NAME_LENGTH {
        Ok(())
    } else {
        Err(anyhow!(
            "'{}' is not a valid EKS cluster name.",
            cluster_name
        ))
    }
}

pub fn context_name(cluster_name: &str) -> String {
    format!("{}{}", CONTEXT_PREFIX, cluster_name)
}

pub fn default_kubeconfig_path(cluster_name: &str) -> Result<PathBuf> {
    Ok(kube_dir()?.join(format!("{}.yaml", context_name(cluster_name))))
}

pub fn default_merged_kubeconfig_path() -> Result<PathBuf> {
    Ok(kube_dir()?.join(MERGED_KUBECONFIG_NAME))
}

fn kube_dir() -> Result<PathBuf> {
    let home = directories::UserDirs::new()
        .context("Unable to get home directory")?
        .home_dir()
        .to_path_buf();

    Ok(home.join(".kube"))
}

pub fn with_tunnel(existing: Kubeconfig, target: &TunnelTarget) -> Kubeconfig {
    let name = context_name(target.cluster_name);

    Kubeconfig {
        clusters: replace_named(existing.clusters, tunnel_cluster(&name, target), |entry| {
            &entry.name
        }),
        auth_infos: replace_named(existing.auth_infos, tunnel_user(&name, target), |entry| {
            &entry.name
        }),
        contexts: replace_named(existing.contexts, tunnel_context(&name), |entry| {
            &entry.name
        }),
        current_context: existing.current_context.or(Some(name)),
        api_version: existing.api_version.or(Some(String::from("v1"))),
        kind: existing.kind.or(Some(String::from("Config"))),
        ..existing
    }
}

pub fn with_tunnels(existing: Kubeconfig, targets: &[TunnelTarget]) -> Kubeconfig {
    targets.iter().fold(existing, with_tunnel)
}

pub fn read_kubeconfig(path: &Path) -> Result<Kubeconfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => Kubeconfig::from_yaml(&text)
            .with_context(|| format!("Unable to parse the kubeconfig at {}.", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Kubeconfig::default()),
        Err(error) => Err(error)
            .with_context(|| format!("Unable to read the kubeconfig at {}.", path.display())),
    }
}

pub fn write_kubeconfig(path: &Path, kubeconfig: &Kubeconfig) -> Result<()> {
    let contents = serde_saphyr::to_string(kubeconfig)
        .with_context(|| "Unable to serialise the kubeconfig.")?;
    let symlink_target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    replace_private_file(&symlink_target, &contents)
}

fn replace_named<T: Clone>(
    entries: Vec<T>,
    replacement: T,
    name_of: impl Fn(&T) -> &String,
) -> Vec<T> {
    let replacement_name = name_of(&replacement).clone();
    let already_present = entries
        .iter()
        .any(|entry| *name_of(entry) == replacement_name);
    let updated: Vec<T> = entries
        .into_iter()
        .map(|entry| {
            if *name_of(&entry) == replacement_name {
                replacement.clone()
            } else {
                entry
            }
        })
        .collect();

    if already_present {
        updated
    } else {
        updated.into_iter().chain([replacement]).collect()
    }
}

fn tunnel_cluster(name: &str, target: &TunnelTarget) -> NamedCluster {
    NamedCluster {
        name: name.to_string(),
        cluster: Some(Cluster {
            server: Some(format!("https://127.0.0.1:{}", target.local_port)),
            tls_server_name: Some(target.endpoint.endpoint_host.clone()),
            certificate_authority_data: Some(target.endpoint.certificate_authority_data.clone()),
            ..Cluster::default()
        }),
        ..NamedCluster::default()
    }
}

fn tunnel_user(name: &str, target: &TunnelTarget) -> NamedAuthInfo {
    let args = [
        "--region",
        target.region,
        "eks",
        "get-token",
        "--cluster-name",
        target.cluster_name,
        "--output",
        "json",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    let profile_env = HashMap::from([
        (String::from("name"), String::from("AWS_PROFILE")),
        (String::from("value"), target.aws_profile.to_string()),
    ]);

    NamedAuthInfo {
        name: name.to_string(),
        auth_info: Some(AuthInfo {
            exec: Some(ExecConfig {
                api_version: Some(String::from(EXEC_API_VERSION)),
                command: Some(String::from("aws")),
                args: Some(args),
                env: Some(vec![profile_env]),
                interactive_mode: Some(ExecInteractiveMode::Never),
                ..ExecConfig::default()
            }),
            ..AuthInfo::default()
        }),
        ..NamedAuthInfo::default()
    }
}

fn tunnel_context(name: &str) -> NamedContext {
    NamedContext {
        name: name.to_string(),
        context: Some(KubeContext {
            cluster: name.to_string(),
            user: Some(name.to_string()),
            ..KubeContext::default()
        }),
        ..NamedContext::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        context_name, read_kubeconfig, validate_cluster_name, with_tunnel, with_tunnels,
        write_kubeconfig, TunnelTarget,
    };
    use crate::aws::eks::ClusterEndpoint;
    use kube::config::Kubeconfig;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    const ENDPOINT_HOST: &str = "EXAMPLE0123456789.gr7.eu-west-2.eks.amazonaws.com";

    fn endpoint() -> ClusterEndpoint {
        ClusterEndpoint {
            endpoint_host: String::from(ENDPOINT_HOST),
            certificate_authority_data: String::from("Q0EtREFUQQ=="),
            vpc_id: String::from("vpc-00000000000000000"),
            private_access: true,
        }
    }

    fn target<'a>(endpoint: &'a ClusterEndpoint, cluster_name: &'a str) -> TunnelTarget<'a> {
        TunnelTarget {
            cluster_name,
            region: "eu-west-2",
            aws_profile: "example-profile",
            endpoint,
            local_port: 21443,
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "awsctl-kubeconfig-{}-{}",
            label,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn points_the_cluster_at_the_local_port_and_verifies_the_real_endpoint() {
        let endpoint = endpoint();
        let kubeconfig = with_tunnel(Kubeconfig::default(), &target(&endpoint, "example-eks"));

        let cluster = kubeconfig.clusters[0].cluster.as_ref().expect("cluster");
        assert_eq!(kubeconfig.clusters[0].name, "ssm-example-eks");
        assert_eq!(cluster.server.as_deref(), Some("https://127.0.0.1:21443"));
        assert_eq!(cluster.tls_server_name.as_deref(), Some(ENDPOINT_HOST));
        assert_eq!(
            cluster.certificate_authority_data.as_deref(),
            Some("Q0EtREFUQQ==")
        );
        assert_eq!(cluster.insecure_skip_tls_verify, None);
    }

    #[test]
    fn authenticates_with_aws_eks_get_token_for_the_same_profile_and_region() {
        let endpoint = endpoint();
        let kubeconfig = with_tunnel(Kubeconfig::default(), &target(&endpoint, "example-eks"));

        let exec = kubeconfig.auth_infos[0]
            .auth_info
            .as_ref()
            .and_then(|user| user.exec.as_ref())
            .expect("exec");
        assert_eq!(exec.command.as_deref(), Some("aws"));
        assert_eq!(
            exec.args.clone().expect("args"),
            [
                "--region",
                "eu-west-2",
                "eks",
                "get-token",
                "--cluster-name",
                "example-eks",
                "--output",
                "json"
            ]
        );
        let env = &exec.env.as_ref().expect("env")[0];
        assert_eq!(env["name"], "AWS_PROFILE");
        assert_eq!(env["value"], "example-profile");
    }

    #[test]
    fn makes_the_tunnel_the_current_context_of_a_new_file() {
        let endpoint = endpoint();
        let kubeconfig = with_tunnel(Kubeconfig::default(), &target(&endpoint, "example-eks"));

        assert_eq!(
            kubeconfig.current_context.as_deref(),
            Some("ssm-example-eks")
        );
        assert_eq!(kubeconfig.contexts[0].name, "ssm-example-eks");
        let context = kubeconfig.contexts[0].context.as_ref().expect("context");
        assert_eq!(context.cluster, "ssm-example-eks");
        assert_eq!(context.user.as_deref(), Some("ssm-example-eks"));
    }

    #[test]
    fn merges_beside_other_contexts_without_switching_the_current_one() {
        let existing = Kubeconfig::from_yaml(
            "apiVersion: v1
kind: Config
current-context: other
clusters:
- name: other
  cluster:
    server: https://other.example.com
users:
- name: other
  user:
    token: other-token
contexts:
- name: other
  context:
    cluster: other
    user: other
",
        )
        .expect("parses");
        let endpoint = endpoint();

        let merged = with_tunnel(existing, &target(&endpoint, "example-eks"));

        assert_eq!(merged.current_context.as_deref(), Some("other"));
        let names: Vec<&str> = merged.contexts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["other", "ssm-example-eks"]);
        assert_eq!(merged.clusters.len(), 2);
        assert_eq!(merged.auth_infos.len(), 2);
    }

    #[test]
    fn replaces_the_entries_from_an_earlier_run_instead_of_duplicating_them() {
        let endpoint = endpoint();
        let first = with_tunnel(Kubeconfig::default(), &target(&endpoint, "example-eks"));
        let moved = TunnelTarget {
            local_port: 22443,
            ..target(&endpoint, "example-eks")
        };

        let second = with_tunnel(first, &moved);

        assert_eq!(second.clusters.len(), 1);
        assert_eq!(second.auth_infos.len(), 1);
        assert_eq!(second.contexts.len(), 1);
        let cluster = second.clusters[0].cluster.as_ref().expect("cluster");
        assert_eq!(cluster.server.as_deref(), Some("https://127.0.0.1:22443"));
    }

    #[test]
    fn merges_every_tunnel_into_one_kubeconfig() {
        let endpoint = endpoint();
        let second = TunnelTarget {
            local_port: 22443,
            aws_profile: "other-profile",
            ..target(&endpoint, "example-k8s")
        };

        let merged = with_tunnels(
            Kubeconfig::default(),
            &[target(&endpoint, "example-eks"), second],
        );

        let names: Vec<&str> = merged.contexts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["ssm-example-eks", "ssm-example-k8s"]);
        assert_eq!(merged.current_context.as_deref(), Some("ssm-example-eks"));
        let servers: Vec<Option<&str>> = merged
            .clusters
            .iter()
            .map(|c| c.cluster.as_ref().and_then(|c| c.server.as_deref()))
            .collect();
        assert_eq!(
            servers,
            [
                Some("https://127.0.0.1:21443"),
                Some("https://127.0.0.1:22443")
            ]
        );
    }

    #[test]
    fn writes_a_kubeconfig_only_its_owner_can_read_and_reads_it_back() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("ssm-example-eks.yaml");
        let endpoint = endpoint();
        let kubeconfig = with_tunnel(Kubeconfig::default(), &target(&endpoint, "example-eks"));

        write_kubeconfig(&path, &kubeconfig).expect("writes");
        let read_back = read_kubeconfig(&path).expect("reads");

        let mode = std::fs::metadata(&path)
            .expect("exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let cluster = read_back.clusters[0].cluster.as_ref().expect("cluster");
        assert_eq!(cluster.tls_server_name.as_deref(), Some(ENDPOINT_HOST));
        assert_eq!(
            read_back.current_context.as_deref(),
            Some("ssm-example-eks")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_a_missing_kubeconfig_as_empty() {
        let dir = temp_dir("missing");

        let kubeconfig = read_kubeconfig(&dir.join("absent.yaml")).expect("reads");

        assert!(kubeconfig.clusters.is_empty());
    }

    #[test]
    fn accepts_names_eks_allows() {
        for name in ["example-eks", "a", "Cluster_1"] {
            validate_cluster_name(name).expect("valid");
        }
    }

    #[test]
    fn rejects_names_that_could_escape_the_kube_directory() {
        for name in [
            "",
            "../evil",
            "a/b",
            "-leading",
            "has space",
            &"a".repeat(101),
        ] {
            assert!(validate_cluster_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn keeps_the_merged_kubeconfig_apart_from_every_cluster_file() {
        assert!(!super::MERGED_KUBECONFIG_NAME.starts_with(super::CONTEXT_PREFIX));
    }

    #[test]
    fn names_the_context_after_the_cluster() {
        assert_eq!(context_name("example-eks"), "ssm-example-eks");
    }
}
