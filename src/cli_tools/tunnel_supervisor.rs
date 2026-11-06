use anyhow::{anyhow, Context, Result};
use aws_config::SdkConfig;
use std::future::Future;
use std::io::Write;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::signal::unix::{signal, Signal, SignalKind};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::aws::eks::ClusterEndpoint;
use crate::aws::ssm::{stop_plugin, PortForwardSession};
use crate::aws::SsmProvider;
use crate::cli_tools::sdk_config::{profile_sdk_config, stored_role_sdk_config};
use crate::cli_tools::tunnel_backoff::Backoff;
use crate::cli_tools::tunnel_credentials::{classify, FailureKind};
use crate::cli_tools::tunnel_keepalive::{keep_alive, ApiProbe};
use crate::cli_tools::tunnel_status::{
    process_is_alive, remove_stale, scan_statuses, status_dir, status_path, unix_now, write_status,
    StatusFileGuard, SupervisorStatus, TunnelState, TunnelStatus,
};

pub const EKS_API_PORT: i32 = 443;
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
const LISTEN_POLL: Duration = Duration::from_millis(250);
const LOGIN_RECHECK: Duration = Duration::from_secs(60);
const OUTPUT_DRAIN: Duration = Duration::from_secs(1);
const ROUTINE_PLUGIN_LINES: &[&str] = &[
    "Starting session with SessionId",
    "opened for sessionId",
    "Waiting for connections",
    "Connection accepted for session",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelAccount {
    Profile(String),
    StoredRole,
}

impl TunnelAccount {
    pub async fn load_sdk_config(&self, region: &str) -> Result<SdkConfig> {
        match self {
            TunnelAccount::Profile(profile) => Ok(profile_sdk_config(profile, Some(region)).await),
            TunnelAccount::StoredRole => stored_role_sdk_config(region),
        }
    }

    pub fn login_hint(&self) -> String {
        match self {
            TunnelAccount::Profile(profile) => format!(
                "run `aws sso login --profile {}` (or `awsctl sso --refresh` for a profile awsctl wrote)",
                profile
            ),
            TunnelAccount::StoredRole => String::from("run `awsctl sso --refresh`"),
        }
    }
}

pub struct PreparedTunnel {
    pub account: TunnelAccount,
    pub cluster_name: String,
    pub aws_profile: String,
    pub region: String,
    pub endpoint: ClusterEndpoint,
    pub bastion_id: String,
    pub local_port: u16,
    pub kubeconfig: PathBuf,
    pub probe: ApiProbe,
}

impl PreparedTunnel {
    fn initial_status(&self, now: u64) -> TunnelStatus {
        TunnelStatus {
            cluster: self.cluster_name.clone(),
            profile: self.aws_profile.clone(),
            region: self.region.clone(),
            local_port: self.local_port,
            bastion: self.bastion_id.clone(),
            state: TunnelState::Starting,
            state_since: now,
            up_since: None,
            drops: 0,
            last_drop: None,
        }
    }

    fn log(&self, message: &str) {
        say(&format!(
            "{} [{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            self.cluster_name,
            message
        ));
    }
}

enum SessionOutcome {
    Stopped,
    NotStarted(anyhow::Error),
    Dropped { reason: String, lasted: Duration },
}

#[derive(Clone)]
struct StatusBoard {
    status: Arc<Mutex<SupervisorStatus>>,
    path: PathBuf,
}

impl StatusBoard {
    fn update(&self, index: usize, change: impl FnOnce(&TunnelStatus) -> TunnelStatus) {
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        let changed = change(&status.tunnels[index]);
        status.tunnels[index] = changed;

        if let Err(error) = write_status(&self.path, &status) {
            say(&format!(
                "Warning: unable to update {}: {:#}",
                self.path.display(),
                error
            ));
        }
    }
}

pub async fn supervise(tunnels: Vec<PreparedTunnel>) -> Result<()> {
    let dir = status_dir()?;
    remove_stale(&scan_statuses(&dir, process_is_alive)?.stale);
    let path = status_path(&dir, std::process::id());
    let now = unix_now();
    let status = SupervisorStatus {
        pid: std::process::id(),
        started_at: now,
        tunnels: tunnels
            .iter()
            .map(|tunnel| tunnel.initial_status(now))
            .collect(),
    };
    write_status(&path, &status)?;
    let _guard = StatusFileGuard::new(path.clone());
    let board = StatusBoard {
        status: Arc::new(Mutex::new(status)),
        path,
    };

    let mut signals = ShutdownSignals::new()?;
    let (stop_sender, stop) = watch::channel(false);
    let mut running = JoinSet::new();
    for (index, tunnel) in tunnels.into_iter().enumerate() {
        running.spawn(run_tunnel(index, tunnel, board.clone(), stop.clone()));
    }

    let received = signals.recv().await;
    say(&format!("\n{} received, closing every tunnel...", received));
    let _ = stop_sender.send(true);

    let closed = async {
        while let Some(joined) = running.join_next().await {
            if let Err(error) = joined {
                say(&format!(
                    "Warning: a tunnel stopped unexpectedly: {}",
                    error
                ));
            }
        }
    };
    tokio::select! {
        _ = closed => say("All tunnels closed."),
        again = signals.recv() => {
            say(&format!("{} received again, exiting without waiting for the sessions to end.", again));
        }
    }

    Ok(())
}

async fn run_tunnel(
    index: usize,
    tunnel: PreparedTunnel,
    board: StatusBoard,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = Backoff::default();

    loop {
        let delay = match run_session(index, &tunnel, &board, &mut stop).await {
            SessionOutcome::Stopped => break,
            SessionOutcome::Dropped { reason, lasted } => {
                backoff = backoff.after_session(lasted);
                report_retry(index, &tunnel, &board, &reason, backoff.delay());
                backoff.delay()
            }
            SessionOutcome::NotStarted(error) => match classify(&error) {
                FailureKind::NeedsLogin => {
                    report_needs_login(index, &tunnel, &board, &error);
                    if wait_for_login(&tunnel, &mut stop).await == LoginWait::Stopped {
                        break;
                    }
                    tunnel.log("credentials work again, reconnecting");
                    backoff = Backoff::default();
                    continue;
                }
                FailureKind::Transient => {
                    let reason = format!("{:#}", error);
                    report_retry(index, &tunnel, &board, &reason, backoff.delay());
                    backoff.delay()
                }
            },
        };

        if until_stopped(&mut stop, tokio::time::sleep(delay))
            .await
            .is_none()
        {
            break;
        }
        backoff = backoff.failed();
    }

    board.update(index, |status| status.stopped(unix_now()));
    tunnel.log("closed");
}

fn report_retry(
    index: usize,
    tunnel: &PreparedTunnel,
    board: &StatusBoard,
    reason: &str,
    delay: Duration,
) {
    board.update(index, |status| {
        status.dropped(TunnelState::Reconnecting, reason, unix_now())
    });
    tunnel.log(&format!(
        "down: {}; reconnecting in {}s",
        reason,
        delay.as_secs()
    ));
}

fn report_needs_login(
    index: usize,
    tunnel: &PreparedTunnel,
    board: &StatusBoard,
    error: &anyhow::Error,
) {
    let hint = tunnel.account.login_hint();
    board.update(index, |status| {
        status.dropped(
            TunnelState::NeedsLogin,
            &format!("credentials expired or missing; {}", hint),
            unix_now(),
        )
    });
    tunnel.log(&format!(
        "needs login: the credentials for profile '{}' are expired or missing ({:#}). \
         To resume, {}. Checking again every {}s.",
        tunnel.aws_profile,
        error,
        hint,
        LOGIN_RECHECK.as_secs()
    ));
}

async fn run_session(
    index: usize,
    tunnel: &PreparedTunnel,
    board: &StatusBoard,
    stop: &mut watch::Receiver<bool>,
) -> SessionOutcome {
    let started = until_stopped(stop, async {
        let ssm = connect(tunnel).await?;
        let session = ssm
            .start_port_forward_session(
                &tunnel.bastion_id,
                &tunnel.endpoint.endpoint_host,
                i32::from(tunnel.local_port),
                EKS_API_PORT,
            )
            .await?;
        Ok::<_, anyhow::Error>((ssm, session))
    })
    .await;

    let (ssm, session) = match started {
        None => return SessionOutcome::Stopped,
        Some(Err(error)) => return SessionOutcome::NotStarted(error),
        Some(Ok(started)) => started,
    };
    let outcome = carry_session(index, tunnel, board, &session, stop).await;
    ssm.terminate_session(&session.session_id).await;

    outcome
}

async fn connect(tunnel: &PreparedTunnel) -> Result<SsmProvider> {
    let config = tunnel.account.load_sdk_config(&tunnel.region).await?;
    let ssm = SsmProvider::new(&config);
    ssm.ensure_instance_online(&tunnel.bastion_id).await?;

    Ok(ssm)
}

async fn carry_session(
    index: usize,
    tunnel: &PreparedTunnel,
    board: &StatusBoard,
    session: &PortForwardSession,
    stop: &mut watch::Receiver<bool>,
) -> SessionOutcome {
    let (mut plugin, output) = match spawn_plugin(session) {
        Ok(spawned) => spawned,
        Err(error) => return SessionOutcome::NotStarted(error),
    };

    let listening = tokio::select! {
        ready = wait_until_listening(tunnel.local_port) => ready,
        status = plugin.wait() => {
            return SessionOutcome::Dropped {
                reason: output.exit_reason(status).await,
                lasted: Duration::ZERO,
            };
        }
        _ = stopped(stop) => {
            let _ = stop_plugin(&mut plugin).await;
            return SessionOutcome::Stopped;
        }
    };
    if let Err(error) = listening {
        let _ = stop_plugin(&mut plugin).await;
        return SessionOutcome::Dropped {
            reason: format!("{:#}", error),
            lasted: Duration::ZERO,
        };
    }

    board.update(index, |status| status.up(unix_now()));
    tunnel.log(&format!(
        "up: 127.0.0.1:{} -> {}:{} via {}",
        tunnel.local_port, tunnel.endpoint.endpoint_host, EKS_API_PORT, tunnel.bastion_id
    ));
    let came_up = Instant::now();

    tokio::select! {
        status = plugin.wait() => SessionOutcome::Dropped {
            reason: output.exit_reason(status).await,
            lasted: came_up.elapsed(),
        },
        reason = keep_alive(
            || tunnel.probe.reaches_api_server(),
            |failures, reason| tunnel.log(&format!("keep-alive probe {} failed: {}", failures, reason)),
        ) => {
            let _ = stop_plugin(&mut plugin).await;
            SessionOutcome::Dropped {
                reason: format!("keep-alive probes failed: {}", reason),
                lasted: came_up.elapsed(),
            }
        },
        _ = stopped(stop) => {
            let _ = stop_plugin(&mut plugin).await;
            SessionOutcome::Stopped
        }
    }
}

struct PluginOutput {
    stdout: JoinHandle<Option<String>>,
    stderr: JoinHandle<Option<String>>,
}

impl PluginOutput {
    async fn exit_reason(self, status: std::io::Result<ExitStatus>) -> String {
        let last_line = |reader: JoinHandle<Option<String>>| async move {
            tokio::time::timeout(OUTPUT_DRAIN, reader)
                .await
                .ok()
                .and_then(|joined| joined.ok())
                .flatten()
        };
        let stderr = last_line(self.stderr).await;
        let stdout = last_line(self.stdout).await;
        let status = match status {
            Ok(status) => status.to_string(),
            Err(error) => error.to_string(),
        };

        match stderr.or(stdout) {
            Some(line) => format!("{} (session-manager-plugin {})", line, status),
            None => format!("session-manager-plugin exited ({})", status),
        }
    }
}

fn spawn_plugin(session: &PortForwardSession) -> Result<(Child, PluginOutput)> {
    let mut plugin = session
        .plugin_command()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // The terminal sends Ctrl-C to its whole foreground process group.
        .process_group(0)
        .spawn()
        .with_context(|| "Unable to start session-manager-plugin.")?;

    let output = PluginOutput {
        stdout: tokio::spawn(last_telling_line(plugin.stdout.take())),
        stderr: tokio::spawn(last_telling_line(plugin.stderr.take())),
    };

    Ok((plugin, output))
}

async fn last_telling_line(stream: Option<impl AsyncRead + Unpin>) -> Option<String> {
    let mut lines = BufReader::new(stream?).lines();
    let mut last = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if !line.is_empty() && !is_routine_plugin_line(line) {
            last = Some(line.to_string());
        }
    }
    last
}

