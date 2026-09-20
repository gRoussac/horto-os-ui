use anyhow::Result;
use clap::Parser;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear as CtClear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand,
};
use horto_os_ui_shared::{
    backup_etc_timestamped, box_status, offer_save_api_token, pipeline, probe_disk_backup,
    probe_surfaces, remote_box_snapshot, remote_run_cli, remote_upload_cli, require_root_for_apply,
    setup_run, setup_step, ApplyMode, DiskBackupOpts, HostContext, RemoteBoxCliStatus,
    RemoteOptions, RemoteRunRequest, SetupKind, StdioPrompts, SurfaceProbeReport,
    SystemProcessRunner, GIT_COMMIT, LONG_VERSION, VERSION,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use std::io::{self, stdout, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};

mod tabs;
use tabs::Screen;

const READY: &str = "Ready (? help)";

/// Set by SIGINT/SIGTERM so the loop can restore the tty before exit.
static STOP: AtomicBool = AtomicBool::new(false);

/// Box CLI footer/overview value (remote mode).
#[derive(Debug, Clone, PartialEq, Eq)]
enum BoxCliView {
    /// Remote TUI just opened; no `r` / s0 probe yet (not the same as missing).
    NotProbed,
    /// Last probe result from shared remote layer.
    Known(RemoteBoxCliStatus),
}

impl BoxCliView {
    fn as_label(&self) -> &str {
        match self {
            Self::NotProbed => "…",
            Self::Known(s) => s.as_label(),
        }
    }
}

fn footer_cli_label(cli_local: &str, remote: bool, box_cli: &BoxCliView) -> String {
    match (remote, box_cli) {
        (true, BoxCliView::NotProbed) => format!("local={cli_local}"),
        (true, BoxCliView::Known(_)) => {
            format!("local={cli_local} box={}", box_cli.as_label())
        }
        (false, _) => format!("local={cli_local}"),
    }
}

fn footer_hints(app: &App) -> &'static str {
    if app.help_open {
        return "Esc or ? close help";
    }
    if app.confirm_destructive.is_some() {
        return "Enter/y confirm · Esc/n cancel · Ctrl+C quit";
    }
    match app.screen {
        Screen::Setup => {
            "j/k select · Enter run · a all · b backup · ←/→ pipeline · d dry-run/apply · r probe · ? help · q quit"
        }
        Screen::Logs => "c clear · Tab screens · r probe · B disk · ? help · q quit",
        Screen::Overview | Screen::Ssh | Screen::Cli | Screen::Api | Screen::Mcp => {
            "Enter action · r probe · Tab screens · ? help · q quit"
        }
        Screen::Reboot => "Enter reboot · r refresh · Tab · ? help · q quit",
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "horto-os-ui-tui",
    about = "Horto OS UI terminal",
    version,
    long_version = LONG_VERSION
)]
struct Cli {
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    minimal: bool,
    #[arg(long)]
    skip_piper: bool,
    /// OpenSSH Host alias or user@host; run setup via remote runner
    #[arg(long)]
    remote: Option<String>,
    /// Opt-in: install this PC's public key on the box. Off by default.
    #[arg(long, default_value_t = false)]
    install_ssh_key: bool,
    /// Local directory with box binaries (skips GitHub Release download)
    #[arg(long, env = "HORTO_BIN_DIR")]
    bin_dir: Option<std::path::PathBuf>,
    /// GitHub Release tag for box tar.gz (`v0.1.0` or tip `dev-preview`)
    #[arg(long, env = "HORTO_RELEASE_TAG")]
    release_tag: Option<String>,
}

struct App {
    screen: Screen,
    dry_run: bool,
    kind: SetupKind,
    skip_piper: bool,
    remote: Option<String>,
    install_ssh_key: bool,
    bin_dir: Option<std::path::PathBuf>,
    release_tag: Option<String>,
    step_state: ListState,
    logs: Vec<String>,
    status_lines: Vec<String>,
    overview_text: String,
    panel_text: String,
    confirm_destructive: Option<String>,
    help_open: bool,
    message: String,
    /// Local TUI / tip CLI long version (`LONG_VERSION`).
    cli_local: String,
    /// Last known box CLI status (remote). Starts as [`BoxCliView::NotProbed`].
    box_cli: BoxCliView,
    /// Box CLI matches `cli_local` (remote mode). Local mode always true.
    cli_current: bool,
    /// Last full surface probe (SSH/CLI/API/MCP).
    surfaces: Option<SurfaceProbeReport>,
    /// Extra Overview lines from doctor/status snapshot.
    overview_extra: String,
}

