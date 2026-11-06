use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::constants::APP_NAME;
use crate::utils::private_file::replace_private_file;

const STATUS_DIR_NAME: &str = "tunnels";
const STATUS_FILE_EXTENSION: &str = "json";
const MAX_REASON_WIDTH: usize = 60;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelState {
    Starting,
    Up,
    Reconnecting,
    NeedsLogin,
    Stopped,
}

impl TunnelState {
    fn label(self) -> &'static str {
        match self {
            TunnelState::Starting => "starting",
            TunnelState::Up => "up",
            TunnelState::Reconnecting => "reconnecting",
            TunnelState::NeedsLogin => "needs login",
            TunnelState::Stopped => "stopped",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TunnelDrop {
    pub at: u64,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TunnelStatus {
    pub cluster: String,
    pub profile: String,
    pub region: String,
    pub local_port: u16,
    pub bastion: String,
    pub state: TunnelState,
    pub state_since: u64,
    pub up_since: Option<u64>,
    pub drops: u32,
    pub last_drop: Option<TunnelDrop>,
}

impl TunnelStatus {
    pub fn up(&self, now: u64) -> Self {
        TunnelStatus {
            state: TunnelState::Up,
            state_since: now,
            up_since: Some(now),
            ..self.clone()
        }
    }

    pub fn dropped(&self, state: TunnelState, reason: &str, now: u64) -> Self {
        let was_up = self.state == TunnelState::Up;
        TunnelStatus {
            state,
            state_since: if self.state == state {
                self.state_since
            } else {
                now
            },
            up_since: None,
            drops: self.drops + u32::from(was_up),
            last_drop: Some(TunnelDrop {
                at: now,
                reason: reason.to_string(),
            }),
            ..self.clone()
        }
    }

    pub fn stopped(&self, now: u64) -> Self {
        TunnelStatus {
            state: TunnelState::Stopped,
            state_since: now,
            up_since: None,
            ..self.clone()
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SupervisorStatus {
    pub pid: u32,
    pub started_at: u64,
    pub tunnels: Vec<TunnelStatus>,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub fn status_dir() -> Result<PathBuf> {
    let config_path = confy::get_configuration_file_path(APP_NAME, None)
        .with_context(|| "Unable to locate awsctl's config directory.")?;
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("awsctl's config file has no parent directory."))?;

    Ok(config_dir.join(STATUS_DIR_NAME))
}

pub fn status_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{}.{}", pid, STATUS_FILE_EXTENSION))
}

pub fn write_status(path: &Path, status: &SupervisorStatus) -> Result<()> {
    let contents =
        serde_json::to_string_pretty(status).with_context(|| "Unable to serialise the status.")?;
    replace_private_file(path, &contents)
}

pub struct StatusFileGuard {
    path: PathBuf,
}

impl StatusFileGuard {
    pub fn new(path: PathBuf) -> Self {
        StatusFileGuard { path }
    }
}

impl Drop for StatusFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Default)]
pub struct StatusScan {
    pub live: Vec<SupervisorStatus>,
    pub stale: Vec<PathBuf>,
}

pub fn scan_statuses(dir: &Path, is_alive: impl Fn(u32) -> bool) -> Result<StatusScan> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(StatusScan::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("Unable to read {}.", dir.display()))
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == STATUS_FILE_EXTENSION)
        })
        .collect();
    paths.sort();

    let (live, stale): (Vec<_>, Vec<_>) = paths
        .into_iter()
        .map(|path| (read_status(&path).filter(|s| is_alive(s.pid)), path))
        .partition(|(status, _)| status.is_some());

    Ok(StatusScan {
        live: live.into_iter().filter_map(|(status, _)| status).collect(),
        stale: stale.into_iter().map(|(_, path)| path).collect(),
    })
}

fn read_status(path: &Path) -> Option<SupervisorStatus> {
    let text = std::fs::read_to_string(path).ok()?;
    let status: SupervisorStatus = serde_json::from_str(&text).ok()?;
    let named_pid = path.file_stem()?.to_str()?.parse::<u32>().ok()?;

    (named_pid == status.pid).then_some(status)
}

pub fn remove_stale(stale: &[PathBuf]) {
    for path in stale {
        let _ = std::fs::remove_file(path);
    }
}

