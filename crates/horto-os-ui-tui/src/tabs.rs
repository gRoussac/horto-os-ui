//! TUI tab order and panel text for surface probes.

use horto_os_ui_shared::{SurfaceProbeReport, LONG_VERSION};

/// Visible TUI screens (Reboot only when remote).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Setup,
    Overview,
    Ssh,
    Cli,
    Api,
    Mcp,
    Reboot,
    Logs,
}

impl Screen {
    /// Tab titles for the current mode (digits match order).
    #[must_use]
    pub fn titles(remote: bool) -> Vec<&'static str> {
        if remote {
            vec![
                "1 Setup",
                "2 Overview",
                "3 SSH",
                "4 CLI",
                "5 API",
                "6 MCP",
                "7 Reboot",
                "8 Logs",
            ]
        } else {
            vec![
                "1 Setup",
                "2 Overview",
                "3 SSH",
                "4 CLI",
                "5 API",
                "6 MCP",
                "7 Logs",
            ]
        }
    }

    #[must_use]
    pub fn all(remote: bool) -> Vec<Self> {
        if remote {
            vec![
                Self::Setup,
                Self::Overview,
                Self::Ssh,
                Self::Cli,
                Self::Api,
                Self::Mcp,
                Self::Reboot,
                Self::Logs,
            ]
        } else {
            vec![
                Self::Setup,
                Self::Overview,
                Self::Ssh,
                Self::Cli,
                Self::Api,
                Self::Mcp,
                Self::Logs,
            ]
        }
    }

    #[must_use]
    pub fn index(self, remote: bool) -> usize {
        Self::all(remote)
            .iter()
            .position(|s| *s == self)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn from_digit(d: char, remote: bool) -> Option<Self> {
        let n = d.to_digit(10)? as usize;
        if n == 0 {
            return None;
        }
        Self::all(remote).into_iter().nth(n - 1)
    }

    #[must_use]
    pub fn next(self, remote: bool) -> Self {
        let all = Self::all(remote);
        let i = self.index(remote);
        all[(i + 1) % all.len()]
    }

    #[must_use]
    pub fn prev(self, remote: bool) -> Self {
        let all = Self::all(remote);
        let i = self.index(remote);
        all[(i + all.len() - 1) % all.len()]
    }
}

/// Format SSH panel from a probe (or empty state).
#[must_use]
pub fn panel_ssh(remote: bool, host: &str, report: Option<&SurfaceProbeReport>) -> String {
    let mut out = String::new();
    if !remote {
        out.push_str("Mode: embedded (on box)\nSSH: n/a\n");
        return out;
    }
    out.push_str(&format!("Host: {host}\n"));
    match report {
        None => out.push_str("Status: ?\nPress r to probe.\n"),
        Some(r) => {
            out.push_str(&format!("Status: {}\n", r.ssh.status));
            out.push_str(&format!("Key BatchMode: {}\n", r.ssh.key_ok));
            out.push_str("Enter: install key when started with --install-ssh-key\n");
        }
    }
    out
}

/// Format CLI panel.
#[must_use]
pub fn panel_cli(remote: bool, report: Option<&SurfaceProbeReport>) -> String {
    let mut out = String::new();
    out.push_str(&format!("local={}\n", LONG_VERSION));
    if !remote {
        out.push_str("box=local (embedded)\n");
        return out;
    }
    match report {
        None => out.push_str("box=?\nPress r to probe.\n"),
        Some(r) => {
            out.push_str(&format!("box={}\n", r.cli.status.as_label()));
            out.push_str(&format!("current={}\n", r.cli.current));
            out.push_str("Enter: sync CLI to box (s0)\n");
        }
    }
    out
}

/// Format API panel.
#[must_use]
pub fn panel_api(report: Option<&SurfaceProbeReport>) -> String {
    match report {
        None => "Status API\nPress r to probe.\n".into(),
        Some(r) => format!(
            "URL: {}\nhealth={}\n/v1/status={}\nlocal_token_file={}\nunit={}\nEnter: re-probe\n",
            r.api.url,
            r.api.health,
            r.api.status,
            r.api.local_token,
            if r.api.unit.is_empty() {
                "-"
            } else {
                r.api.unit.as_str()
            }
        ),
    }
}

/// Format MCP panel (PC + box).
#[must_use]
pub fn panel_mcp(report: Option<&SurfaceProbeReport>) -> String {
    match report {
        None => "MCP\nPress r to probe.\n".into(),
        Some(r) => format!(
            "PC (stdio)\n  binary={}\n  api_health={}\n\nBox (:8790)\n  url={}\n  reach={}\n  unit={}\nEnter: re-probe\n",
            r.mcp_pc.binary.as_deref().unwrap_or("missing"),
            r.mcp_pc.api_health,
            r.mcp_box.url,
            r.mcp_box.reachability,
            if r.mcp_box.unit.is_empty() {
                "-"
            } else {
                r.mcp_box.unit.as_str()
            }
        ),
    }
}

/// Format Reboot panel.
#[must_use]
pub fn panel_reboot(host: &str) -> String {
    format!(
        "Reboot box '{host}' via SSH + sudo.\nEnter: confirm reboot (suspends TUI for password).\n"
    )
}

/// Overview facts from probe + optional doctor lines.
#[must_use]
pub fn panel_overview_remote(
    host: &str,
    report: Option<&SurfaceProbeReport>,
    extra: &str,
) -> String {
    let mut out = format!("Mode: remote ({host})\n");
    match report {
        None => out.push_str("No status yet.\n"),
        Some(r) => {
            out.push_str(&format!(
                "local={}  box={}\nssh={}  api.health={}\nmcp_box={}\n",
                r.local_version,
                r.cli.status.as_label(),
                r.ssh.status,
                r.api.health,
                r.mcp_box.reachability
            ));
        }
    }
    if !extra.is_empty() {
        out.push('\n');
        out.push_str(extra);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_tab_order_ends_with_logs() {
        let t = Screen::titles(true);
        assert_eq!(t.last().copied(), Some("8 Logs"));
        assert!(t.iter().any(|x| x.contains("Reboot")));
        assert_eq!(Screen::from_digit('8', true), Some(Screen::Logs));
        assert_eq!(Screen::from_digit('7', false), Some(Screen::Logs));
        assert_eq!(Screen::Setup.next(true), Screen::Overview);
        assert_eq!(Screen::Logs.prev(true), Screen::Reboot);
    }

    #[test]
    fn panels_not_probed_use_question() {
        assert!(panel_cli(true, None).contains("box=?"));
        assert!(!panel_cli(true, None).contains("missing"));
        assert!(panel_ssh(true, "horto", None).contains("Status: ?"));
        assert!(panel_overview_remote("horto", None, "").contains("No status yet."));
        assert!(!panel_overview_remote("horto", None, "").contains("Press r"));
    }
}
