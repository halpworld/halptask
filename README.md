# HalpTask 🚀

> A high-performance, keyboard-driven Terminal User Interface (TUI) bullet point outliner & task manager for Vim users. Written in **Go** and **Bubble Tea**, storing tasks in human-readable Markdown files with AES-256 encryption support.

![HalpTask TUI](https://img.shields.io/badge/TUI-Bubble%20Tea-purple)
![Go Version](https://img.shields.io/badge/Go-1.22%2B-blue)
![License](https://img.shields.io/badge/license-MIT-green)
![Category](https://img.shields.io/badge/category-terminal--todo--app-violet)

---

## 📸 Screenshots & Showcase

<p align="center">
  <img src="docs/images/halptask_main.jpg" alt="HalpTask Main TUI View" width="850"/>
</p>

### ⌨️ WhichKey Leader Popup & Config Dashboard

<p align="center">
  <img src="docs/images/halptask_whichkey.jpg" alt="WhichKey Leader Menu" width="415"/>
  &nbsp;
  <img src="docs/images/halptask_config.jpg" alt="Interactive Config Dashboard" width="415"/>
</p>

---

## ⚡ Quick Install (macOS, Linux & Windows)

> [!WARNING]
> ⚠️ **SECURITY WARNING**: Never trust install scripts blindly! Always inspect the source code before running remote scripts piped into your shell. You can inspect the installer source code at [`scripts/install.sh`](scripts/install.sh) (or [`scripts/install.ps1`](scripts/install.ps1) for PowerShell).

Install the latest `halptask` binary automatically for your OS and architecture:

```bash
curl -fsSL https://raw.githubusercontent.com/arkalon76/halptask/main/scripts/install.sh | bash
```

*For native Windows PowerShell:*

```powershell
irm https://raw.githubusercontent.com/arkalon76/halptask/main/scripts/install.ps1 | iex
```

---

## 📚 Documentation & GitHub Wiki

Comprehensive guides and persona workflows are documented in our **[GitHub Wiki](wiki/Home.md)**:

- 🚀 **[Getting Started Guide](wiki/Getting-Started.md)**: 5-minute quickstart, installation, and core concepts.
- 👥 **[User Personas & Workflows](wiki/User-Personas-&-Workflows.md)**: Tailored workflows for **Programmers**, **DevOps/SRE**, **Product Managers**, **Lawyers**, and **Knowledge Workers**.
- ⚡ **[Power User Guide](wiki/Power-User-Guide.md)**: Vim motion combos, subtree hoisting (`ff`), tag inheritance, and shell integration.
- 🔒 **[Configuration & Security](wiki/Configuration-&-Encryption-Security.md)**: Config options, theme palettes, and AES-256-GCM encryption architecture.
- ⌨️ **[Keybindings Cheatsheet](wiki/Keybindings-Reference.md)**: Full interactive Leader Key (`<space>`) map and Vim shortcuts.
- 🤖 **[Agent Maintenance Guidelines](wiki/Agent-Wiki-Maintenance-Guidelines.md)**: Protocols for future AI agents to maintain documentation.

---

## ✨ Features

- **Vim Native Keybindings**: Native Vim navigation (`j`, `k`, `h`, `l`, `gg`, `G`, `oo`, `oc`, `O`, `dd`, `x`, `u`, `ctrl+r`, `tab`, `shift+tab`).
- **Leader Menu (`<space>`)**: Leader key popup window displaying all available shortcuts non-intrusively.
- **Dynamic WhichKey Popup**: Visual popup updates as you type key prefixes for Leader options (`<space> b`, `<space> t`, `<space> z`, `<space> e`) as well as all multi-character Vim prefixes (`o`, `d`, `z`, `g`, `w`, `f`).
- **Bullet & Task Management**:
  - Convert any bullet point into a task with checkbox statuses.
  - **Todo**: `[ ]` (Gray empty box)
  - **In Progress**: `[~]` (Orange `~` indicator)
  - **Done**: `[x]` (Green `X` checkmark with faint strikethrough text styling)
- **Task Tags & Labels 🏷️**:
  - Assign direct tags (`T` or `<space> t a`) with emojis and customizable colors.
  - Subtasks dynamically inherit parent tags (`[↖🔥 urgent]`). Unindenting/moving subtasks automatically cleans up inherited tags.
  - Plain text Markdown storage as `#tagname`.
- **Hierarchical Folding**:
  - Collapse (`zc`, `h`), Expand (`zo`, `l`), Toggle (`za`), Close All (`zM`), Open All (`zR`).
  - Child count badges for collapsed subtrees (`▶ [3]`).
- **Plain Text & Encryption**:
  - Data stored in clean human-readable Markdown format by default (`~/.config/halptask/data.txt`).
  - **AES-256-GCM + PBKDF2** encryption mode for secure task storage.
- **Cross Platform Support**:
  - Binaries built for **macOS**, **Linux**, and **Windows** (`amd64` and `arm64`).
- **Customizable Configuration**:
  - Configured via `~/.config/halptask/config.yaml`.

---

## 📦 Alternative Installation Methods

### Standalone Binary Download

Download direct executable binaries for your OS and Architecture directly from [GitHub Releases](https://github.com/arkalon76/halptask/releases/latest):

```bash
# macOS (Apple Silicon arm64)
curl -LO https://github.com/arkalon76/halptask/releases/latest/download/halptask_Darwin_arm64
chmod +x halptask_Darwin_arm64 && sudo mv halptask_Darwin_arm64 /usr/local/bin/halptask

# Linux (x86_64)
curl -LO https://github.com/arkalon76/halptask/releases/latest/download/halptask_Linux_x86_64
chmod +x halptask_Linux_x86_64 && sudo mv halptask_Linux_x86_64 /usr/local/bin/halptask

# Windows (x86_64)
curl -LO https://github.com/arkalon76/halptask/releases/latest/download/halptask_Windows_x86_64.exe
```

### 🐧 Linux Packages (.deb / .rpm)

- **Debian / Ubuntu**: `sudo apt install ./halptask_<version>_amd64.deb`
- **Fedora / RHEL**: `sudo dnf install ./halptask_<version>_amd64.rpm`

### 🛠️ Build & Install from Source

```bash
# Using Go
go install github.com/arkalon76/halptask@latest

# Or clone and build manually
git clone https://github.com/arkalon76/halptask.git
cd halptask
go build -o halptask .
./halptask
```

### CLI Flags

HalpTask follows standard Unix/POSIX (`nix`) CLI flag conventions, supporting double-dash (`--`) for long options and single-dash (`-`) for short options:

| Short | Long | Description | Example |
|---|---|---|---|
| `-f` | `--file` | Path to halptask data file | `halptask -f ~/tasks.txt` / `halptask --file ~/tasks.txt` |
| `-e` | `--encrypt` | Force enable encryption | `halptask -e` / `halptask --encrypt` |
| `-v` | `--version` | Print version info | `halptask -v` / `halptask --version` |
| `-u` | `--update` | Check and auto-update binary | `halptask -u` / `halptask --update` |
| `-c` | `--check-update` | Check if new version is available | `halptask -c` / `halptask --check-update` |
| `-r` | `--repo` | Override target GitHub repository | `halptask -r owner/repo` / `halptask --repo owner/repo` |


---

## ⌨️ Summary Keybindings Quick Reference

| Leader Key | Category | Command / Action |
|---|---|---|
| `<space> b n` | Bullets | New bullet below |
| `<space> b N` | Bullets | New bullet above |
| `<space> b c` | Bullets | Add child bullet |
| `<space> b e` | Bullets | Edit bullet text |
| `<space> b d` | Bullets | Delete bullet & subtree |
| `<space> b i` | Bullets | Indent bullet (demote) |
| `<space> b o` | Bullets | Unindent bullet (promote) |
| `<space> b j` | Bullets | Move bullet down |
| `<space> b k` | Bullets | Move bullet up |
| `<space> t t` | Tasks | Toggle bullet into task `[ ]` |
| `<space> t c` | Tasks | Cycle status (`Todo` ➜ `In Progress` ➜ `Done`) |
| `<space> t d` | Tasks | Mark Done `[x]` (Green X, strikethrough) |
| `<space> t p` | Tasks | Mark In Progress `[~]` (Orange ~) |
| `<space> t s` | Tasks | Mark Todo `[ ]` (Gray empty) |
| `<space> t a` / `T` | Tasks | Manage task tags & labels |
| `<space> t D` | Tasks | Toggle default creation type (`bullet` <-> `task`) |
| `<space> c c` | Config | Open interactive Config Dashboard modal |
| `<space> c a` | Config | Toggle Auto-Save |
| `<space> c d` | Config | Toggle default creation item type (`bullet` <-> `task`) |
| `<space> c t` | Config | Cycle visual theme palette |
| `<space> c w` | Config | Toggle WhichKey popup menu |
| `<space> c e` | Config | Open `config.yaml` in `$EDITOR` (auto-reloaded on exit) |
| `<space> z c` | Folds | Close fold |
| `<space> z o` | Folds | Open fold |
| `<space> z a` | Folds | Toggle fold |
| `<space> z M` | Folds | Close all folds |
| `<space> z R` | Folds | Open all folds |
| `<space> e e` | Encrypt | Toggle encryption |
| `<space> e p` | Encrypt | Set / Change passphrase |
| `<space> w` | File | Save file |
| `<space> /` | Search | Search bullet points |
| `<space> ?` | Help | Show keymap cheat sheet modal |
| `<space> q` | Quit | Save and exit |

*For full details, view the [Keybindings Cheatsheet](docs/cheatsheet.md).*

---

## ⚙️ In-App & File Configuration (`~/.config/halptask/config.yaml`)

HalpTask provides a **LazyVim-inspired configuration model**:
1. **Interactive Config Dashboard (`<space> c c`)**: Browse categorized settings (`General`, `UI & Appearance`, `Storage & Security`), toggle booleans with `Space`/`Enter`, and cycle themes with instant disk persistence.
2. **Quick Leader Toggles**: One-keypress toggles like `<space> c a` (Auto-Save), `<space> c d` (Default Item), and `<space> c t` (Cycle Theme).
3. **External Editor Harmony (`<space> c e`)**: Launches your preferred `$EDITOR` on `~/.config/halptask/config.yaml`, auto-reloading changes on save.

```yaml
auto_save: true
check_updates: true
data_file: ~/.config/halptask/data.txt
default_item_type: bullet # "bullet" or "task"
encrypted: false
indent_spaces: 2
leader_key: " "
show_which_key: true
theme: default
```

**Auto-Save & Encrypted Files**: 
When `auto_save: true`, HalpTask will automatically save all tree state mutations in the background. If you open or create an encrypted file but haven't provided a passphrase yet, auto-save will pause until you enter your passphrase to prevent data loss or lockouts.

---

## 🛠️ Releases & Compilation

We use [GoReleaser](https://goreleaser.com) and GitHub Actions to automatically build and release binaries for macOS, Linux, and Windows across `amd64` and `arm64` architectures.

For full details on the automated release process or how to build releases locally, refer to the **[Release Guide](docs/RELEASE.md)**.

---

## 📄 License

MIT License. Developed with Go & Charm Bubble Tea.

---

## 💡 Acknowledgments

Special thanks to the [LazyVim](https://github.com/LazyVim/LazyVim) project for inspiration.