impl App {
    fn new(cli: &Cli) -> Self {
        let kind = if cli.minimal {
            SetupKind::Minimal
        } else {
            SetupKind::Full
        };
        let mut step_state = ListState::default();
        step_state.select(Some(0));
        let mut app = Self {
            screen: Screen::Setup,
            dry_run: cli.dry_run,
            kind,
            skip_piper: cli.skip_piper,
            remote: cli.remote.clone(),
            install_ssh_key: cli.install_ssh_key,
            bin_dir: cli.bin_dir.clone(),
            release_tag: cli.release_tag.clone(),
            step_state,
            logs: Vec::new(),
            status_lines: Vec::new(),
            overview_text: String::new(),
            panel_text: String::new(),
            confirm_destructive: None,
            help_open: false,
            message: READY.into(),
            cli_local: LONG_VERSION.to_owned(),
            box_cli: if cli.remote.is_some() {
                BoxCliView::NotProbed
            } else {
                BoxCliView::Known(RemoteBoxCliStatus::Found(LONG_VERSION.to_owned()))
            },
            cli_current: cli.remote.is_none(),
            surfaces: None,
            overview_extra: String::new(),
        };
        if app.remote.is_some() {
            app.rebuild_remote_steps(None);
            app.overview_text =
                tabs::panel_overview_remote(app.remote.as_deref().unwrap_or("?"), None, "");
            app.refresh_panel_text();
        } else {
            app.refresh();
        }
        app
    }

    fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    fn refresh_panel_text(&mut self) {
        let host = self.remote.as_deref().unwrap_or("local");
        self.panel_text = match self.screen {
            Screen::Ssh => tabs::panel_ssh(self.is_remote(), host, self.surfaces.as_ref()),
            Screen::Cli => tabs::panel_cli(self.is_remote(), self.surfaces.as_ref()),
            Screen::Api => tabs::panel_api(self.surfaces.as_ref()),
            Screen::Mcp => tabs::panel_mcp(self.surfaces.as_ref()),
            Screen::Reboot => tabs::panel_reboot(host),
            Screen::Overview => {
                if self.is_remote() {
                    tabs::panel_overview_remote(host, self.surfaces.as_ref(), &self.overview_extra)
                } else {
                    self.overview_text.clone()
                }
            }
            Screen::Setup | Screen::Logs => String::new(),
        };
    }

    fn s0_line(&self) -> String {
        let status = match &self.box_cli {
            BoxCliView::NotProbed => "…",
            BoxCliView::Known(RemoteBoxCliStatus::AuthFailed)
            | BoxCliView::Known(RemoteBoxCliStatus::Unreachable) => "blocked",
            _ if self.cli_current => "done",
            _ => "needed",
        };
        format!("s0 | {status} | Sync CLI to box")
    }

    fn rebuild_remote_steps(&mut self, setup: Option<&horto_os_ui_shared::SetupStatusReport>) {
        let mut lines = vec![self.s0_line()];
        if let Some(report) = setup {
            lines.extend(report.steps.iter().map(|s| {
                format!(
                    "{} | {} | {}{}",
                    s.id,
                    s.status,
                    s.title,
                    if s.destructive { " *" } else { "" }
                )
            }));
        } else {
            lines.extend(pipeline(self.kind).iter().map(|s| {
                format!(
                    "{} | … | {}{}",
                    s.id(),
                    s.title(),
                    if s.destructive() { " *" } else { "" }
                )
            }));
        }
        self.status_lines = lines;
        self.select_smart_step();
    }

    fn select_smart_step(&mut self) {
        if self.remote.is_some() && !self.cli_current {
            self.step_state.select(Some(0));
            return;
        }
        let start = if self.remote.is_some() { 1 } else { 0 };
        for (i, line) in self.status_lines.iter().enumerate().skip(start) {
            let status = line.split(" | ").nth(1).unwrap_or("");
            if status == "pending" || status == "…" {
                self.step_state.select(Some(i));
                return;
            }
        }
        if self.status_lines.len() > start {
            self.step_state.select(Some(start));
        } else {
            self.step_state.select(Some(0));
        }
    }

    fn set_pipeline_kind_local(&mut self, kind: SetupKind) {
        self.kind = kind;
        if self.remote.is_some() {
            // Local only: no SSH/SCP. Keep s0 state; reset setup rows to placeholders.
            self.rebuild_remote_steps(None);
            self.message = format!("pipeline={} (press r for status)", kind.as_str());
        } else {
            self.refresh_local();
            self.message = format!("pipeline={}", kind.as_str());
        }
    }

