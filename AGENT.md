# Agent Developer Guide for HalpTask 🤖

Welcome agent! This document provides an architectural overview, codebase walkthrough, and development guidelines for AI agents working on **HalpTask**.

---

## 📌 Project Overview

**HalpTask** is a high-performance, keyboard-driven Terminal User Interface (TUI) bullet point and task manager written in **Go** using the **Charm Bubble Tea** framework and **Lip Gloss** styling engine.

### Core Features
- **Hierarchical Outliner**: Indent/unindent bullet points, nested subtrees, folding (`zc`, `zo`, `za`, `zM`, `zR`).
- **Task Management**: Convert bullet points to tasks (`[ ]` Todo, `[~]` In-Progress, `[x]` Done).
- **Keyboard-Driven**: Direct Vim motions (`j`, `k`, `h`, `l`, `gg`, `G`, `o`, `O`, `dd`, `x`, `u`, `ctrl+r`, `tab`, `shift+tab`) plus a Leader Key (`<space>`) WhichKey popup menu system.
- **Storage & Security**: Plain text Markdown storage format (`~/.config/halptask/data.txt`) with optional **AES-256-GCM + PBKDF2** encryption.

---

## 📂 Repository Layout

```
.
├── main.go               # CLI entrypoint, flag parsing, Bubble Tea program launcher
├── config/
│   └── config.go         # YAML config file loader/saver (~/.config/halptask/config.yaml, default_item_type: "bullet"|"task")
├── model/
│   ├── item.go           # Item struct (nodes), TaskStatus enum, VisibleItem flat projection
│   ├── tree.go           # Tree data structure, tree mutations, visibility filtering, undo/redo stack
│   ├── storage.go        # Markdown file parser & serializer, AES-256-GCM encryption engine
│   └── storage_test.go   # Unit tests for storage serialization and encryption
├── ui/
│   ├── app.go            # Primary Bubble Tea AppModel implementing tea.Model (Init, Update, View)
│   ├── keys.go           # Central keybinding registry (GetAllKeyBindings)
│   ├── configmodal.go    # Interactive configuration dashboard modal (<space> c c)
│   ├── configmodal_test.go# Unit tests for ConfigModal
│   ├── whichkey.go       # WhichKey popup renderer for multi-key leader sequences
│   ├── whichkey_test.go  # Unit tests for WhichKey option lookup
│   ├── quickhelp.go      # Persistent unintrusive key hints bar renderer
│   ├── quickhelp_test.go # Unit tests for QuickHelp bar
│   ├── treeview.go       # Tree view renderer (indentation, bullet icons, checkboxes, search highlights)
│   ├── help.go           # Interactive modal cheat sheet renderer
│   └── statusbar.go      # Bottom status bar renderer
├── docs/                 # Documentation hub and user cheatsheet
└── README.md             # Project user-facing documentation
```

---

## 🏗️ Architecture & Key Concepts

