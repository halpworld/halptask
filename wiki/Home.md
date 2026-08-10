# Welcome to the HalpTask Wiki 🚀

> **HalpTask** is a high-performance, keyboard-driven Terminal User Interface (TUI) bullet outliner & task manager designed for speed, security, and developer productivity. Built in Go with Charm Bubble Tea & Lip Gloss, storing tasks in human-readable Markdown with optional AES-256-GCM encryption.

<p align="center">
  <img src="images/halptask_banner.jpg" alt="HalpTask Wiki Banner" width="900"/>
</p>

---

## 🧭 Navigation & Overview Sitemap

| Section | Description |
|---|---|
| 🚀 **[Getting Started](Getting-Started)** | Installation, first launch, core concepts, and a 5-minute quickstart guide. |
| 👥 **[User Personas & Workflows](User-Personas-&-Workflows)** | Tailored guides for Programmers, DevOps/SRE, Product Managers, Lawyers, and Knowledge Workers. |
| ⚡ **[Power User Guide](Power-User-Guide)** | Master Vim motion combos, WhichKey leader hacks, subtree hoisting (`ff`), dynamic tag inheritance, and terminal shell integration. |
| ⚙️ **[Configuration & Security](Configuration-&-Encryption-Security)** | Customizing `config.yaml`, using the Config Dashboard (`<space> c c`), theme switching, and AES-256-GCM encryption setup. |
| ⌨️ **[Keybindings Reference](Keybindings-Reference)** | Exhaustive, categorized cheatsheet for all Leader Key (`<space>`) sequences and Vim Normal Mode motions. |
| 🤖 **[Agent Wiki Maintenance Guidelines](Agent-Wiki-Maintenance-Guidelines)** | Protocols for AI Coding Agents to keep documentation and Wiki pages synchronized with code updates. |

---

## 📸 Interface Preview

<p align="center">
  <img src="images/halptask_main.jpg" alt="HalpTask Main Interface" width="850"/>
</p>

<p align="center">
  <img src="images/halptask_whichkey.jpg" alt="WhichKey Leader Menu" width="420"/>
  &nbsp;
  <img src="images/halptask_config.jpg" alt="Interactive Config Dashboard" width="420"/>
</p>

---

## ✨ Core Highlights

- **Vim Native Motion Engine**: Seamless `j`/`k`/`h`/`l`, `gg`/`G`, `gi` (jump to ID), `oo`/`oc`, `dd`, `x`, `u`, `ctrl+r`, `tab`/`shift+tab` tree editing.
- **Permanent Task/Bullet Numbering**: Each item has a persistent ID (`<!-- id: # -->`). Instantly jump to any node with `gi` or `<space> g i`.
- **Leader Key & WhichKey System (`<space>`)**: Non-intrusive popup menu guide that updates in real time as you type key sequences.
- **Hierarchical Outliner & Folding**: Collapse subtrees (`zc`), expand (`zo`), toggle (`za`), or fold everything (`zM`/`zR`) with dynamic child counters.
- **Task Management**: Convert bullets into rich tasks with status cycling: `[ ]` Todo ➔ `[~]` In Progress ➔ `[x]` Done.
- **Tag & Label System 🏷️**: Assign direct tags (`T` / `<space> t a`) with custom emojis & colors. Subtasks dynamically inherit parent tags (`[↖🔥 urgent]`).
- **Live Dashboard Panel (`<space> d`)**: Real-time task statistics, completion progress bar (`[████░░░░] 50%`), and active in-progress task focus view.
- **AES-256-GCM Encryption**: Secure sensitive task vaults with strong passphrase protection and zero plaintext memory leakage.
- **Plaintext Markdown Storage**: Standard bullet and checkbox format (`~/.config/halptask/data.txt`), compatible with standard text editors.

---

> [!IMPORTANT]
> **Notice for AI Agents**: When modifying features, keybindings, or configuration settings in HalpTask, **always update this Wiki repository**. See [Agent Wiki Maintenance Guidelines](Agent-Wiki-Maintenance-Guidelines) for details.
