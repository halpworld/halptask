# Getting Started with HalpTask 🚀

Welcome to HalpTask! This guide will get you up and running in under 5 minutes, covering installation, basic navigation, bullet hierarchy, task cycling, and your first project outline.

---

## 📦 Installation

Choose the installation method best suited for your platform:

### ⚡ Automatic Script Install (macOS & Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/arkalon76/halptask/main/scripts/install.sh | bash
```

### 🪟 Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/arkalon76/halptask/main/scripts/install.ps1 | iex
```

### 🛠️ Go Install

```bash
go install github.com/arkalon76/halptask@latest
```

### 📦 Standalone Binary Downloads

Pre-built binaries for macOS (Intel & Apple Silicon), Linux (`x86_64` & `arm64`), and Windows are available on the [GitHub Releases Page](https://github.com/arkalon76/halptask/releases/latest).

---

## 🏁 Launching HalpTask

Run `halptask` in your terminal:

```bash
# Launch with default file (~/.config/halptask/data.txt)
halptask

# Launch with a custom task file (short -f or long --file)
halptask -f ~/projects/website_redesign.txt
halptask --file ~/projects/website_redesign.txt

# Prompt for encryption setup on launch (short -e or long --encrypt)
halptask -e
halptask --encrypt

# Print version or check updates (-v / --version, -u / --update, -c / --check-update)
halptask -v
halptask -c
```

---

## 🧱 Core Concepts

 HalpTask combines two powerful workflows into a unified TUI:

### 1. Hierarchical Bullets vs Tasks
- **Bullet (`•`)**: Plain text outline note or node.
- **Task (`[ ]` / `[~]` / `[x]`)**: Interactive item with status indicators:
  - `[ ]` **Todo**: Gray empty checkbox.
  - `[~]` **In Progress**: Highlighted orange active status.
  - `[x]` **Done**: Green checkmark with strikethrough text.

### 2. Parent-Child Subtrees & Indentation
- Sub-items are indented under parent nodes using `Tab` (indent) and `Shift+Tab` (unindent).
- Moving or deleting a parent node automatically carries its entire subtree.

### 3. Folding (`zc` / `zo` / `za` / `zM` / `zR`)
- Collapse subtrees to reduce visual noise (`zc` or `h` on folded node).
- Collapsed nodes display a child counter badge (e.g. `▶ Project Roadmap [5]`).

---

## ⏱️ 5-Minute Quickstart Tutorial

Follow these steps to master the basics:

### Step 1: Create Bullets & Sub-tasks
1. Press `oo` to add a new item below your current selection and enter Insert mode.
2. Type `Q3 Infrastructure Upgrade` and press `Esc` to return to Normal mode.
3. Press `oc` to create a new **child bullet** directly underneath.
4. Type `Migrate database cluster` and press `Esc`.
5. Press `oo` again to add sibling items like `Configure SSL certificates` and `Run benchmark tests`.

### Step 2: Convert Bullets to Tasks & Cycle Status
1. Move your cursor to `Migrate database cluster` using `k` or `j`.
2. Press `t` (or `<space> t t`) to turn the bullet into a task `[ ]`.
3. Press `t` again to cycle its status to `[~]` (**In Progress**).
4. Notice how the status updates in the status bar and the live overview dashboard!

### Step 3: Add Tags & Labels (`T` / `<space> t a`)
1. With your cursor on `Migrate database cluster`, press `T` (or `<space> t a`) to open the **Tag Manager**.
2. Press `a` to attach a tag like `#urgent` or `#backend`.
3. Subtasks underneath will dynamically inherit this tag (`[↖🔥 urgent]`)!

### Step 4: Toggle the Live Dashboard (`<space> d`)
1. Press `<space> d` to toggle the side overview dashboard pane.
2. View total completed vs active tasks and your visual progress bar!

### Step 5: Save & Exit (`<space> w` / `<space> q`)
1. Press `<space> w` or `ww` to save your file.
2. Press `<space> q` to quit.

---

## 🎯 Next Steps

- Explore specialized workflows in [User Personas & Workflows](User-Personas-&-Workflows).
- Discover Vim shortcuts and efficiency secrets in the [Power User Guide](Power-User-Guide).
- Learn how to secure your notes with AES-256 in [Configuration & Security](Configuration-&-Encryption-Security).
