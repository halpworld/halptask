# HalpTask Keybindings & Commands Cheatsheet ⌨️

HalpTask uses native **Vim** keybindings combined with well-known Leader Key (`<space>`) shortcuts.

---

## 🚀 Leader Key Menu (`<space>`)

Pressing `<space>` or pressing the first key of any multi-character command (such as `o`, `d`, `z`, `g`, `w`, `f`) opens the interactive **WhichKey popup window** at the bottom of your screen. As you press subsequent keys, the popup dynamically updates to show available options and completion choices.

```
┌ Space (Leader Menu) ─────────────────────────────────────────────────────────┐
│ b  +bullets   │ t  +tasks     │ z  +folds     │ e  +encrypt   │ w  save      │
│ /  search     │ ?  help       │ q  quit       │                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 1. Bullets Menu (`<space> b ...`)

| Shortcut | Description | Detail |
|---|---|---|
| `<space> b n` | New bullet below | Creates a bullet point directly below current selection |
| `<space> b N` | New bullet above | Creates a bullet point directly above current selection |
| `<space> b c` | Add child bullet | Creates a sub-bullet as a child of current selection |
| `<space> b e` | Edit text | Edits current bullet point text in inline insert mode |
| `<space> b d` | Delete bullet | Deletes current bullet point and all its sub-bullets |
| `<space> b i` | Indent bullet | Demotes bullet to become child of preceding sibling (`Tab`) |
| `<space> b o` | Unindent bullet | Promotes bullet up one hierarchy level (`Shift+Tab`) |
| `<space> b j` | Move down | Swaps position with next sibling |
| `<space> b k` | Move up | Swaps position with previous sibling |
| `<space> b t` | Toggle task | Converts bullet into task or cycles task status |

### 2. Task Menu (`<space> t ...`)

| Shortcut | Visual Indicator | Status Description |
|---|---|---|
| `<space> t t` | `[ ]` / `•` | Toggle between plain bullet `•` and task `[ ]` |
| `<space> t c` | `[ ]` ➔ `[~]` ➔ `[x]` | Cycle status (Todo ➔ In-Progress ➔ Done ➔ Todo) |
| `<space> t s` | `[ ]` | Mark as **Todo** (Gray empty checkbox) |
| `<space> t p` | `[~]` | Mark as **In Progress** (Orange `~` indicator) |
| `<space> t d` | `[x]` | Mark as **Done** (Green `X`, faint strikethrough text) |
| `Esc` / `q` / `fo` / `tf` | `🎯` | Exit / Toggle **Current Focus Task** & display top Focus Banner |
| `<space> t a` | `🏷️` | Open **Tag / Label Manager** overlay |
| `<space> n` / `<space> t n` | `📝` | Open / Edit **Task Markdown Note** modal |
| `<space> t D` | `⚙️` | Toggle default item creation type (`bullet` <-> `task`) |

### 3. Fold Menu (`<space> z ...`)

| Shortcut | Action | Description |
|---|---|---|
| `<space> z c` | Close Fold | Collapses sub-bullets of selected node |
| `<space> z o` | Open Fold | Expands sub-bullets of selected node |
| `<space> z a` | Toggle Fold | Toggles fold state of selected node |
| `<space> z M` | Close All Folds | Folds every nested node across entire document |
| `<space> z R` | Open All Folds | Unfolds every node across entire document |

### 4. Configuration Menu (`<space> c ...`)

| Shortcut | Command | Description |
|---|---|---|
| `<space> c c` | Config Dashboard | Open interactive, keyboard-navigable configuration modal |
| `<space> c a` | Toggle Auto-Save | Turn background auto-save on/off |
| `<space> c d` | Toggle Default Item | Switch default creation node type (`bullet` <-> `task`) |
| `<space> c t` | Cycle Visual Theme | Cycle theme palette (`default`, `tokyonight`, `catppuccin`, `dracula`, `nord`) |
| `<space> c w` | Toggle WhichKey | Show or hide Leader key WhichKey popup menu |
| `<space> c e` | Edit Config File | Launch `$EDITOR` on `~/.config/halptask/config.yaml` with auto-reload |

### 5. Encryption & System (`<space> e ...`, `<space> w`, `<space> q`)

| Shortcut | Action | Description |
|---|---|---|
| `<space> e e` | Toggle Encryption | Toggle AES-256-GCM encryption on file save |
| `<space> e p` | Set Passphrase | Set or change file encryption passphrase |
| `<space> w` | Save | Save current document to disk |
| `<space> s` | Save | Save current document to disk |
| `<space> /` | Search | Open search input filter bar |
| `<space> ?` | Help Modal | Display on-screen keybindings cheat sheet |
| `<space> q` | Quit | Save and exit HalpTask |

---

## 🎯 Direct Normal Mode Keybindings

You can also use single or double-character Vim motions directly without pressing `<space>`.

> [!NOTE]
> **Single-Key Efficiency**: Single `t` in Normal Mode provides lightning-fast 1-keypress task toggling and cycling (`•` ➔ `[ ]` ➔ `[~]` ➔ `[x]`). Explicit task statuses (`[x]` Done, `[~]` In-Progress, `[ ]` Todo) are accessible via the Leader key menu (`<space> t d`, `<space> t p`, `<space> t s`).

### Motion & Navigation

| Key | Description |
|---|---|
| `j` or `↓` | Move selection cursor down |
| `k` or `↑` | Move selection cursor up |
| `h` or `←` | Close fold / Jump to parent node |
| `l` or `→` | Open fold / Jump to first child node |
| `gg` | Jump to top of document |
| `G` | Jump to bottom of document |
| `ff` | Zoom / Hoist focused subtree |

### Editing & Tree Manipulation

| Key | Action |
|---|---|
| `oo` | Create new bullet below and enter insert mode |
| `oc` | Create new child bullet (subtask) and enter insert mode |
| `O` | Create new bullet above and enter insert mode |
| `i` / `a` / `e` | Edit text of selected bullet |
| `c` | Clear line text & enter insert mode |
| `dd` / `x` | Delete selected bullet and its children |
| `Tab` or `>` | Indent selected bullet (make child) |
| `Shift+Tab` or `<` | Unindent selected bullet (make sibling of parent) |
| `J` | Move bullet down among siblings |
| `K` | Move bullet up among siblings |
| `t` | Toggle bullet into task / Cycle task status |
| `T` | Open **Tag / Label Manager** overlay |
| `N` | Open / Edit **Task Markdown Note** modal |
| `fc` | Toggle hide/show `[x]` completed tasks |
| `da` | Delete all `[x]` completed tasks |
| `ww` | Quick save file to disk |
| `u` | Undo last modification |
| `Ctrl+r` | Redo last modification |

---

## 🏷️ Task Tags & Labels

HalpTask supports rich tag management with emojis and customizable colors.

### Key Features & Rules:
- **Direct Tags**: Assigned explicitly to a task (`T` or `<space> t a`). Persisted as `#tagname` in markdown files. Rendered as `[🐛 bug]`.
- **Inherited Tags**: Dynamically inherited from parent tasks down the subtree. Rendered with an inherited indicator `[↖🔥 urgent]`.
- **Subtask Movement Rules**:
  - Moving a subtask out of a parent (`Shift+Tab` / move out) automatically removes inherited parent tags. Direct tags remain untouched.
  - Moving a subtask into a parent (`Tab` / move in) dynamically inherits the new parent's tags.
- **Tag Creation Wizard**: Press `n` inside the Tag Overlay (`T`) to create custom tags with custom emojis and a 12-color ANSI palette.

### Fold Shortcuts

| Key | Action |
|---|---|
| `Enter` / `za` | Toggle fold |
| `zc` | Close fold |
| `zo` | Open fold |
| `zM` | Close all folds in document |
| `zR` | Open all folds in document |

---

## 🔍 Search Mode (`/`)

1. Press `/` or `<space> /` to open the search bar at the bottom.
2. Type your search query. Matching items will be highlighted in orange.
3. Cursor automatically moves to the first match.
4. Press `Enter` or `Esc` to exit search mode while keeping highlights.

---

## 🔒 Encryption Details

When encryption is enabled:
1. Data files start with the header `# HALPTASK-ENCRYPTED-v1`.
2. Content is encrypted using **AES-256-GCM** with **PBKDF2** key derivation (100,000 iterations).
3. Upon opening an encrypted file, HalpTask prompts for your secret passphrase before rendering the TUI.