fn is_routine_plugin_line(line: &str) -> bool {
    ROUTINE_PLUGIN_LINES
        .iter()
        .any(|routine| line.contains(routine))
}

async fn wait_until_listening(port: u16) -> Result<()> {
    let deadline = Instant::now() + LISTEN_TIMEOUT;
    loop {
        if TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "session-manager-plugin did not open 127.0.0.1:{} within {}s",
                port,
                LISTEN_TIMEOUT.as_secs()
            ));
        }
        tokio::time::sleep(LISTEN_POLL).await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginWait {
    CredentialsWork,
    Stopped,
}

async fn wait_for_login(tunnel: &PreparedTunnel, stop: &mut watch::Receiver<bool>) -> LoginWait {
    loop {
        let checked = until_stopped(stop, async {
            tokio::time::sleep(LOGIN_RECHECK).await;
            connect(tunnel).await
        })
        .await;

        match checked {
            None => return LoginWait::Stopped,
            Some(Err(error)) if classify(&error) == FailureKind::NeedsLogin => {}
            Some(_) => return LoginWait::CredentialsWork,
        }
    }
}

// After SIGHUP the terminal is gone, and println! would panic writing to it
// in the middle of closing the sessions.
fn say(message: &str) {
    let _ = writeln!(std::io::stdout(), "{}", message);
}

