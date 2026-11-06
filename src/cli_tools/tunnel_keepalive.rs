use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::future::Future;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::MissedTickBehavior;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::aws::eks::ClusterEndpoint;

// Session Manager ends a session after 20 idle minutes by default, and an
// account can lower that. Probing every two minutes keeps traffic flowing
// well inside it, and notices a dead tunnel within a few minutes.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(2 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const FAILURES_BEFORE_RECONNECT: u32 = 2;
const STATUS_LINE_PREFIX: &[u8] = b"HTTP/1.";

pub struct ApiProbe {
    connector: TlsConnector,
    server_name: ServerName<'static>,
    request: String,
    local_port: u16,
}

impl ApiProbe {
    pub fn new(endpoint: &ClusterEndpoint, local_port: u16) -> Result<Self> {
        let roots = cluster_roots(&endpoint.certificate_authority_data)?;
        let provider = Arc::new(tokio_rustls::rustls::crypto::aws_lc_rs::default_provider());
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .with_context(|| "Unable to set up TLS for the keep-alive probe.")?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = ServerName::try_from(endpoint.endpoint_host.clone())
            .with_context(|| format!("'{}' is not a valid TLS name.", endpoint.endpoint_host))?;

        Ok(ApiProbe {
            connector: TlsConnector::from(Arc::new(config)),
            server_name,
            request: format!(
                "GET /livez HTTP/1.1\r\nHost: {}\r\nUser-Agent: awsctl-keepalive\r\nConnection: close\r\n\r\n",
                endpoint.endpoint_host
            ),
            local_port,
        })
    }

    pub async fn reaches_api_server(&self) -> Result<()> {
        tokio::time::timeout(PROBE_TIMEOUT, self.exchange())
            .await
            .map_err(|_| anyhow!("no answer within {}s", PROBE_TIMEOUT.as_secs()))?
    }

    async fn exchange(&self) -> Result<()> {
        let tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, self.local_port))
            .await
            .with_context(|| format!("unable to connect to 127.0.0.1:{}", self.local_port))?;
        let mut tls = self
            .connector
            .connect(self.server_name.clone(), tcp)
            .await
            .with_context(|| "TLS handshake with the API server failed")?;
        tls.write_all(self.request.as_bytes()).await?;

        let mut head = [0u8; 12];
        tls.read_exact(&mut head)
            .await
            .with_context(|| "the API server closed the connection without answering")?;
        let _ = tls.shutdown().await;

        if is_http_status_line(&head) {
            Ok(())
        } else {
            Err(anyhow!("the API server sent something other than HTTP"))
        }
    }
}

fn cluster_roots(certificate_authority_data: &str) -> Result<RootCertStore> {
    let pem = STANDARD
        .decode(certificate_authority_data.trim())
        .with_context(|| "The cluster's certificate authority is not valid base64.")?;
    let certificates: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&pem)
        .collect::<Result<_, _>>()
        .map_err(|error| anyhow!("The cluster's certificate authority is not PEM: {}", error))?;

    let mut roots = RootCertStore::empty();
    let (added, _ignored) = roots.add_parsable_certificates(certificates);
    if added == 0 {
        return Err(anyhow!(
            "The cluster's certificate authority holds no usable certificate."
        ));
    }

    Ok(roots)
}

