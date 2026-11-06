use anyhow::{anyhow, Context, Result};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};
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

pub fn assign_ports(requests: &[(&str, Option<u16>)]) -> Result<Vec<u16>> {
    let mut taken = BTreeSet::new();
    for (cluster, pinned) in requests {
        if let Some(port) = pinned {
            if !taken.insert(*port) {
                return Err(anyhow!(
                    "Local port {} is pinned for {} and another cluster too.",
                    port,
                    cluster
                ));
            }
        }
    }

    let mut derived_order: Vec<&str> = requests
        .iter()
        .filter(|(_, pinned)| pinned.is_none())
        .map(|(cluster, _)| *cluster)
        .collect();
    derived_order.sort_unstable();

    let mut derived = BTreeMap::new();
    for cluster in derived_order {
        let port = next_free_port(default_local_port(cluster), &taken)?;
        taken.insert(port);
        derived.insert(cluster, port);
    }

    Ok(requests
        .iter()
        .map(|(cluster, pinned)| pinned.unwrap_or_else(|| derived[cluster]))
        .collect())
}

fn next_free_port(start: u16, taken: &BTreeSet<u16>) -> Result<u16> {
    let offset = start - DEFAULT_PORT_FIRST;
    (0..DEFAULT_PORT_COUNT)
        .map(|step| DEFAULT_PORT_FIRST + (offset + step) % DEFAULT_PORT_COUNT)
        .find(|port| !taken.contains(port))
        .ok_or_else(|| anyhow!("No free local port is left for another tunnel."))
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
    use super::{
        assign_ports, default_local_port, ensure_port_free, DEFAULT_PORT_COUNT, DEFAULT_PORT_FIRST,
    };
    use std::collections::{BTreeSet, HashMap};
    use std::net::{Ipv4Addr, TcpListener};

    fn colliding_names() -> (String, String) {
        let mut seen: HashMap<u16, String> = HashMap::new();
        (0..)
            .map(|index| format!("cluster-{}", index))
            .find_map(|name| {
                let port = default_local_port(&name);
                match seen.get(&port) {
                    Some(earlier) => Some((earlier.clone(), name)),
                    None => {
                        seen.insert(port, name);
                        None
                    }
                }
            })
            .expect("finds a collision")
    }

    #[test]
    fn gives_each_cluster_its_derived_port_when_none_collide() {
        let ports = assign_ports(&[("example-eks", None), ("example-k8s", None)]).expect("assigns");

        assert_eq!(
            ports,
            [
                default_local_port("example-eks"),
                default_local_port("example-k8s")
            ]
        );
    }

    #[test]
    fn gives_many_clusters_distinct_ports_in_range() {
        let names: Vec<String> = (0..500).map(|index| format!("cluster-{}", index)).collect();
        let requests: Vec<(&str, Option<u16>)> =
            names.iter().map(|name| (name.as_str(), None)).collect();

        let ports = assign_ports(&requests).expect("assigns");

        let distinct: BTreeSet<u16> = ports.iter().copied().collect();
        assert_eq!(distinct.len(), names.len());
        assert!(ports.iter().all(|port| (DEFAULT_PORT_FIRST
            ..DEFAULT_PORT_FIRST + DEFAULT_PORT_COUNT)
            .contains(port)));
    }

    #[test]
    fn moves_the_later_of_two_colliding_names_whatever_the_order() {
        let (first, second) = colliding_names();
        let (earlier, later) = if first < second {
            (first, second)
        } else {
            (second, first)
        };

        let forward = assign_ports(&[(&earlier, None), (&later, None)]).expect("assigns");
        let backward = assign_ports(&[(&later, None), (&earlier, None)]).expect("assigns");

        assert_eq!(forward[0], default_local_port(&earlier));
        assert_ne!(forward[0], forward[1]);
        assert_eq!(forward, [backward[1], backward[0]]);
    }

    #[test]
    fn keeps_a_pinned_port_and_moves_a_derived_one_off_it() {
        let pinned = default_local_port("example-eks");

        let ports =
            assign_ports(&[("other-eks", Some(pinned)), ("example-eks", None)]).expect("assigns");

        assert_eq!(ports[0], pinned);
        assert_ne!(ports[1], pinned);
    }

    #[test]
    fn refuses_one_port_pinned_twice() {
        assert!(
            assign_ports(&[("example-eks", Some(21443)), ("example-k8s", Some(21443))]).is_err()
        );
    }

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
