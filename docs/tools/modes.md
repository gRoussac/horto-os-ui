# Modes and surfaces

Two modes describe **where you sit**, not two kinds of CLI on the box.

| Mode         | You sit    | You open             | Who applies setup                                                                     |
| ------------ | ---------- | -------------------- | ------------------------------------------------------------------------------------- |
| **Embedded** | On the box | CLI or TUI           | Same process, in-process shared engine                                                |
| **Remote**   | On a PC    | CLI, TUI, or Desktop | Shared remote runner SSHs in and runs the **CLI binary on the box** as an apply agent |

**Desktop is never embedded.** Always on a PC. Always remote (SSH for first install; HTTP day-2 once the status API is up).

Day-2 HTTP: Desktop (and KPI) use `GET /v1/status`. The only mutate route is
`POST /v1/backup/etc` (bearer token + `X-Horto-Confirm: backup-etc`). Reinstall
and setup stay on SSH / embedded CLI/TUI. See [status-api.md](status-api.md).

MCP (`horto-os-ui-mcp`) is the AI adapter over the same surfaces: Cursor on a PC
uses Docker stdio (status-api HTTP + OpenSSH); an on-box model uses Streamable
HTTP on LAN `:8790` with a bearer token. One tool catalog; backends switch by
`HORTO_MCP_MODE`. See [mcp.md](mcp.md).

There is **one** CLI binary. Same `horto-os-ui setup …` commands.

- **Embedded:** you type those commands (or TUI does) while logged into the box.
- **Remote:** the runner uploads that binary once, then `ssh … sudo horto-os-ui setup …`.

## Surfaces

| Surface               | Embedded   | Remote           | Role                        |
| --------------------- | ---------- | ---------------- | --------------------------- |
| CLI                   | yes        | yes (`--remote`) | scripts / CI                |
| TUI                   | yes        | yes (`--remote`) | power users                 |
| Desktop (Tauri + web) | **no**     | **always**       | home users                  |
| Status API            | on the box | HTTP day-2       | LAN clients / Desktop / KPI |
| MCP                   | box HTTP   | PC Docker stdio  | AI tools (Cursor / on-box)  |

CLI `surfaces` / `--json`, TUI surface tabs, and Desktop Connection **Probe surfaces** share one probe report (SSH, CLI, API, MCP PC + box). Remote TUI probes at open (BatchMode); footer shows `local=` then `box=` after that probe.

## Auth (OpenSSH only)

Hard rules:

- Horto never shows its own password window.
- Horto never stores box user/root/sudo passwords.
- Remote mode does **not** require installing an SSH key on the box.
- **Default:** never write the box `authorized_keys`.
- **Key install:** only with explicit opt-in (`--install-ssh-key` / TUI flag / Desktop checkbox). Off by default.

| Check     | Who asks          | Typical home box                               |
| --------- | ----------------- | ---------------------------------------------- |
| SSH login | `sshd`            | Password **or** existing public key            |
| sudo      | `sudo` on the box | Same account password again, unless `NOPASSWD` |

CLI / TUI: OpenSSH and sudo prompt in the terminal.
Desktop (no TTY): OpenSSH `SSH_ASKPASS` (system askpass binary).

Remote progress: before each SSH/SCP/`ssh-copy-id` step (and before box reboot) the
runner logs `[horto remote] PC → box '…': …` on stderr (`tracing`, default `info`).
Status-api enable (token + unit + install bins) runs in one SSH session so sudo
usually prompts once. Password lines themselves still come from OpenSSH.

### CLI remote examples

Preferred short paths (Make or env), then one raw-flag example:

```bash
# Make (defaults: REMOTE=horto, RELEASE_TAG=dev-preview, dry-run)
make remote-doctor
make remote-setup
make remote-reinstall INSTALL_SSH_KEY=1
make remote-reinstall INSTALL_SSH_KEY=1 APPLY=1

# CLI with env defaults (--remote ← HORTO_REMOTE_HOST)
export HORTO_REMOTE_HOST=horto
export HORTO_RELEASE_TAG=dev-preview
horto-os-ui --install-ssh-key --dry-run doctor
horto-os-ui --dry-run setup run --full

# Explicit flags (no env)
horto-os-ui --remote horto-box --dry-run setup status --full
horto-os-ui --remote horto-box setup run --full
horto-os-ui --remote horto-box --bin-dir target/debug --dry-run doctor
horto-os-ui --remote horto-box --install-ssh-key --dry-run doctor
```

### TUI remote

```bash
horto-os-ui-tui --remote horto-box --dry-run
horto-os-ui-tui --remote horto-box --install-ssh-key
```

Enter / `a` run steps or the full pipeline through the same remote runner.

Full apply pipeline: `s1`…`s7`, then `d0` (Docker Engine), `d1` (prepare `/srv/docker`), `d2` (start Dockge `:5001` and Homepage `:3021`).

### Desktop remote

Connection → **Remote install (OpenSSH)**. Same shared runner as CLI/TUI.

- **Dry-run only** checked by default (preview).
- Uncheck it, confirm the dialog, then **Remote apply setup** installs CLI + TUI + status-api on the box.
- After apply: Connection may offer to save the status-api bearer (localStorage).
  Point **Status API** at `http://<box>:8787`. Paste remains a fallback.
- CLI / TUI propose the same bearer to `~/.config/horto-os-ui/api_token` (`[y/N]`).

### Binaries for the box

Remote mode downloads `horto-os-ui-{V}-{target}.tar.gz` from GitHub Releases for the box arch (`uname -m`), or uses `--bin-dir` / `HORTO_BIN_DIR`.

Default download tag is `v{VERSION}` (matches a stable Release). For tip Pre-release assets use `--release-tag dev-preview` or `HORTO_RELEASE_TAG=dev-preview` (asset filenames still use the Cargo workspace version). Stable `v*` tags keep a warm local cache under the XDG cache dir; tip tags such as `dev-preview` always re-download so an overwritten Pre-release is not stuck on stale bins.

After a successful remote `setup run` (apply), the CLI/TUI prompts to reboot the box so hostname and netplan take effect. Non-TTY surfaces print a reboot reminder instead.

### Practice tests

Unit tests mock OpenSSH. Live Docker SSH fixture (ignored by default):

```bash
make test-remote-docker
```