fn is_http_status_line(head: &[u8]) -> bool {
    head.starts_with(STATUS_LINE_PREFIX)
        && head
            .get(9..12)
            .is_some_and(|code| code.iter().all(u8::is_ascii_digit))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeVerdict {
    Healthy,
    Failing { failures: u32, reason: String },
    Reconnect { reason: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProbeTracker {
    consecutive_failures: u32,
}

impl ProbeTracker {
    pub fn record(self, outcome: &Result<()>) -> (Self, ProbeVerdict) {
        match outcome {
            Ok(()) => (ProbeTracker::default(), ProbeVerdict::Healthy),
            Err(error) => {
                let failures = self.consecutive_failures + 1;
                let reason = format!("{:#}", error);
                let verdict = if failures >= FAILURES_BEFORE_RECONNECT {
                    ProbeVerdict::Reconnect { reason }
                } else {
                    ProbeVerdict::Failing { failures, reason }
                };
                (
                    ProbeTracker {
                        consecutive_failures: failures,
                    },
                    verdict,
                )
            }
        }
    }
}

pub async fn keep_alive<P, F>(mut probe: P, on_failure: impl Fn(u32, &str)) -> String
where
    P: FnMut() -> F,
    F: Future<Output = Result<()>>,
{
    let mut ticker = tokio::time::interval(PROBE_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut tracker = ProbeTracker::default();

    loop {
        ticker.tick().await;
        let outcome = probe().await;
        let (next, verdict) = tracker.record(&outcome);
        tracker = next;
        match verdict {
            ProbeVerdict::Healthy => {}
            ProbeVerdict::Failing { failures, reason } => on_failure(failures, &reason),
            ProbeVerdict::Reconnect { reason } => return reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_http_status_line, keep_alive, ApiProbe, ProbeTracker, ProbeVerdict, PROBE_INTERVAL,
    };
    use crate::aws::eks::ClusterEndpoint;
    use anyhow::anyhow;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const SSM_DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(20 * 60);

    fn endpoint(certificate_authority_data: &str) -> ClusterEndpoint {
        ClusterEndpoint {
            endpoint_host: String::from("EXAMPLE0123456789.gr7.eu-west-2.eks.amazonaws.com"),
            certificate_authority_data: String::from(certificate_authority_data),
            vpc_id: String::from("vpc-00000000000000000"),
            private_access: true,
        }
    }

    #[test]
    fn probes_well_inside_the_default_idle_timeout() {
        assert!(PROBE_INTERVAL * 4 <= SSM_DEFAULT_IDLE_TIMEOUT);
    }

    #[test]
    fn recognises_an_http_status_line() {
        assert!(is_http_status_line(b"HTTP/1.1 200"));
        assert!(is_http_status_line(b"HTTP/1.1 401"));
        assert!(!is_http_status_line(b"SSH-2.0-Open"));
        assert!(!is_http_status_line(b"HTTP/1.1 OK "));
    }

    #[test]
    fn asks_for_a_reconnect_only_after_consecutive_failures() {
        let failure = Err(anyhow!("refused"));

        let (tracker, first) = ProbeTracker::default().record(&failure);
        let (_, second) = tracker.record(&failure);

        assert_eq!(
            first,
            ProbeVerdict::Failing {
                failures: 1,
                reason: String::from("refused")
            }
        );
        assert_eq!(
            second,
            ProbeVerdict::Reconnect {
                reason: String::from("refused")
            }
        );
    }

    #[test]
    fn forgets_a_failure_once_a_probe_succeeds() {
        let (tracker, _) = ProbeTracker::default().record(&Err(anyhow!("refused")));
        let (tracker, healthy) = tracker.record(&Ok(()));
        let (_, after) = tracker.record(&Err(anyhow!("refused")));

        assert_eq!(healthy, ProbeVerdict::Healthy);
        assert!(matches!(after, ProbeVerdict::Failing { failures: 1, .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn probes_at_once_and_then_every_interval() {
        let calls = Arc::new(AtomicU32::new(0));
        let counted = calls.clone();

        let watched = tokio::time::timeout(
            PROBE_INTERVAL * 3 + Duration::from_secs(1),
            keep_alive(
                move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    async { Ok(()) }
                },
                |_, _| {},
            ),
        )
        .await;

        assert!(watched.is_err(), "a healthy tunnel is watched forever");
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_one_interval_after_the_first_failure() {
        let started = tokio::time::Instant::now();
        let failures = Arc::new(AtomicU32::new(0));
        let reported = failures.clone();

        let reason = keep_alive(
            || async { Err(anyhow!("TLS handshake failed")) },
            move |_, _| {
                reported.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await;

        assert_eq!(reason, "TLS handshake failed");
        assert_eq!(failures.load(Ordering::SeqCst), 1);
        assert_eq!(started.elapsed(), PROBE_INTERVAL);
    }

    #[test]
    fn refuses_a_certificate_authority_that_is_not_a_certificate() {
        assert!(ApiProbe::new(&endpoint("not base64!"), 21443).is_err());
        assert!(ApiProbe::new(&endpoint("Q0EtREFUQQ=="), 21443).is_err());
    }

    #[tokio::test]
    async fn fails_when_nothing_answers_on_the_port() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
            listener.local_addr().expect("has an address").port()
        };
        let probe = ApiProbe {
            local_port: port,
            ..ApiProbe::new(&endpoint(SELF_SIGNED_CA_PEM_BASE64), 21443).expect("builds")
        };

        assert!(probe.reaches_api_server().await.is_err());
    }

    const SELF_SIGNED_CA_PEM_BASE64: &str = concat!(
        "LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0tCk1JSUJnakNDQVNlZ0F3SUJBZ0lVSFhIcVZOTHAr",
        "Z2J2QlBKRHVGM3dkekNDVnhjd0NnWUlLb1pJemowRUF3SXcKRlRFVE1CRUdBMVVFQXd3S1pYaGhi",
        "WEJzWlMxallUQWdGdzB5TmpFeE1EWXhNekF6TVRoYUdBOHlNVEkyTVRBeApNekV6TURNeE9Gb3dG",
        "VEVUTUJFR0ExVUVBd3dLWlhoaGJYQnNaUzFqWVRCWk1CTUdCeXFHU000OUFnRUdDQ3FHClNNNDlB",
        "d0VIQTBJQUJIQjZhMkdzUkI3NVV5QkFhOS9GVkhwTnFzVU9WSXZTazNJSmVsQVlBeFprVFZKSS9V",
        "OVMKQjZrQk5hTnc3blJOKzlDelpRSTBYL3pGbGtqbklITUM5Sm1qVXpCUk1CMEdBMVVkRGdRV0JC",
        "U2FHYjhZSEowNQp3VW8rNHBSWTFjc2Q3Vnk1RFRBZkJnTlZIU01FR0RBV2dCU2FHYjhZSEowNXdV",
        "bys0cFJZMWNzZDdWeTVEVEFQCkJnTlZIUk1CQWY4RUJUQURBUUgvTUFvR0NDcUdTTTQ5QkFNQ0Ew",
        "a0FNRVlDSVFEYmZGWFQ1RmlCTnVrTHM0cEUKb3JMczN0N0ZNUUdxL1pqYmpvTExZNnRSdndJaEFN",
        "QnFGUVJORldxNnNycllCVThHcDBROWhnS21yVGJHOE1pbwpEV0k4VGY2YgotLS0tLUVORCBDRVJU",
        "SUZJQ0FURS0tLS0tCg==",
    );
}