    fn remote_opts(&self) -> Option<RemoteOptions> {
        self.remote.as_ref().map(|host| {
            let mut opts = RemoteOptions {
                host: host.clone(),
                install_ssh_key: self.install_ssh_key,
                bin_dir: self.bin_dir.clone(),
                ..RemoteOptions::default()
            };
            if let Some(tag) = self
                .release_tag
                .as_ref()
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
            {
                opts.release_tag = tag.to_owned();
            }
            opts
        })
    }

    fn refresh(&mut self) {
        if self.remote.is_some() {
            self.refresh_remote();
        } else {
            self.refresh_local();
        }
    }

    fn refresh_remote(&mut self) {
        let Some(opts) = self.remote_opts() else {
            return;
        };
        let host = opts.host.clone();
        self.message = format!("Probing {host}…");
        match probe_surfaces(&SystemProcessRunner, &opts, false) {
            Ok(report) => {
                self.box_cli = BoxCliView::Known(report.cli.status.clone());
                self.cli_current = report.cli.current;
                self.surfaces = Some(report);
                self.push_log(format!(
                    "probe ssh={} cli={} api={}",
                    self.surfaces.as_ref().unwrap().ssh.status,
                    self.box_cli.as_label(),
                    self.surfaces.as_ref().unwrap().api.health
                ));
            }
            Err(e) => {
                self.box_cli = BoxCliView::Known(RemoteBoxCliStatus::Unreachable);
                self.cli_current = false;
                self.surfaces = None;
                self.push_log(format!("ERROR probe: {e}"));
                self.message = format!("Probe failed: {e}");
                self.rebuild_remote_steps(None);
                self.refresh_panel_text();
                return;
            }
        }

        let full = self.kind != SetupKind::Minimal;
        self.overview_extra.clear();
        if self.cli_current {
            match remote_box_snapshot(&SystemProcessRunner, opts, full) {
                Ok(snap) => {
                    self.rebuild_remote_steps(snap.setup.as_ref());
                    if let Some(doc) = &snap.doctor {
                        self.overview_extra.push_str(&format!(
                            "doctor: root={} sudo={} docker={} full_env={} minimal_env={}\n",
                            doc.is_root,
                            doc.has_sudo,
                            doc.docker_present,
                            doc.full_env,
                            doc.minimal_env
                        ));
                    }
                    self.push_log(format!(
                        "setup steps={}",
                        snap.setup.as_ref().map(|s| s.steps.len()).unwrap_or(0)
                    ));
                }
                Err(e) => {
                    self.rebuild_remote_steps(None);
                    self.push_log(format!("ERROR setup status: {e}"));
                }
            }
        } else {
            self.rebuild_remote_steps(None);
        }

        self.overview_text =
            tabs::panel_overview_remote(&host, self.surfaces.as_ref(), &self.overview_extra);
        self.refresh_panel_text();
        self.message = format!("Probed {host}");
    }

    fn run_surface_enter(&mut self) {
        match self.screen {
            Screen::Ssh => {
                if !self.install_ssh_key {
                    self.message = "Start with --install-ssh-key to enable key install".into();
                    return;
                }
                let Some(opts) = self.remote_opts() else {
                    return;
                };
                self.push_log("SSH: installing key…");
                match horto_os_ui_shared::remote_ensure_ssh_key(&SystemProcessRunner, &opts) {
                    Ok(()) => {
                        self.message = "SSH key installed (or already authorized)".into();
                        self.refresh_remote();
                    }
                    Err(e) => {
                        self.push_log(format!("ERROR ssh key: {e}"));
                        self.message = format!("SSH key failed: {e}");
                    }
                }
            }
            Screen::Cli => self.run_s0_sync(),
            Screen::Api | Screen::Mcp | Screen::Overview => self.refresh(),
            Screen::Reboot => self.run_reboot(),
            Screen::Setup | Screen::Logs => {}
        }
    }

    fn run_reboot(&mut self) {
        if self.confirm_destructive.as_deref() != Some("reboot") {
            self.confirm_destructive = Some("reboot".into());
            self.message = "Reboot box? Enter/y confirm, Esc/n cancel.".into();
            return;
        }
        self.confirm_destructive = None;
        let Some(opts) = self.remote_opts() else {
            return;
        };
        self.push_log("reboot: sudo reboot on box…");
        match horto_os_ui_shared::remote_reboot(&SystemProcessRunner, &opts) {
            Ok(()) => self.message = "Reboot issued".into(),
            Err(e) => {
                self.push_log(format!("ERROR reboot: {e}"));
                self.message = format!("Reboot failed: {e}");
            }
        }
    }