async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopped| *stopped).await;
}

async fn until_stopped<T>(
    stop: &mut watch::Receiver<bool>,
    work: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        done = work => Some(done),
        _ = stopped(stop) => None,
    }
}

struct ShutdownSignals {
    interrupt: Signal,
    terminate: Signal,
    hangup: Signal,
}

impl ShutdownSignals {
    fn new() -> Result<Self> {
        let listen = |kind: SignalKind| {
            signal(kind).with_context(|| "Unable to listen for shutdown signals.")
        };
        Ok(ShutdownSignals {
            interrupt: listen(SignalKind::interrupt())?,
            terminate: listen(SignalKind::terminate())?,
            hangup: listen(SignalKind::hangup())?,
        })
    }

    async fn recv(&mut self) -> &'static str {
        tokio::select! {
            _ = self.interrupt.recv() => "Ctrl-C",
            _ = self.terminate.recv() => "SIGTERM",
            _ = self.hangup.recv() => "SIGHUP",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_routine_plugin_line, last_telling_line, until_stopped, TunnelAccount};
    use tokio::sync::watch;

    #[test]
    fn ignores_the_lines_every_healthy_session_prints() {
        for line in [
            "Starting session with SessionId: example-0123456789abcdef0",
            "Port 21443 opened for sessionId example-0123456789abcdef0.",
            "Waiting for connections...",
            "Connection accepted for session [example-0123456789abcdef0]",
        ] {
            assert!(is_routine_plugin_line(line), "{line}");
        }
        assert!(!is_routine_plugin_line(
            "SessionId: example-0 : Your session timed out due to inactivity and has been terminated."
        ));
    }

    #[tokio::test]
    async fn keeps_the_last_line_that_explains_a_drop() {
        let output: &[u8] = b"Starting session with SessionId: x\n\
            Your session timed out due to inactivity and has been terminated.\n\
            \n\
            Waiting for connections...\n";

        let line = last_telling_line(Some(output)).await;

        assert_eq!(
            line.as_deref(),
            Some("Your session timed out due to inactivity and has been terminated.")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn abandons_work_once_stopped() {
        let (sender, mut stop) = watch::channel(false);
        sender.send(true).expect("sends");

        let finished = until_stopped(&mut stop, std::future::pending::<()>()).await;

        assert!(finished.is_none());
    }

    #[test]
    fn tells_a_profile_user_how_to_log_in_again() {
        let hint = TunnelAccount::Profile(String::from("dev")).login_hint();

        assert!(hint.contains("aws sso login --profile dev"));
        assert!(TunnelAccount::StoredRole
            .login_hint()
            .contains("awsctl sso --refresh"));
    }
}