pub fn process_is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill with signal 0 sends nothing; it only checks that the
    // process exists and may be signalled, and touches no memory.
    let result = unsafe { libc::kill(pid, 0) };

    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn render_statuses(supervisors: &[SupervisorStatus], now: u64) -> String {
    if supervisors.is_empty() {
        return String::from("No tunnels are running.\n");
    }

    let header = [
        "CLUSTER",
        "PROFILE",
        "PORT",
        "BASTION",
        "STATE",
        "UPTIME",
        "LAST DROP",
    ]
    .map(String::from)
    .to_vec();
    let rows: Vec<Vec<String>> = supervisors
        .iter()
        .flat_map(|supervisor| supervisor.tunnels.iter())
        .map(|tunnel| status_row(tunnel, now))
        .collect();
    let table = render_table(&[vec![header], rows].concat());
    let supervisor_lines: String = supervisors
        .iter()
        .map(|supervisor| {
            format!(
                "awsctl pid {} has supervised {} tunnel(s) for {}\n",
                supervisor.pid,
                supervisor.tunnels.len(),
                format_duration(now.saturating_sub(supervisor.started_at))
            )
        })
        .collect();

    format!("{}\n{}", table, supervisor_lines)
}

fn status_row(tunnel: &TunnelStatus, now: u64) -> Vec<String> {
    let state = match tunnel.state {
        TunnelState::Up => tunnel.state.label().to_string(),
        other => format!(
            "{} ({})",
            other.label(),
            format_duration(now.saturating_sub(tunnel.state_since))
        ),
    };
    let uptime = tunnel
        .up_since
        .map(|since| format_duration(now.saturating_sub(since)))
        .unwrap_or_else(|| String::from("-"));
    let last_drop = tunnel
        .last_drop
        .as_ref()
        .map(|drop| {
            format!(
                "{} ago: {}",
                format_duration(now.saturating_sub(drop.at)),
                truncate(&drop.reason, MAX_REASON_WIDTH)
            )
        })
        .unwrap_or_else(|| String::from("-"));

    vec![
        tunnel.cluster.clone(),
        tunnel.profile.clone(),
        tunnel.local_port.to_string(),
        tunnel.bastion.clone(),
        state,
        uptime,
        last_drop,
    ]
}

fn render_table(rows: &[Vec<String>]) -> String {
    let columns = rows.first().map(Vec::len).unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();

    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .zip(&widths)
                .map(|(cell, width)| format!("{:<width$}", cell, width = width))
                .collect();
            format!("{}\n", cells.join("  ").trim_end())
        })
        .collect()
}

fn truncate(text: &str, width: usize) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= width {
        single_line
    } else {
        let kept: String = single_line.chars().take(width - 1).collect();
        format!("{}…", kept)
    }
}

