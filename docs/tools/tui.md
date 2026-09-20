# TUI (`horto-os-ui-tui`)

SSH-friendly ratatui shell. Same engine as the CLI. Mouse capture is off so terminal selection / copy-paste works.

Supports **embedded** (default) and **remote** (`--remote Host`) modes. See [modes.md](modes.md).

On start the TUI clears the alternate screen and stays there for the whole session.
Remote SSH passwords and sudo secrets use system **`SSH_ASKPASS`** (`force_askpass`).
Non-secret y/N (destructive steps, token save, reboot) and free-text (OpenSSH Host on
the SSH tab) use in-TUI Ratatui modals. The TUI never leaves the alternate screen for
prompts.

Tabs: **Setup**, **Overview**, **SSH**, **CLI**, **API**, **MCP**, **Reboot** (remote only), **Logs** (last).
Remote open runs a BatchMode surface probe immediately (footer `local=` / `box=`). Press `r`
to re-probe without leaving the TUI. After probe: `box=<version>`, `missing`, `auth failed`,
or `unreachable`. Auth failure shows a clear status message; the UI stays up.

`s0` (Setup or CLI tab Enter) syncs the tip CLI to the box when the version differs.

## Keys

| Key                | Action                                      |
| ------------------ | ------------------------------------------- |
| `q` / Esc / Ctrl+C | Quit (while typing Host, only Ctrl+C quits) |
| `?`                | Help overlay                                |
| Tab / Shift-Tab    | Next / previous tab                         |
| `1`-`8`            | Jump to tab (remote: `7` Reboot, `8` Logs)  |
| j k / arrows       | Select step (Setup)                         |
| Left / Right       | Full / Minimal kind                         |
| Enter              | Setup: run · SSH: edit Host · other: action |
| `e` / `i`          | SSH: edit Host / install key (opt-in flag)  |
| `a`                | Run pipeline                                |
| `b` / `B`          | `/etc` backup / disk probe in Logs          |
| `r`                | Re-probe surfaces (stays in TUI)            |
| `c`                | Clear Logs (on Logs tab)                    |
| `d`                | Toggle dry-run                              |
| `y` / `n`          | Confirm / cancel (modals)                   |

```bash
make tui
sudo horto-os-ui-tui
horto-os-ui-tui --remote horto-box --dry-run
horto-os-ui-tui --remote horto-box --install-ssh-key   # opt-in key install
```
