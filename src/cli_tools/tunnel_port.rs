use anyhow::{anyhow, Context, Result};
use sha1::{Digest, Sha1};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, TcpListener};

// Below the ephemeral range on both macOS and Linux, so the OS never hands
// these ports out on its own.
const DEFAULT_PORT_FIRST: u16 = 20000;
const DEFAULT_PORT_COUNT: u16 = 10000;

pub fn default_local_port(cluster_name: &str) -> u16 {
    let digest = Sha1::digest(cluster_name.as_bytes());
    let seed = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    let offset = seed % u32::from(DEFAULT_PORT_COUNT);

    DEFAULT_PORT_FIRST + u16::try_from(offset).unwrap_or(0)
}

pub fn ensure_port_free(port: u16) -> Result<()> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AddrInUse => Err(anyhow!(
            "Local port {} is already in use, perhaps by another tunnel. \
             Pass --local-port with a free port.",
            port
        )),
        Err(error) => Err(error).with_context(|| format!("Unable to check local port {}.", port)),
    }
}

#[cfg(test)]
mod tests {
    use super::{default_local_port, ensure_port_free, DEFAULT_PORT_COUNT, DEFAULT_PORT_FIRST};
    use std::net::{Ipv4Addr, TcpListener};

    #[test]
    fn gives_a_cluster_the_same_port_every_time() {
        assert_eq!(
            default_local_port("example-eks"),
            default_local_port("example-eks")
        );
    }

    #[test]
    fn gives_different_clusters_different_ports() {
        assert_ne!(
            default_local_port("example-eks"),
            default_local_port("example-k8s")
        );
    }

    #[test]
    fn keeps_default_ports_in_their_range() {
        for name in ["a", "example-eks", "another-cluster", "x-y-z-1"] {
            let port = default_local_port(name);
            assert!((DEFAULT_PORT_FIRST..DEFAULT_PORT_FIRST + DEFAULT_PORT_COUNT).contains(&port));
        }
    }

    #[test]
    fn refuses_a_port_that_is_already_listening() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("binds");
        let port = listener.local_addr().expect("has an address").port();

        let error = ensure_port_free(port).expect_err("port is taken");

        assert!(error.to_string().contains("already in use"));
    }

    #[test]
    fn accepts_a_free_port() {
        let port = {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("binds");
            listener.local_addr().expect("has an address").port()
        };

        ensure_port_free(port).expect("port is free");
    }
}