pub fn format_duration(seconds: u64) -> String {
    let (days, hours, minutes, secs) = (
        seconds / 86_400,
        seconds % 86_400 / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
    );
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{}s", secs),
        (0, 0, _) => format!("{}m{:02}s", minutes, secs),
        (0, _, _) => format!("{}h{:02}m", hours, minutes),
        _ => format!("{}d{:02}h", days, hours),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_duration, process_is_alive, render_statuses, scan_statuses, status_path,
        write_status, StatusFileGuard, SupervisorStatus, TunnelState, TunnelStatus,
    };
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("awsctl-status-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn tunnel(cluster: &str) -> TunnelStatus {
        TunnelStatus {
            cluster: String::from(cluster),
            profile: String::from("dev"),
            region: String::from("eu-west-2"),
            local_port: 21443,
            bastion: String::from("i-0123456789abcdef0"),
            state: TunnelState::Starting,
            state_since: 1_000,
            up_since: None,
            drops: 0,
            last_drop: None,
        }
    }

    fn supervisor(pid: u32) -> SupervisorStatus {
        SupervisorStatus {
            pid,
            started_at: 1_000,
            tunnels: vec![tunnel("example-eks")],
        }
    }

    #[test]
    fn counts_a_drop_only_when_the_tunnel_was_up() {
        let up = tunnel("example-eks").up(1_100);

        let dropped = up.dropped(TunnelState::Reconnecting, "idle timeout", 1_200);
        let failed_again = dropped.dropped(TunnelState::Reconnecting, "refused", 1_210);

        assert_eq!(dropped.drops, 1);
        assert_eq!(dropped.up_since, None);
        assert_eq!(failed_again.drops, 1);
        assert_eq!(failed_again.state_since, 1_200);
        assert_eq!(
            failed_again.last_drop.map(|drop| drop.reason),
            Some(String::from("refused"))
        );
    }

    #[test]
    fn writes_a_status_file_only_its_owner_can_read_and_reads_it_back() {
        let dir = temp_dir("write");
        let path = status_path(&dir, 4242);

        write_status(&path, &supervisor(4242)).expect("writes");
        let scan = scan_statuses(&dir, |_| true).expect("scans");

        let mode = std::fs::metadata(&path)
            .expect("exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(scan.live, [supervisor(4242)]);
        assert!(scan.stale.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_the_file_of_a_supervisor_that_died_as_stale() {
        let dir = temp_dir("stale");
        write_status(&status_path(&dir, 4242), &supervisor(4242)).expect("writes");
        write_status(&status_path(&dir, 4343), &supervisor(4343)).expect("writes");

        let scan = scan_statuses(&dir, |pid| pid == 4343).expect("scans");

        assert_eq!(scan.live, [supervisor(4343)]);
        assert_eq!(scan.stale, [status_path(&dir, 4242)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_an_unreadable_or_misnamed_file_as_stale() {
        let dir = temp_dir("garbage");
        std::fs::create_dir_all(&dir).expect("creates");
        std::fs::write(dir.join("4242.json"), "not json").expect("writes");
        write_status(&status_path(&dir, 4343), &supervisor(4444)).expect("writes");

        let scan = scan_statuses(&dir, |_| true).expect("scans");

        assert!(scan.live.is_empty());
        assert_eq!(scan.stale.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_a_missing_directory_as_empty() {
        let scan = scan_statuses(&temp_dir("missing"), |_| true).expect("scans");

        assert!(scan.live.is_empty() && scan.stale.is_empty());
    }

    #[test]
    fn removes_the_status_file_when_the_guard_drops() {
        let dir = temp_dir("guard");
        let path = status_path(&dir, 4242);
        write_status(&path, &supervisor(4242)).expect("writes");

        drop(StatusFileGuard::new(path.clone()));

        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sees_this_process_alive_and_an_absent_one_dead() {
        assert!(process_is_alive(std::process::id()));
        assert!(!process_is_alive(0));
        assert!(!process_is_alive(u32::MAX));
    }

    #[test]
    fn renders_each_tunnel_with_its_state_uptime_and_last_drop() {
        let up = tunnel("example-eks").up(1_000);
        let needs_login = TunnelStatus {
            cluster: String::from("example-k8s"),
            local_port: 22443,
            ..tunnel("example-k8s").up(1_000).dropped(
                TunnelState::NeedsLogin,
                "credentials for profile 'dev' expired",
                4_000,
            )
        };
        let status = SupervisorStatus {
            pid: 4242,
            started_at: 1_000,
            tunnels: vec![up, needs_login],
        };

        let rendered = render_statuses(&[status], 4_600);

        let lines: Vec<&str> = rendered.lines().collect();
        assert!(lines[0].starts_with("CLUSTER"));
        assert!(lines[1].contains("example-eks") && lines[1].contains("1h00m"));
        assert!(lines[2].contains("needs login (10m00s)"));
        assert!(lines[2].contains("10m00s ago: credentials for profile 'dev' expired"));
        assert!(rendered.contains("awsctl pid 4242 has supervised 2 tunnel(s) for 1h00m"));
    }

    #[test]
    fn says_so_when_nothing_runs() {
        assert_eq!(render_statuses(&[], 0), "No tunnels are running.\n");
    }

    #[test]
    fn formats_durations_compactly() {
        assert_eq!(format_duration(42), "42s");
        assert_eq!(format_duration(310), "5m10s");
        assert_eq!(format_duration(3_720), "1h02m");
        assert_eq!(format_duration(273_600), "3d04h");
    }
}