### 1. Elm Architecture (Bubble Tea)
HalpTask follows the standard Elm architecture via `tea.Model` in [`ui/app.go`](file:///Users/kenth/code/halptask/ui/app.go):
- **Model**: `AppModel` holds the entire application state (Tree, CursorIndex, Mode, WhichKey, Search Query, Config, Storage).
- **Update**: `Update(msg tea.Msg) (tea.Model, tea.Cmd)` handles keyboard inputs (`tea.KeyMsg`), terminal resize events (`tea.WindowSizeMsg`), and internal messages (`saveResultMsg`, `passphraseSubmitMsg`, `configEditorClosedMsg`).
- **View**: `View() string` renders the full TUI interface composed of Header, TreeView / Modal / WhichKey, and StatusBar.

### 2. Modes State Machine (`ui/app.go`)
The app operates in one of several modes:
- `ModeNormal`: Navigation, fold toggling, leader key popup triggering.
- `ModeInsert`: Inline text editing for a bullet item.
- `ModePassphrasePrompt`: Modal prompt for passphrase when opening or configuring an encrypted file.
- `ModeSearch`: Real-time filtering search bar.
- `ModeHelp`: Keymaps cheat sheet modal overlay.
- `ModeTagPicker`: Interactive tag management modal overlay.
- `ModeConfig`: Interactive configuration dashboard modal overlay.
- `ModeNote`: Interactive task note modal overlay with Markdown view/edit modes and link navigation.

### 3. Tree & Item Data Structure (`model/item.go`, `model/tree.go`)
- Nodes are represented by `*model.Item`:
  - `ID`: Unique string ID.
  - `Text`: String text content.
  - `IsTask`: Boolean indicating whether item is a task checkbox.
  - `Status`: `StatusNone`, `StatusTodo` (`"todo"`), `StatusInProgress` (`"in_progress"`), or `StatusDone` (`"done"`).
  - `Folded`: Boolean for collapsed subtrees.
  - `Note`: Markdown note string attached to the item.
  - `Children`: `[]*Item` slice of sub-items.
  - `Parent`: `*Item` pointer to parent node (`nil` for root items).
- **CRITICAL**: Whenever tree nodes are added, deleted, re-ordered, indented, or loaded, ensure `tree.SetParents()` is called to maintain valid `Parent` pointers.

### 4. Visibility & Projection (`model/tree.go`)
`tree.FlattenVisible()` produces a flat list of `VisibleItem` structs:
- Only items whose parents are not folded are included.
- `Depth` is computed recursively.
- Search queries apply filtering/highlighting across visible items.

### 5. Undo / Redo Engine (`model/tree.go`)
- `tree.SaveState()` deep-clones the root items (`root.Clone()`) and pushes the state onto `tree.UndoStack`.
- `tree.Undo()` and `tree.Redo()` swap state stacks and recalculate parent pointers.
- Structural mutations (add, delete, indent, move up/down) should call `tree.SaveState()` before mutating.

### 6. Storage & Serialization (`model/storage.go`, `model/storage_proto.go`, `proto/v1/storage.proto`)
- **Protobuf v1 Protocol (`.pb`)**:
  - Primary persistence uses **Protobuf v1 binary storage** (`TreeProto`, `ItemProto`) with schema version `1`.
  - Defined in [`proto/v1/storage.proto`](file:///Users/kenth/code/halptask/proto/v1/storage.proto) and implemented with custom zero-dependency marshalling in [`proto/v1/storage.pb.go`](file:///Users/kenth/code/halptask/proto/v1/storage.pb.go).
  - Serializes IDs, titles, task status, fold states, tags, child hierarchy, creation/update timestamps, versioning, node IDs, and attached Markdown notes (`Note`).
  - Detailed specification available in [`docs/storage_protocol.md`](file:///Users/kenth/code/halptask/docs/storage_protocol.md).
- **Legacy Markdown Migration (`MigrateIfNeeded`)**:
  - Legacy `.txt` / `.md` files are parsed and automatically upgraded to `.pb` Protobuf binary payloads while saving a `.bak` backup file.
- **Encryption**:
  - Cipher: **AES-256-GCM**
  - Key Derivation: **PBKDF2** with SHA-256 (100,000 iterations + 16-byte random salt + 12-byte nonce).
  - File Header: `# HALPTASK-ENCRYPTED-v1`. Encrypts raw Protobuf binary payload.

### 7. Task Tags & Dynamic Inheritance (`model/item.go`, `model/tree.go`)
- **Direct Tags**: Assigned explicitly to an `Item` node (`Tags []string`), stored in Markdown as `#tagname`.
- **Inherited Tags**: Dynamically inherited from parent/ancestor nodes at runtime (`tree.GetEffectiveTags()`).
- **Subtask Movement Rules**: Moving subtasks (`Tab`, `Shift+Tab`, `J`, `K`) automatically recalculates inherited parent tags dynamically. Direct tags on subtasks remain untouched.

---

## ⚙️ Development Workflow & Testing

### Building
To compile the project binary:
```bash
go build -o halptask .
```

### Running Tests
To run unit tests across all packages:
```bash
go test ./...
```

To run UI menu layout consistency tests:
```bash
go test -v ./ui -run TestMenuLayoutConsistency
```

To inspect visual snapshots of all UI menus:
```bash
go test -v ./ui -run TestPrintMenuSnapshots
```

To run tests with race detector enabled:
```bash
go test -race ./...
```

### Code Formatting
Ensure Go code is properly formatted:
```bash
go fmt ./...
```

---

## 💡 Guidelines for Agents Modifying Code

1. **Preserve API Contracts & Parent Pointers**:
   Always verify parent references by calling `tree.SetParents()` when modifying tree node relationships.
2. **Auto-Save & Data Persistence for Modal Mutations**:
   Whenever a modal or UI handler mutates tree state, item tags, or configuration settings (such as in `ModeTagPicker` or `ModeInsert`), **always** set `m.PendingAutoSave = true` and call `m.saveFile()` / `config.SaveConfig(m.Config)` upon closing the modal so changes persist immediately to disk.
3. **ANSI-Safe Rendering & Padding Rules**:
   - **Never** pass ANSI-styled strings to `fmt.Sprintf("%-18s", styledString)`. `fmt.Sprintf` counts invisible ANSI control bytes, causing column misalignment and broken line wrapping.
   - **Always** calculate visual width using `lipgloss.Width()` on un-styled text or pad plain strings *before* applying LipGloss styles.
   - **Never** include trailing newlines (`\n`) inside `lipgloss.Render("text\n")`. Move `\n` outside the `Render()` call (`lipgloss.Render("text") + "\n"`), preventing ANSI color reset sequences (`\x1b[0m`) from leaking to the start of subsequent lines and triggering soft wraps.
4. **Keybinding Registration & Prefix Collision Prevention**:
   - When adding or updating keybindings, update `GetAllKeyBindings()` in [`ui/keys.go`](file:///Users/kenth/code/halptask/ui/keys.go) and key dispatchers in [`ui/app.go`](file:///Users/kenth/code/halptask/ui/app.go).
   - **Prefix Collision Rule**: Never register a single-key terminal action in `switch k` if multi-key sequences start with that same prefix (e.g. single `t` must NOT intercept before `tt`, `ts`, `td`, `tp`, `tc` can be typed). Prefix keys must accumulate into `KeyBuffer` rather than executing immediately.
   - Run `go test ./ui/...` to verify that `TestNoKeybindingPrefixCollisions` passes cleanly without prefix collision errors.
5. **Markdown Compatibility**:
   Maintain backwards compatibility with the Markdown storage format in [`model/storage.go`](file:///Users/kenth/code/halptask/model/storage.go). Avoid breaking standard Markdown parsing.
6. **GitHub Wiki Maintenance Directive**:
   - **CRITICAL**: Whenever adding, modifying, or removing features, keybindings, CLI flags, configuration options, or UI workflows, ALWAYS update the GitHub Wiki documentation files located in `wiki/` (`wiki/Home.md`, `wiki/Getting-Started.md`, `wiki/User-Personas-&-Workflows.md`, `wiki/Power-User-Guide.md`, `wiki/Configuration-&-Encryption-Security.md`, `wiki/Keybindings-Reference.md`, `wiki/Agent-Wiki-Maintenance-Guidelines.md`).
   - Maintain image assets in `wiki/images/` and `docs/images/` to ensure visual screenshots match current application design.
7. **Verification Step**:
   Before declaring a task resolved, ALWAYS run `go test ./...` and `go build -o halptask .` to ensure zero compilation or runtime regression errors.
