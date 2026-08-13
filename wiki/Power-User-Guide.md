# Power User Workflow & Tips ⚡

Take your HalpTask productivity to the highest level! This guide details advanced Vim shortcuts, subtree hoisting hacks, dynamic tag mechanics, configuration fine-tuning, and terminal shell integration.

---

## ⌨️ 1. Vim Motion Combos & Lightning Navigation

HalpTask is built around authentic Vim navigation and manipulation concepts.

```
 Navigation          Subtree Editing            Folding & Focus
 ──────────          ───────────────            ───────────────
 j / k : Down/Up     oo : New below             zc / zo : Close/Open fold
 h / l : Parent/Child oc : New child (subtask)  za      : Toggle fold
 gg    : Jump Top    dd : Delete subtree        zM / zR : Close/Open all
 G     : Jump Bottom J/K: Move down/up          ff      : Hoist / Zoom view
 gi    : Jump to ID  u/Ctrl+r: Undo/Redo        <space>gi: Jump to ID
```

### Motion Combos to Master:
- **`gi` or `<space> g i`**: Jump directly to any task or bullet item by its permanent `#ID` number!
- **`oo` vs `oc`**: `oo` creates a sibling bullet directly below; `oc` creates a child subtask underneath current selection.
- **`i` / `a` / `e`**: Enter insert mode at the current node text.
- **`c`**: Clear the node text entirely and instantly enter insert mode.
- **`J` / `K`**: Shift items up or down among their siblings without losing subtree structure.
- **`Tab` / `Shift+Tab`**: Demote (indent) or promote (unindent) items across hierarchical levels.
- **`u` / `Ctrl+r`**: Full undo/redo stack for all structural changes, node deletions, and status toggles.

---

## 🚀 2. Speed Hacks: Single `t` vs Leader Keys

HalpTask features two distinct ways to interact with tasks:

### Lightning 1-Key Task Toggle (`t`)
In Normal mode, pressing single `t` performs an instant status cycle:
1. `•` (Bullet) ➔ `[ ]` (Todo)
2. `[ ]` (Todo) ➔ `[~]` (In Progress)
3. `[~]` (In Progress) ➔ `[x]` (Done)
4. `[x]` (Done) ➔ `•` (Bullet)

> [!TIP]
> **Power Tip**: Tap `t` rapidly while moving with `j`/`k` to update task statuses across your list without opening any menus!

### Explicit Leader Shortcuts (`<space> t ...`)
When you need direct, non-cycling status assignment:
- `<space> t d`: Mark **Done** (`[x]`) instantly.
- `<space> t p`: Mark **In Progress** (`[~]`) instantly.
- `<space> t s`: Mark **Todo** (`[ ]`) instantly.
- `<space> t a`: Open **Tag Manager** (`🏷️`).
- `<space> n` / `N`: Open / Edit **Task Markdown Note** (`📝`).
- `<space> t D`: Toggle **Default Creation Type** (`bullet` <-> `task`).

### 📝 Task Notes & Cross-Task Linking (`N` or `<space> n`)
Attach detailed Markdown context to any task item.
- Press `N` or `<space> n` to open the Note modal.
- Supports rich Markdown formatting (headings `#`, lists `-`, quotes `>`, code blocks ` ``` `).
- Write internal task links in flexible formats: `#123`, `[Label](#123)`, `[Label](123)`, `task:123`.
- Press `Tab` / `Shift+Tab` to cycle between links in View mode.
- Press `Enter` on a focused link to instantly close the note and navigate to that task in your tree view!

---

## 🔍 3. Subtree Hoisting / Zooming (`ff`)

When working inside deep task trees with hundreds of items, extraneous sections can cause visual distraction.

<p align="center">
  <img src="images/halptask_main.jpg" alt="Subtree View" width="800"/>
</p>

- Press **`ff`** on any parent node to **Hoist / Zoom** into that node.
- The screen zooms in, making the focused node the temporary root of your viewport!
- Press **`ff`** again to un-hoist and return to full document view.

---

## 🧹 4. Task Cleanup: Hide & Purge Completed Tasks

Keep your workplace tidy with completed task management shortcuts:

- **`fc` (Toggle Completed Filter)**: Hides or shows all `[x]` completed tasks from your view without deleting them. Perfect for focusing strictly on active work!
- **`da` (Delete All Completed)**: Permanently purges all `[x]` completed tasks from the current document.

---

## 🏷️ 5. Dynamic Tag Inheritance Secrets

HalpTask features a sophisticated tag hierarchy system:

1. **Direct Tags**: Created with `T` or `<space> t a` and stored in plain text as `#tagname`.
2. **Inherited Tags**: Dynamically derived from ancestor nodes. Subtasks automatically display `[↖🔥 urgent]`.
3. **Subtask Reparenting Behavior**:
   - When you press `Tab` to demote a task under a tagged parent, it **instantly inherits** the parent's tags.
   - When you press `Shift+Tab` to promote a subtask out of a tagged parent, the inherited parent tags are **automatically removed**, preserving only direct tags.

---

## 🖥️ 6. Terminal Shell Integration & Workflows

### Multi-File Project Vaults
HalpTask supports opening project-specific files directly:

```bash
# Work-specific tasks
halptask -f ~/work/sprint_tasks.txt

# Personal goals vault
halptask -f ~/personal/goals.txt
```

### Useful Shell Aliases (`~/.zshrc` / `~/.bashrc`)
```bash
# Quick aliases
alias ht="halptask"
alias htp="halptask -f ~/projects/main.txt"
alias htv="halptask --encrypt -f ~/.config/halptask/vault.txt"
```

### Tmux Split Window Workflow
Keep HalpTask permanently open alongside your code editor inside Tmux:

```bash
# In Tmux session:
tmux split-window -h -p 35 "halptask -f .tasks.txt"
```

---

## ⚙️ 7. Configuration Dashboard & Editor Harmony

HalpTask provides two seamless ways to tune settings:

### Interactive Config Dashboard (`<space> c c`)
1. Press `<space> c c` to launch the interactive modal.
2. Navigate settings with `j`/`k` or `↑`/`↓`.
3. Press `Space` or `Enter` to toggle booleans (e.g. `auto_save`, `show_which_key`).
4. Press `t` to cycle visual color themes (`default`, `tokyonight`, `catppuccin`, `dracula`, `nord`).
5. Settings persist instantly to disk!

<p align="center">
  <img src="images/halptask_config.jpg" alt="Interactive Config Dashboard" width="450"/>
</p>

### External Editor Harmony (`<space> c e`)
Press `<space> c e` to launch your system `$EDITOR` (Neovim, Vim, Nano) directly on `~/.config/halptask/config.yaml`. When you save and exit your editor, HalpTask automatically reloads the updated configuration without restarting!
