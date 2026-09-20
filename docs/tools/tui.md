# TUI (`horto-os-ui-tui`)

SSH-friendly ratatui shell. Same engine as the CLI. Mouse capture is off so terminal selection / copy-paste works.

Supports **embedded** (default) and **remote** (`--remote Host`) modes. See [modes.md](modes.md).

On start the TUI clears the alternate screen. Before remote SSH / sudo / interactive
prompts (setup apply, key install, CLI sync, reboot) it leaves the TUI, runs on the
normal terminal, then restores a clean frame.

Tabs: **Setup**, **Overview**, **SSH**, **CLI**, **API**, **MCP**, **Reboot** (remote only), **Logs** (last).
Remote open runs a BatchMode surface probe immediately (footer `local=` / `box=`). Press `r`
to re-probe without leaving the TUI. After probe: `box=<version>`, `missing`, `auth failed`,
or `unreachable`.

`s0` (Setup or CLI tab Enter) syncs the tip CLI to the box when the version differs.

## Keys

| Key                | Action                                      |
| ------------------ | ------------------------------------------- |
| `q` / Esc / Ctrl+C | Quit                                        |
| `?`                | Help overlay                                |
| Tab / Shift-Tab    | Next / previous tab                         |
| `1`-`8`            | Jump to tab (remote: `7` Reboot, `8` Logs)  |
| j k / arrows       | Select step (Setup)                         |
| Left / Right       | Full / Minimal kind                         |
| Enter              | Setup: run step · surface tabs: tab action  |
| `a`                | Run pipeline                                |
| `b` / `B`          | `/etc` backup / disk probe in Logs          |
| `r`                | Re-probe surfaces (stays in TUI)            |
| `c`                | Clear Logs (on Logs tab)                    |
| `d`                | Toggle dry-run                              |
| `y` / `n`          | Confirm / cancel                            |

```bash
make tui
sudo horto-os-ui-tui
horto-os-ui-tui --remote horto-box --dry-run
horto-os-ui-tui --remote horto-box --install-ssh-key   # opt-in key install
```