    fn run_s0_sync(&mut self) {
        let Some(opts) = self.remote_opts() else {
            return;
        };
        self.push_log("s0: syncing CLI to box…");
        match remote_upload_cli(&SystemProcessRunner, &opts) {
            Ok(probe) => {
                self.box_cli = BoxCliView::Known(probe.status.clone());
                self.cli_current = probe.current;
                self.push_log(format!("s0: box={}", probe.status.as_label()));
                if probe.current {
                    self.refresh_remote();
                    self.message = "s0 done; CLI synced".into();
                } else {
                    self.rebuild_remote_steps(None);
                    self.message = format!(
                        "s0 finished but box still {}; check SSH/auth",
                        probe.status.as_label()
                    );
                }
            }
            Err(e) => {
                self.push_log(format!("ERROR s0: {e}"));
                self.message = format!("s0 failed: {e}");
            }
        }
    }

    fn refresh_local(&mut self) {
        let ctx = self.make_ctx();
        let report = horto_os_ui_shared::setup_status(&ctx, self.kind);
        self.status_lines = report
            .steps
            .iter()
            .map(|s| {
                format!(
                    "{} | {} | {}{}",
                    s.id,
                    s.status,
                    s.title,
                    if s.destructive { " *" } else { "" }
                )
            })
            .collect();
        let box_st = box_status(&ctx, self.kind);
        let mut overview = String::new();
        overview.push_str("Mode: embedded\n");
        overview.push_str(&format!("Hostname: {}\n", box_st.hostname));
        overview.push_str(&format!(
            "Root: {}  Docker: {}  Full env: {}  Minimal env: {}\n",
            box_st.doctor.is_root,
            box_st.doctor.docker_present,
            box_st.doctor.full_env,
            box_st.doctor.minimal_env
        ));
        for n in &box_st.doctor.notes {
            overview.push_str(&format!("- {n}\n"));
        }
        overview.push_str("\nContainers:\n");
        if box_st.containers.is_empty() {
            overview.push_str("  (none)\n");
        } else {
            for c in &box_st.containers {
                overview.push_str(&format!("  {} {}\n", c.names, c.status));
            }
        }
        overview.push_str("\nURLs:\n");
        for u in &box_st.urls {
            let mark = if u.up { "up" } else { "down" };
            overview.push_str(&format!("  {} [{}]: {}\n", u.name, mark, u.url));
        }
        overview.push_str(&format!("\nLeases: {}\n", box_st.leases.len()));
        for l in box_st.leases.iter().take(12) {
            overview.push_str(&format!("  {} {}\n", l.hostname, l.ip));
        }
        overview.push_str("\nBackup:\n");
        overview.push_str(&format!(
            "  initial_setup: {}\n",
            box_st.backup.initial_setup_present
        ));
        if box_st.backup.timestamped.is_empty() {
            overview.push_str("  timestamped: (none)\n");
        } else {
            let recent: Vec<_> = box_st.backup.timestamped.iter().rev().take(5).collect();
            overview.push_str(&format!(
                "  timestamped ({}): {}\n",
                box_st.backup.timestamped.len(),
                recent
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        overview.push_str(&format!(
            "  disk root={} safe={} blockers={}\n",
            box_st.backup.disk.root_source,
            box_st.backup.disk.safe_to_apply,
            box_st.backup.disk.blockers.len()
        ));
        self.overview_text = overview;
        let opts = RemoteOptions::default();
        match probe_surfaces(&SystemProcessRunner, &opts, true) {
            Ok(report) => {
                self.surfaces = Some(report);
            }
            Err(e) => {
                self.push_log(format!("ERROR local probe: {e}"));
            }
        }
        self.refresh_panel_text();
    }

    fn run_remote_cli(&mut self, rest: &[&str], use_sudo: bool, install_payload: bool) {
        let Some(options) = self.remote_opts() else {
            return;
        };
        let mut cli_args = Vec::new();
        if self.dry_run {
            cli_args.push("--dry-run".into());
        }
        if self.skip_piper {
            cli_args.push("--skip-piper".into());
        }
        for a in rest {
            cli_args.push((*a).to_owned());
        }
        match remote_run_cli(
            &SystemProcessRunner,
            &RemoteRunRequest {
                options,
                cli_args,
                use_sudo,
                install_payload_on_success: install_payload,
                offer_reboot_on_success: install_payload && !self.dry_run,
                capture_output: false,
            },
        ) {
            Ok(outcome) => {
                for line in outcome.log.lines() {
                    self.push_log(line.to_owned());
                }
                if let Some(token) = outcome.api_token.as_deref() {
                    match offer_save_api_token(token) {
                        Ok(true) => {
                            self.push_log(
                                "Saved status-api bearer to ~/.config/horto-os-ui/api_token"
                                    .to_owned(),
                            );
                            self.message = "Remote finished; API token saved".into();
                        }
                        Ok(false) => {
                            self.push_log("Skipped saving status-api bearer locally");
                            self.message = "Remote command finished".into();
                        }
                        Err(e) => {
                            self.push_log(format!("Token save failed: {e}"));
                            self.message = "Remote finished; token save failed".into();
                        }
                    }
                } else {
                    self.message = "Remote command finished".into();
                }
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Remote failed: {e}");
            }
        }
    }

    fn make_ctx(&self) -> HostContext {
        let mode = if self.dry_run {
            ApplyMode::DryRun
        } else {
            ApplyMode::Apply
        };
        let mut ctx = HostContext::new(mode, self.kind).with_prompts(Box::new(StdioPrompts));
        ctx.skip_piper = self.skip_piper;
        ctx
    }

    fn selected_step_id(&self) -> Option<String> {
        let idx = self.step_state.selected()?;
        self.status_lines
            .get(idx)
            .and_then(|l| l.split(" | ").next())
            .map(str::to_string)
    }

    fn push_log(&mut self, line: impl Into<String>) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.logs.push(format!("{ts} {}", line.into()));
        if self.logs.len() > 500 {
            self.logs.drain(0..self.logs.len() - 500);
        }
    }

    fn clear_logs(&mut self) {
        self.logs.clear();
        self.message = "Logs cleared".into();
    }

    fn next_screen(&mut self) {
        self.screen = self.screen.next(self.is_remote());
        self.refresh_panel_text();
    }

    fn prev_screen(&mut self) {
        self.screen = self.screen.prev(self.is_remote());
        self.refresh_panel_text();
    }

    fn select_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.help_open = false;
        self.refresh_panel_text();
    }

    fn run_selected(&mut self) {
        let Some(id) = self.selected_step_id() else {
            return;
        };
        if id == "s0" {
            self.run_s0_sync();
            return;
        }
        if self.remote.is_some() && !self.cli_current {
            self.message = match &self.box_cli {
                BoxCliView::NotProbed => "Press r to probe box CLI (or run s0 to sync)".into(),
                BoxCliView::Known(RemoteBoxCliStatus::AuthFailed) => {
                    "SSH auth failed: install key (--install-ssh-key) or run s0 after login".into()
                }
                BoxCliView::Known(RemoteBoxCliStatus::Unreachable) => {
                    "Box unreachable: check host/network, then press r".into()
                }
                _ => "Run s0 (Sync CLI) before other steps".into(),
            };
            return;
        }
        let ctx_probe = self.make_ctx();
        let report = horto_os_ui_shared::setup_status(&ctx_probe, self.kind);
        if let Some(row) = report.steps.iter().find(|s| s.id == id) {
            if row.destructive && !self.dry_run && self.confirm_destructive.is_none() {
                self.confirm_destructive = Some(id.clone());
                self.message = format!("Step {id} is destructive. Enter/y confirm, Esc/n cancel.");
                return;
            }
        }
        self.confirm_destructive = None;
        self.execute_step(&id);
    }

    fn execute_step(&mut self, id: &str) {
        self.push_log(format!("Running step {id} (dry_run={})", self.dry_run));
        if self.remote.is_some() {
            let kind = if self.kind == SetupKind::Minimal {
                "--minimal"
            } else {
                "--full"
            };
            self.run_remote_cli(&["setup", "step", id, kind], !self.dry_run, false);
            self.refresh();
            return;
        }
        let mut ctx = self.make_ctx();
        match setup_step(&mut ctx, self.kind, id) {
            Ok(()) => {
                for l in &ctx.logs {
                    self.push_log(l.clone());
                }
                self.message = format!("Step {id} finished");
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Step {id} failed: {e}");
            }
        }
        self.refresh();
    }

    fn run_all(&mut self) {
        if self.remote.is_some() && !self.cli_current {
            self.message = match &self.box_cli {
                BoxCliView::NotProbed => "Press r to probe box CLI (or run s0 to sync)".into(),
                _ => "Run s0 (Sync CLI) before running all steps".into(),
            };
            return;
        }
        self.push_log(format!(
            "Running all pipeline steps (dry_run={})",
            self.dry_run
        ));
        if self.remote.is_some() {
            let kind = if self.kind == SetupKind::Minimal {
                "--minimal"
            } else {
                "--full"
            };
            self.run_remote_cli(&["setup", "run", kind], !self.dry_run, !self.dry_run);
            self.refresh();
            return;
        }
        let mut ctx = self.make_ctx();
        match setup_run(&mut ctx, self.kind) {
            Ok(()) => {
                for l in &ctx.logs {
                    self.push_log(l.clone());
                }
                self.message = "Pipeline finished".into();
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Pipeline failed: {e}");
            }
        }
        self.refresh();
    }

    fn run_backup_etc(&mut self) {
        self.push_log(format!(
            "Timestamped /etc backup (dry_run={})",
            self.dry_run
        ));
        let mut ctx = self.make_ctx();
        if let Err(e) = require_root_for_apply(ctx.mode) {
            self.push_log(format!("ERROR: {e}"));
            self.message = format!("Backup etc failed: {e}");
            return;
        }
        match backup_etc_timestamped(&mut ctx) {
            Ok(report) => {
                for l in &ctx.logs {
                    self.push_log(l.clone());
                }
                self.message = format!("Backup etc -> {}", report.dest);
            }
            Err(e) => {
                self.push_log(format!("ERROR: {e}"));
                self.message = format!("Backup etc failed: {e}");
            }
        }
        self.refresh();
    }

    fn show_disk_backup_status(&mut self) {
        let probe = probe_disk_backup(&DiskBackupOpts::default());
        self.push_log(format!(
            "disk backup: root={} safe={} partclone={}",
            probe.root_source, probe.safe_to_apply, probe.partclone_present
        ));
        for b in &probe.blockers {
            self.push_log(format!("blocker: {b}"));
        }
        self.message = if probe.safe_to_apply {
            "Disk backup looks safe (CLI: horto backup disk)".into()
        } else {
            format!(
                "Disk backup blocked ({}). See Logs. Boot from SD for eMMC image.",
                probe.blockers.len()
            )
        };
        self.screen = Screen::Logs;
        self.refresh_panel_text();
    }
}

fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn install_signal_handlers() {
    let _ = ctrlc::set_handler(|| {
        STOP.store(true, Ordering::SeqCst);
    });
}

fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

/// Restores the terminal on drop (normal exit, `?`, or unwind).
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<(Self, Terminal<CrosstermBackend<io::Stdout>>)> {
        enable_raw_mode()?;
        execute!(
            stdout(),
            EnterAlternateScreen,
            Hide,
            CtClear(ClearType::All),
            CtClear(ClearType::Purge)
        )?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;
        Ok((Self, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Reset SGR, show cursor, leave alt buffer, clear leftover TUI cells.
fn hard_reset_tty() {
    let mut out = stdout();
    let _ = write!(
        out,
        "\x1b[0m\x1b[?25h\x1b[?1049l\x1b[?47l\x1b[2J\x1b[3J\x1b[H"
    );
    let _ = out.execute(CtClear(ClearType::All));
    let _ = out.execute(CtClear(ClearType::Purge));
    let _ = out.execute(Show);
    let _ = out.flush();
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = stdout();
    let _ = out.execute(LeaveAlternateScreen);
    let _ = out.flush();
    hard_reset_tty();
}

/// Leave the ratatui alt screen so SSH/sudo/prompts own the real TTY, then restore.
fn with_suspended_tui<R>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    f: impl FnOnce() -> R,
) -> io::Result<R> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen, Show)?;
    let _ = stdout().flush();
    let out = f();
    enable_raw_mode()?;
    execute!(
        stdout(),
        EnterAlternateScreen,
        Hide,
        CtClear(ClearType::All),
        CtClear(ClearType::Purge)
    )?;
    terminal.clear()?;
    drain_pending_keys();
    Ok(out)
}

/// Drop key/escape bytes queued while the TUI was suspended (avoids `CCCC` bleed).
fn drain_pending_keys() {
    while event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    install_signal_handlers();
    install_panic_hook();
    let (_guard, mut terminal) = TerminalGuard::enter()?;
    let mut app = App::new(&cli);
    // Remote: BatchMode probe at open (no alt-screen suspend; no password UI).
    if app.is_remote() {
        app.refresh();
    }
    run_app(&mut terminal, &mut app)
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        if STOP.load(Ordering::SeqCst) {
            return Ok(());
        }
        terminal.draw(|f| ui(f, app))?;
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if is_quit(key) {
            return Ok(());
        }
        if app.help_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    app.help_open = false;
                    app.message = READY.into();
                }
                _ => {}
            }
            continue;
        }
        if app.confirm_destructive.is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if app.confirm_destructive.as_deref() == Some("reboot") {
                        with_suspended_tui(terminal, || app.run_reboot())?;
                    } else {
                        with_suspended_tui(terminal, || app.run_selected())?;
                    }
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    app.confirm_destructive = None;
                    app.message = "Cancelled".into();
                }
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(()),
            KeyCode::Char('?') => {
                app.help_open = true;
                app.message = "Help".into();
            }
            KeyCode::Tab => app.next_screen(),
            KeyCode::BackTab => app.prev_screen(),
            KeyCode::Char(d) if d.is_ascii_digit() => {
                if let Some(screen) = Screen::from_digit(d, app.is_remote()) {
                    app.select_screen(screen);
                }
            }
            KeyCode::Char('c') if app.screen == Screen::Logs => app.clear_logs(),
            KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.dry_run = !app.dry_run;
                app.message = format!("dry-run = {}", app.dry_run);
                if app.remote.is_none() {
                    app.refresh();
                }
            }
            KeyCode::Char('r') => {
                // Surface probe is BatchMode + Capture: stay in the TUI.
                app.refresh();
                if app.remote.is_none() {
                    app.message = "Refreshed".into();
                }
            }
            KeyCode::Char('a') => {
                with_suspended_tui(terminal, || app.run_all())?;
            }
            KeyCode::Char('b') => {
                with_suspended_tui(terminal, || app.run_backup_etc())?;
            }
            KeyCode::Char('B') => app.show_disk_backup_status(),
            KeyCode::Enter if app.screen == Screen::Setup => {
                // Destructive apply: first Enter only arms the confirm dialog (stay in TUI).
                let ask_confirm = !app.dry_run
                    && app
                        .step_state
                        .selected()
                        .and_then(|i| app.status_lines.get(i))
                        .is_some_and(|line| line.contains(" *"));
                if ask_confirm {
                    app.run_selected();
                } else {
                    with_suspended_tui(terminal, || app.run_selected())?;
                }
            }
            KeyCode::Enter
                if matches!(
                    app.screen,
                    Screen::Ssh
                        | Screen::Cli
                        | Screen::Api
                        | Screen::Mcp
                        | Screen::Overview
                        | Screen::Reboot
                ) =>
            {
                match app.screen {
                    Screen::Reboot => app.run_reboot(),
                    Screen::Ssh | Screen::Cli => {
                        with_suspended_tui(terminal, || app.run_surface_enter())?;
                    }
                    Screen::Api | Screen::Mcp | Screen::Overview => {
                        app.run_surface_enter();
                    }
                    Screen::Setup | Screen::Logs => {}
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(i) = app.step_state.selected() {
                    if i > 0 {
                        app.step_state.select(Some(i - 1));
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = app.status_lines.len();
                if let Some(i) = app.step_state.selected() {
                    if i + 1 < len {
                        app.step_state.select(Some(i + 1));
                    }
                }
            }
            KeyCode::Left => {
                app.set_pipeline_kind_local(SetupKind::Full);
            }
            KeyCode::Right => {
                app.set_pipeline_kind_local(SetupKind::Minimal);
            }
            _ => {}
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(4),
        ])
        .split(f.area());

    let remote = app.is_remote();
    let titles = Screen::titles(remote)
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    let idx = app.screen.index(remote);
    let tabs = Tabs::new(titles)
        .select(idx)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("horto-tui/{VERSION}/{GIT_COMMIT}")),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[0]);

    match app.screen {
        Screen::Setup => draw_setup(f, app, chunks[1]),
        Screen::Logs => draw_logs(f, app, chunks[1]),
        Screen::Overview
        | Screen::Ssh
        | Screen::Cli
        | Screen::Api
        | Screen::Mcp
        | Screen::Reboot => draw_panel(f, app, chunks[1]),
    }

    let mode = if app.dry_run { "DRY-RUN" } else { "APPLY" };
    let status = if app.message.is_empty() {
        READY.to_string()
    } else {
        app.message.clone()
    };
    let cli_label = footer_cli_label(&app.cli_local, remote, &app.box_cli);
    let footer = Paragraph::new(vec![
        Line::from(format!(
            "[{mode}] pipeline={} · {cli_label} · {status}",
            app.kind.as_str()
        )),
        Line::from(Span::styled(
            footer_hints(app),
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(footer, chunks[2]);

    if app.help_open {
        draw_help(f);
    }
}

fn draw_help(f: &mut Frame) {
    let area = centered_rect(70, 80, f.area());
    f.render_widget(Clear, area);
    let body = [
        "Horto TUI help",
        "",
        "q / Esc / Ctrl+C   Quit",
        "?                  Toggle this help",
        "Tab / Shift-Tab    Next / previous tab",
        "1-8                Jump to tab (remote: 7 Reboot, 8 Logs)",
        "j k / arrows       Move step selection (Setup)",
        "Left / Right       Full / Minimal pipeline",
        "Enter              Setup: run step · surface tabs: action",
        "a                  Run all pipeline steps",
        "b / B              Timestamped /etc backup / disk probe",
        "r                  Re-probe surfaces (stay in TUI)",
        "c                  Clear Logs (on Logs tab)",
        "d                  Toggle dry-run / apply",
        "y / n              Confirm / cancel",
        "",
        "Remote open probes SSH/CLI/API/MCP (BatchMode).",
        "Mouse capture is off so you can select and copy text.",
        "Press Esc or ? to close.",
    ]
    .join("\n");
    let p = Paragraph::new(body).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Help")
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(p, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
}

fn draw_setup(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .status_lines
        .iter()
        .map(|l| {
            let style = if l.contains("| done |") {
                Style::default().fg(Color::Green)
            } else if l.contains("| stale |") {
                Style::default().fg(Color::Magenta)
            } else if l.contains("| failed |") {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(Line::from(Span::styled(l.clone(), style)))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Steps (Enter run · a = all · * = destructive)"),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, area, &mut app.step_state);
}

fn draw_logs(f: &mut Frame, app: &App, area: Rect) {
    let n = app.logs.len();
    let text = if app.logs.is_empty() {
        "(empty · c to clear)".to_string()
    } else {
        app.logs
            .iter()
            .rev()
            .take(40)
            .cloned()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    };
    let p = Paragraph::new(text).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Logs ({n})")),
    );
    f.render_widget(p, area);
}

fn draw_panel(f: &mut Frame, app: &App, area: Rect) {
    let title = match app.screen {
        Screen::Overview => "Overview",
        Screen::Ssh => "SSH",
        Screen::Cli => "CLI",
        Screen::Api => "API",
        Screen::Mcp => "MCP",
        Screen::Reboot => "Reboot",
        Screen::Setup | Screen::Logs => "",
    };
    let body = if app.screen == Screen::Overview && !app.is_remote() {
        app.overview_text.clone()
    } else {
        app.panel_text.clone()
    };
    let p = Paragraph::new(body)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::Parser;

    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_flags() {
        let cli =
            Cli::try_parse_from(["horto-os-ui-tui", "--dry-run", "--minimal", "--skip-piper"])
                .unwrap();
        assert!(cli.dry_run);
        assert!(cli.minimal);
        assert!(cli.skip_piper);
    }

    #[test]
    fn quit_keys() {
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let plain_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(is_quit(q));
        assert!(is_quit(ctrl_c));
        assert!(!is_quit(plain_c));
    }

    #[test]
    fn footer_cli_label_not_probed_omits_box() {
        let label = footer_cli_label("0.1.0 (abc)", true, &BoxCliView::NotProbed);
        assert_eq!(label, "local=0.1.0 (abc)");
        assert!(!label.contains("box="));
        assert!(!label.contains("missing"));
    }

    #[test]
    fn footer_cli_label_known_statuses() {
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::Missing)
            ),
            "local=0.1.0 box=missing"
        );
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::AuthFailed)
            ),
            "local=0.1.0 box=auth failed"
        );
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::Unreachable)
            ),
            "local=0.1.0 box=unreachable"
        );
        assert_eq!(
            footer_cli_label(
                "0.1.0",
                true,
                &BoxCliView::Known(RemoteBoxCliStatus::Found("0.1.0 (deadbeef)".into()))
            ),
            "local=0.1.0 box=0.1.0 (deadbeef)"
        );
        assert_eq!(
            footer_cli_label("0.1.0", false, &BoxCliView::NotProbed),
            "local=0.1.0"
        );
    }

    #[test]
    fn remote_app_starts_with_box_not_probed() {
        let cli = Cli::try_parse_from(["horto-os-ui-tui", "--remote", "horto"]).unwrap();
        let app = App::new(&cli);
        assert_eq!(app.box_cli, BoxCliView::NotProbed);
        assert!(app.s0_line().contains("| … |"));
        assert!(!app.cli_current);
    }
}
