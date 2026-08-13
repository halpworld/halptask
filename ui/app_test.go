package ui

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/kenth/halptask/config"
	"github.com/kenth/halptask/model"
	"github.com/kenth/halptask/updater"
)

func TestNormalModeTaskShortcuts(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	item := tree.InsertBelow("", "Test Bullet")

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}
	app.ensureValidCursor()
	app.SelectedID = item.ID

	// Test single key 't': Instant toggle bullet -> task (Todo)
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'t'}})
	app = m.(AppModel)
	updatedItem := app.Tree.FindItem(item.ID)
	if !updatedItem.IsTask || updatedItem.Status != model.StatusTodo {
		t.Fatalf("expected item to be Todo task after single 't', got isTask=%v status=%s", updatedItem.IsTask, updatedItem.Status)
	}

	// Test single key 't' again: Cycle to InProgress
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'t'}})
	app = m.(AppModel)
	updatedItem = app.Tree.FindItem(item.ID)
	if updatedItem.Status != model.StatusInProgress {
		t.Fatalf("expected item status InProgress after second 't', got %s", updatedItem.Status)
	}

	// Test single key 't' third time: Cycle to Done
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'t'}})
	app = m.(AppModel)
	updatedItem = app.Tree.FindItem(item.ID)
	if updatedItem.Status != model.StatusDone {
		t.Fatalf("expected item status Done after third 't', got %s", updatedItem.Status)
	}

	// Test Leader key '<space> t s': Mark Todo
	app.tryExecuteKeyBinding([]string{" ", "t", "s"})
	updatedItem = app.Tree.FindItem(item.ID)
	if updatedItem.Status != model.StatusTodo {
		t.Fatalf("expected item status Todo after Leader '<space> t s', got %s", updatedItem.Status)
	}

	// Test Leader key '<space> t d': Mark Done
	app.tryExecuteKeyBinding([]string{" ", "t", "d"})
	updatedItem = app.Tree.FindItem(item.ID)
	if updatedItem.Status != model.StatusDone {
		t.Fatalf("expected item status Done after Leader '<space> t d', got %s", updatedItem.Status)
	}

	// Test Leader key '<space> t p': Mark In Progress
	app.tryExecuteKeyBinding([]string{" ", "t", "p"})
	updatedItem = app.Tree.FindItem(item.ID)
	if updatedItem.Status != model.StatusInProgress {
		t.Fatalf("expected item status InProgress after Leader '<space> t p', got %s", updatedItem.Status)
	}
}

func TestTier1AndTier2Shortcuts(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	parent := tree.InsertBelow("", "Parent Item")
	child := tree.AddChild(parent.ID, "Child Item")
	_ = child
	doneTask := tree.InsertBelow("", "Done Task")
	tree.SetStatus(doneTask.ID, model.StatusDone)

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		TextInput: textinput.New(),
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}
	app.ensureValidCursor()
	app.SelectedID = parent.ID

	// Test 'enter': Toggle fold
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'z'}})
	_ = m
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyEnter})
	app = m.(AppModel)
	pItem := app.Tree.FindItem(parent.ID)
	if !pItem.Folded {
		t.Fatalf("expected parent item to be folded after pressing enter")
	}

	// Test 'c': Clear text and enter insert mode
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'c'}})
	app = m.(AppModel)
	if app.Mode != ModeInsert || app.TextInput.Value() != "" {
		t.Fatalf("expected insert mode with empty text after 'c', got mode=%v val=%q", app.Mode, app.TextInput.Value())
	}
	app.Mode = ModeNormal // reset back to normal

	// Test 'fc': Toggle hide completed
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'f'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'c'}})
	app = m.(AppModel)
	if !app.HideCompleted {
		t.Fatalf("expected HideCompleted to be true after 'fc'")
	}

	// Test 'ff': Zoom into focused subtree
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'f'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'f'}})
	app = m.(AppModel)
	if app.ZoomedID != parent.ID {
		t.Fatalf("expected ZoomedID to be parent.ID after 'ff', got %q", app.ZoomedID)
	}

	// Test 'da': Delete all done tasks
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'d'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'a'}})
	app = m.(AppModel)
	if app.Tree.FindItem(doneTask.ID) != nil {
		t.Fatalf("expected done task to be deleted after 'da'")
	}
}

func TestQuickHelpInView(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	tree.InsertBelow("", "Test Bullet")

	app := AppModel{
		Config:    cfg,
		Storage:   model.NewStorage("test.md", false),
		Tree:      tree,
		Mode:      ModeNormal,
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
		Width:     80,
		Height:    24,
	}

	viewStr := app.View()
	if !strings.Contains(viewStr, "<space>") || !strings.Contains(viewStr, "leader") {
		t.Fatalf("expected app View output to render quick help bar containing leader key shortcuts")
	}
}

func TestOpenAndChildShortcuts(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	item := tree.InsertBelow("", "Root Bullet 1")

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		TextInput: textinput.New(),
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}
	app.ensureValidCursor()
	app.SelectedID = item.ID

	// Test 'oc': Add child bullet
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'c'}})
	app = m.(AppModel)

	if app.Mode != ModeInsert {
		t.Fatalf("expected insert mode after 'oc', got mode=%v", app.Mode)
	}
	rootItem := app.Tree.FindItem(item.ID)
	if len(rootItem.Children) != 1 {
		t.Fatalf("expected rootItem to have 1 child after 'oc', got %d", len(rootItem.Children))
	}

	// Test 'oo': New bullet below (sibling) on a bullet without children
	item2 := app.Tree.InsertBelow("", "Root Bullet 2")
	app.Mode = ModeNormal
	app.SelectedID = item2.ID

	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)

	if app.Mode != ModeInsert {
		t.Fatalf("expected insert mode after 'oo', got mode=%v", app.Mode)
	}
	if len(app.Tree.Roots) != 3 {
		t.Fatalf("expected 3 root items after 'oo', got %d", len(app.Tree.Roots))
	}
}

func TestInsertBelowOnTaskWithSubtasks(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	mainTask := tree.InsertBelow("", "Main Task")
	tree.AddChild(mainTask.ID, "Subtask 1")
	tree.AddChild(mainTask.ID, "Subtask 2")
	mainTask.Folded = false

	if len(mainTask.Children) != 2 {
		t.Fatalf("expected 2 subtasks, got %d", len(mainTask.Children))
	}

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		TextInput: textinput.New(),
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}

	// Highlight main task and press 'oo'
	app.SelectedID = mainTask.ID
	app.Mode = ModeNormal

	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)

	if app.Mode != ModeInsert {
		t.Fatalf("expected insert mode after 'oo', got mode=%v", app.Mode)
	}

	// Root should now have 2 items (Main Task, New Task), and Main Task should still have 2 subtasks
	if len(app.Tree.Roots) != 2 {
		t.Fatalf("expected 2 root items at main task level after 'oo', got %d", len(app.Tree.Roots))
	}
	if len(mainTask.Children) != 2 {
		t.Fatalf("expected mainTask to still have 2 children, got %d", len(mainTask.Children))
	}
}

func TestTagPickerAutoSave(t *testing.T) {
	tmpDir := t.TempDir()
	dataFile := tmpDir + "/tasks.txt"

	cfg := config.DefaultConfig()
	cfg.AutoSave = true
	cfg.DataFile = dataFile

	storage := model.NewStorage(dataFile, false)
	tree := model.NewTree()
	item := tree.InsertBelow("", "Build feature")

	app := AppModel{
		Config:    cfg,
		Storage:   storage,
		Tree:      tree,
		Mode:      ModeNormal,
		TagModal:  NewTagModal(cfg.Tags),
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}
	app.ensureValidCursor()
	app.SelectedID = item.ID

	// 1. Open TagPicker via 'T'
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'T'}})
	app = m.(AppModel)
	if app.Mode != ModeTagPicker {
		t.Fatalf("Expected AppMode to be ModeTagPicker after pressing 'T', got %v", app.Mode)
	}

	// 2. Toggle tag 1 ("bug") in TagModal
	m, _ = app.Update(tea.KeyMsg{Type: tea.KeyEnter}) // toggle first tag
	app = m.(AppModel)

	targetItem := app.Tree.FindItem(item.ID)
	if !targetItem.HasDirectTag("bug") {
		t.Fatalf("Expected target item to have direct tag 'bug'")
	}

	// 3. Close TagPicker modal via 'esc'
	m, _ = app.Update(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)

	if app.Mode != ModeNormal {
		t.Fatalf("Expected mode to reset to ModeNormal after esc, got %v", app.Mode)
	}

	// 4. Verify data file on disk contains the tag #bug!
	loadedTree, err := storage.Load("")
	if err != nil {
		t.Fatalf("Failed to load saved data file: %v", err)
	}
	if len(loadedTree.Roots) == 0 || !loadedTree.Roots[0].HasDirectTag("bug") {
		t.Fatalf("Saved file on disk missing tag 'bug'")
	}
}

func TestDefaultItemTypeCreation(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.DefaultItemType = "task" // Set default creation to task

	tree := model.NewTree()
	item := tree.InsertBelow("", "Root Bullet")

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		TextInput: textinput.New(),
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}
	app.ensureValidCursor()
	app.SelectedID = item.ID

	// Create new item below using 'oo'
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)

	visible := app.getVisibleItems()
	if len(visible) != 2 {
		t.Fatalf("expected 2 visible items, got %d", len(visible))
	}

	createdItem := visible[1].Item
	if !createdItem.IsTask || createdItem.Status != model.StatusTodo {
		t.Fatalf("expected newly created item to be a Todo task, got isTask=%v status=%s", createdItem.IsTask, createdItem.Status)
	}

	// Switch default item type back to "bullet"
	app.Config.DefaultItemType = "bullet"
	app.Mode = ModeNormal

	// Create new item below using 'oo'
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)

	visible = app.getVisibleItems()
	if len(visible) != 3 {
		t.Fatalf("expected 3 visible items, got %d", len(visible))
	}

	createdBullet := visible[2].Item
	if createdBullet.IsTask {
		t.Fatalf("expected newly created item to be a bullet point (isTask=false), got isTask=true")
	}
}

func TestToggleDefaultItemType(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.DefaultItemType = "bullet"

	app := AppModel{
		Config:    cfg,
		Tree:      model.NewTree(),
		Mode:      ModeNormal,
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}

	// Toggle via Leader key '<space> t D'
	app.tryExecuteKeyBinding([]string{" ", "t", "D"})
	if app.Config.DefaultItemType != "task" {
		t.Fatalf("expected DefaultItemType to be 'task' after toggle, got %s", app.Config.DefaultItemType)
	}
	if !strings.Contains(app.StatusMsg, "Task") {
		t.Fatalf("expected status message to mention Task, got %q", app.StatusMsg)
	}

	// Toggle again
	app.tryExecuteKeyBinding([]string{" ", "t", "D"})
	if app.Config.DefaultItemType != "bullet" {
		t.Fatalf("expected DefaultItemType to be 'bullet' after second toggle, got %s", app.Config.DefaultItemType)
	}
	if !strings.Contains(app.StatusMsg, "Bullet") {
		t.Fatalf("expected status message to mention Bullet, got %q", app.StatusMsg)
	}
}

func TestLeaderConfigSequence(t *testing.T) {
	cfg := config.DefaultConfig()
	app := AppModel{
		Config:      cfg,
		Tree:        model.NewTree(),
		Mode:        ModeNormal,
		WhichKey:    NewWhichKeyModel(),
		ConfigModal: NewConfigModal(cfg),
		QuickHelp:   NewQuickHelp(),
		TreeView:    NewTreeView(),
		StatusBar:   NewStatusBar(),
		HelpModal:   NewHelpModal(),
	}

	// 1. Press '<space>' -> WhichKey active
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{' '}})
	app = m.(AppModel)
	if !app.WhichKey.Active {
		t.Fatalf("expected WhichKey to be active after '<space>'")
	}

	// 2. Press 'c' -> WhichKey stays active with prefix [' ', 'c']
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'c'}})
	app = m.(AppModel)
	if !app.WhichKey.Active {
		t.Fatalf("expected WhichKey to stay active after '<space> c'")
	}

	// 3. Press 'c' -> Triggers Config Dashboard (ModeConfig)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'c'}})
	app = m.(AppModel)
	if app.Mode != ModeConfig {
		t.Fatalf("expected mode to be ModeConfig after '<space> c c', got %v", app.Mode)
	}
}

func TestCancelNewItemEscape(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	item1 := tree.InsertBelow("", "Initial Bullet")

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		TextInput: textinput.New(),
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}
	app.ensureValidCursor()
	app.SelectedID = item1.ID

	// 1. Create new item via 'oo' with 0 chars typed, press Esc -> should remove new item completely
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)

	if app.Mode != ModeInsert {
		t.Fatalf("expected ModeInsert after 'oo', got %v", app.Mode)
	}
	if len(app.Tree.Roots) != 2 {
		t.Fatalf("expected 2 roots after 'oo', got %d", len(app.Tree.Roots))
	}

	// Press Esc with 0 chars typed
	m, _ = app.updateInsert(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)
	if app.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after Esc, got %v", app.Mode)
	}
	if len(app.Tree.Roots) != 1 {
		t.Fatalf("expected new empty bullet to be removed after Esc (1 root left), got %d", len(app.Tree.Roots))
	}

	// 2. Create new item via 'oo' with 3 chars typed (<= 5), press Esc -> should remove new item
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	app.TextInput.SetValue("abc")

	m, _ = app.updateInsert(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)
	if app.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after Esc on <= 5 chars, got %v", app.Mode)
	}
	if len(app.Tree.Roots) != 1 {
		t.Fatalf("expected new bullet with 3 chars to be removed after Esc, got %d", len(app.Tree.Roots))
	}

	// 3. Create new item via 'oo' with 6 chars typed (> 5), press Esc -> prompt user to save
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	app.TextInput.SetValue("123456")

	m, _ = app.updateInsert(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)
	if app.Mode != ModePrompt || app.PromptType != PromptConfirmSaveNewItem {
		t.Fatalf("expected ModePrompt with PromptConfirmSaveNewItem after Esc on > 5 chars, got mode=%v prompt=%v", app.Mode, app.PromptType)
	}

	// Press 'y' to save
	m, _ = app.updatePrompt(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	app = m.(AppModel)
	if app.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after pressing 'y' in save prompt, got %v", app.Mode)
	}
	if len(app.Tree.Roots) != 2 {
		t.Fatalf("expected 2 roots after saving item, got %d", len(app.Tree.Roots))
	}
	savedItem := app.Tree.Roots[1]
	if savedItem.Text != "123456" {
		t.Fatalf("expected saved item text to be '123456', got %q", savedItem.Text)
	}

	// 4. Create new item with > 5 chars, press Esc, press 'n' to discard
	app.SelectedID = item1.ID
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)
	app.TextInput.SetValue("Discard me please")

	m, _ = app.updateInsert(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)
	if app.Mode != ModePrompt || app.PromptType != PromptConfirmSaveNewItem {
		t.Fatalf("expected prompt mode on > 5 chars Esc")
	}

	// Press 'n' to discard
	m, _ = app.updatePrompt(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	app = m.(AppModel)
	if app.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after 'n'")
	}
	if len(app.Tree.Roots) != 2 { // should still be 2 roots (initial + saved item)
		t.Fatalf("expected discarded item to be removed (2 roots), got %d", len(app.Tree.Roots))
	}
}

func TestExistingItemEscapeNotDeleted(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	item := tree.InsertBelow("", "Existing Item Text")

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		TextInput: textinput.New(),
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
	}
	app.ensureValidCursor()
	app.SelectedID = item.ID

	// Press 'e' to edit existing item
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'e'}})
	app = m.(AppModel)
	if app.Mode != ModeInsert {
		t.Fatalf("expected ModeInsert after 'e'")
	}

	// Press Esc -> should NOT delete existing item
	m, _ = app.updateInsert(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)
	if app.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after Esc")
	}
	if len(app.Tree.Roots) != 1 {
		t.Fatalf("expected existing item to remain in tree, got %d roots", len(app.Tree.Roots))
	}
	if app.Tree.Roots[0].Text != "Existing Item Text" {
		t.Fatalf("expected existing item text to be intact, got %q", app.Tree.Roots[0].Text)
	}
}

func TestTitleBarVersionAndSpace(t *testing.T) {
	app := AppModel{
		Version: "0.0.6",
		Width:   80,
	}

	// 1. Standard width without update -> title bar includes current version v0.0.6
	renderedBar := app.renderTitleBar()
	if !strings.Contains(renderedBar, "v0.0.6") {
		t.Fatalf("expected title bar to contain current version 'v0.0.6', got %q", renderedBar)
	}

	// 2. Wide terminal width (80) with update available -> includes current version AND update version
	app.UpdateAvailable = true
	app.UpdateInfo = &updater.ReleaseInfo{Version: "0.0.7"}

	renderedBarWithUpdate := app.renderTitleBar()
	if !strings.Contains(renderedBarWithUpdate, "v0.0.6") {
		t.Fatalf("expected title bar to contain current version 'v0.0.6', got %q", renderedBarWithUpdate)
	}
	if !strings.Contains(renderedBarWithUpdate, "v0.0.7") {
		t.Fatalf("expected title bar to contain update version 'v0.0.7' when space is available, got %q", renderedBarWithUpdate)
	}

	// 3. Narrow terminal width (48) with update available -> space is limited, so update version is omitted to fit available space
	app.Width = 48
	renderedNarrow := app.renderTitleBar()
	if !strings.Contains(renderedNarrow, "v0.0.6") {
		t.Fatalf("expected title bar to retain current version 'v0.0.6' under narrow width, got %q", renderedNarrow)
	}
	if strings.Contains(renderedNarrow, "v0.0.7") {
		t.Fatalf("expected title bar to omit update version 'v0.0.7' when space is insufficient, got %q", renderedNarrow)
	}
}
func TestJumpToID(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	r1 := tree.InsertBelow("", "First Root") // ID: 1
	r2 := tree.InsertBelow(r1.ID, "Second Root") // ID: 2
	c1 := tree.AddChild(r2.ID, "Child Item") // ID: 3
	c2 := tree.AddChild(r2.ID, "Target Nested Item") // ID: 4
	_ = c1

	r2.Folded = true // Fold parent

	ji := textinput.New()
	ji.Prompt = "🔢 Jump to ID: #"

	app := AppModel{
		Config:      cfg,
		Tree:        tree,
		Mode:        ModeNormal,
		JumpInput:   ji,
		WhichKey:    NewWhichKeyModel(),
		QuickHelp:   NewQuickHelp(),
		TreeView:    NewTreeView(),
		StatusBar:   NewStatusBar(),
		HelpModal:   NewHelpModal(),
	}
	app.ensureValidCursor()

	// 1. Trigger ModeJumpToID via 'gi'
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'g'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'i'}})
	app = m.(AppModel)

	if app.Mode != ModeJumpToID {
		t.Fatalf("expected ModeJumpToID after 'gi', got %v", app.Mode)
	}

	// 2. Submit ID "4" (target nested inside folded r2)
	app.JumpInput.SetValue("4")
	m, _ = app.updateJumpToID(tea.KeyMsg{Type: tea.KeyEnter})
	app = m.(AppModel)

	if app.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after submitting jump ID, got %v", app.Mode)
	}
	if r2.Folded {
		t.Fatalf("expected parent r2 to be unfolded after jumping to child")
	}
	if app.SelectedID != c2.ID {
		t.Fatalf("expected SelectedID to be %q, got %q", c2.ID, app.SelectedID)
	}
	if !strings.Contains(app.StatusMsg, "Jumped to item #4") {
		t.Fatalf("expected status message to confirm jump, got %q", app.StatusMsg)
	}

	// 3. Test jumping to non-existent ID
	app.Mode = ModeJumpToID
	app.JumpInput.SetValue("999")
	m, _ = app.updateJumpToID(tea.KeyMsg{Type: tea.KeyEnter})
	app = m.(AppModel)
	if !strings.Contains(app.StatusMsg, "Item ID #999 not found") {
		t.Fatalf("expected error status msg for missing ID, got %q", app.StatusMsg)
	}
}

func TestAppNoteModalIntegration(t *testing.T) {
	tree := model.NewTree()
	r1 := tree.InsertBelow("", "Task 1")
	r2 := tree.InsertBelow(r1.ID, "Task 2")

	cfg := config.DefaultConfig()
	app := AppModel{
		Config:      cfg,
		Tree:        tree,
		Mode:        ModeNormal,
		CursorIndex: 0,
		SelectedID:  r1.ID,
		TreeView:    NewTreeView(),
		NoteModal:   NewNoteModal(80, 24),
	}

	// 1. Press 'N' to open Note modal
	m, _ := app.updateNormal(tea.KeyMsg{Runes: []rune{'N'}, Type: tea.KeyRunes})
	app = m.(AppModel)

	if app.Mode != ModeNote {
		t.Fatalf("expected ModeNote after pressing 'N', got %v", app.Mode)
	}

	// Since r1.Note is empty, NoteModal starts in NoteModeEdit
	if app.NoteModal.Mode != NoteModeEdit {
		t.Fatalf("expected NoteModeEdit for empty note, got %v", app.NoteModal.Mode)
	}

	// 2. Type note referencing Task 2 (#r2.ID)
	noteText := "This note points to #" + r2.ID
	app.NoteModal.TextArea.SetValue(noteText)

	// Save note with Esc
	m, _ = app.Update(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)

	// Modal switches to NoteModeView and updates note on item r1
	if r1.Note != noteText {
		t.Fatalf("expected r1.Note to be %q, got %q", noteText, r1.Note)
	}

	if app.NoteModal.Mode != NoteModeView {
		t.Fatalf("expected NoteModeView after saving with Esc, got %v", app.NoteModal.Mode)
	}

	// 3. Press Enter to follow link to r2.ID
	m, _ = app.Update(tea.KeyMsg{Type: tea.KeyEnter})
	app = m.(AppModel)

	if app.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after following task link, got %v", app.Mode)
	}

	if app.SelectedID != r2.ID {
		t.Fatalf("expected jump to task ID %s, got %s", r2.ID, app.SelectedID)
	}

	if !strings.Contains(app.StatusMsg, "Jumped to item #"+r2.ID) {
		t.Fatalf("expected status msg confirming jump, got %q", app.StatusMsg)
	}
}

func TestFocusModeIntegration(t *testing.T) {
	cfg := config.DefaultConfig()
	tree := model.NewTree()
	r1 := tree.InsertBelow("", "Task One")
	r2 := tree.InsertBelow(r1.ID, "Task Two")

	app := AppModel{
		Config:    cfg,
		Tree:      tree,
		Mode:      ModeNormal,
		WhichKey:  NewWhichKeyModel(),
		QuickHelp: NewQuickHelp(),
		TreeView:  NewTreeView(),
		StatusBar: NewStatusBar(),
		HelpModal: NewHelpModal(),
		Width:     80,
		Height:    24,
	}
	app.ensureValidCursor()
	app.SelectedID = r1.ID

	// 1. Toggle focus on r1 via Leader '<space> t f'
	app.tryExecuteKeyBinding([]string{" ", "t", "f"})
	if !r1.IsFocused {
		t.Fatalf("expected r1 to be focused after '<space> t f'")
	}
	if !strings.Contains(app.StatusMsg, "Set current focus") {
		t.Fatalf("expected status msg confirming focus, got %q", app.StatusMsg)
	}

	// 2. Toggle focus on r2 via direct shortcut 'fo'
	app.SelectedID = r2.ID
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'f'}})
	app = m.(AppModel)
	m, _ = app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'o'}})
	app = m.(AppModel)

	if r1.IsFocused {
		t.Fatalf("expected r1 focus to be cleared when r2 is focused")
	}
	if !r2.IsFocused {
		t.Fatalf("expected r2 to be focused after 'fo'")
	}

	// 3. Focus banner is rendered in View() with multi-line note scaling
	r2.Note = "Line 1: Requirement A\nLine 2: Requirement B\nLine 3: Requirement C\nLine 4: Requirement D\nLine 5: Requirement E"
	app.Height = 30
	viewStr := app.View()
	if !strings.Contains(viewStr, "CURRENT FOCUS") || !strings.Contains(viewStr, "Task Two") || !strings.Contains(viewStr, "Line 5: Requirement E") {
		t.Fatalf("expected View output to contain Focus Banner with all multi-line notes rendered under height 30, got: %s", viewStr)
	}
	if !strings.Contains(viewStr, "EXIT focus mode") {
		t.Fatalf("expected Focus Banner hint to clearly state how to EXIT focus mode, got: %s", viewStr)
	}

	// 4. Toggle focus off on r2 via Leader '<space> f o'
	app.tryExecuteKeyBinding([]string{" ", "f", "o"})
	if r2.IsFocused {
		t.Fatalf("expected r2 focus to be cleared after '<space> f o'")
	}
}

func TestAppNoteAutoSaveOnEdit(t *testing.T) {
	tempDir := t.TempDir()
	filePath := filepath.Join(tempDir, "notes_test.pb")

	tree := model.NewTree()
	item := tree.InsertBelow("", "Note Item")
	storage := model.NewStorage(filePath, false)
	_ = storage.Save(tree, "")

	app := AppModel{
		Config:      config.DefaultConfig(),
		Storage:     storage,
		Tree:        tree,
		Mode:        ModeNormal,
		CursorIndex: 0,
		SelectedID:  item.ID,
		TreeView:    NewTreeView(),
		NoteModal:   NewNoteModal(80, 24),
	}

	// Open note modal
	m, _ := app.updateNormal(tea.KeyMsg{Runes: []rune{'N'}, Type: tea.KeyRunes})
	app = m.(AppModel)

	// Set note value and save via Esc
	app.NoteModal.TextArea.SetValue("Super solid note text")
	m, _ = app.Update(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)

	// Verify PendingAutoSave was set and file updated
	loadedTree, err := storage.Load("")
	if err != nil {
		t.Fatalf("failed to load storage file: %v", err)
	}

	if len(loadedTree.Roots) != 1 || loadedTree.Roots[0].Note != "Super solid note text" {
		t.Fatalf("expected loaded note to be 'Super solid note text', got %q", loadedTree.Roots[0].Note)
	}
}

func TestNoteModalQuitReturnsToNormalMode(t *testing.T) {
	app := AppModel{
		Config:    config.DefaultConfig(),
		Mode:      ModeNote,
		NoteModal: NewNoteModal(80, 24),
	}
	app.NoteModal.Mode = NoteModeView

	m, cmd := app.Update(tea.KeyMsg{Runes: []rune{'q'}, Type: tea.KeyRunes})
	res := m.(AppModel)

	if res.Mode != ModeNormal {
		t.Fatalf("expected ModeNormal after pressing 'q' in NoteModeView, got %v", res.Mode)
	}
	if cmd != nil {
		t.Fatalf("expected nil cmd (no quit), got %v", cmd)
	}
}

func TestConfigModalShowItemIDsLiveReflection(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.ShowItemIDs = true
	app := AppModel{
		Config:      cfg,
		Mode:        ModeConfig,
		ConfigModal: NewConfigModal(cfg),
		TreeView:    NewTreeView(),
	}
	app.TreeView.ShowItemIDs = true

	// Toggle show_item_ids in ConfigModal
	app.ConfigModal.SelectedIndex = 7 // show_item_ids item index
	app.ConfigModal.Update(tea.KeyMsg{Type: tea.KeySpace})

	// Close config modal
	m, _ := app.Update(tea.KeyMsg{Type: tea.KeyEsc})
	res := m.(AppModel)

	if res.TreeView.ShowItemIDs != res.Config.ShowItemIDs {
		t.Fatalf("expected TreeView.ShowItemIDs (%v) to match Config.ShowItemIDs (%v)", res.TreeView.ShowItemIDs, res.Config.ShowItemIDs)
	}
}

func TestEncryptionToggleConfigSync(t *testing.T) {
	tempDir := t.TempDir()
	filePath := filepath.Join(tempDir, "data.pb")
	cfg := config.DefaultConfig()
	cfg.Encrypted = false

	storage := model.NewStorage(filePath, false)
	archiveStore := model.NewArchiveStore(filepath.Join(tempDir, "archive.dat"), false)

	app := AppModel{
		Config:       cfg,
		Storage:      storage,
		ArchiveStore: archiveStore,
		ArchiveModal: NewArchiveModal(archiveStore),
		Tree:         model.NewTree(),
		WhichKey:     NewWhichKeyModel(),
		PromptInput:  textinput.New(),
	}

	// Toggle encryption via leader shortcut '<space> e e'
	executed, _ := app.tryExecuteKeyBinding([]string{" ", "e", "e"})
	if !executed {
		t.Fatalf("expected '<space> e e' to be executed")
	}

	if !app.Config.Encrypted {
		t.Fatalf("expected Config.Encrypted to be true after toggling encryption")
	}
	if app.Mode != ModePrompt || app.PromptType != PromptPassphraseSet {
		t.Fatalf("expected ModePrompt PromptPassphraseSet when passphrase is empty")
	}

	// Cancel passphrase prompt with Esc
	m, _ := app.Update(tea.KeyMsg{Type: tea.KeyEsc})
	res := m.(AppModel)

	if res.Storage.Encrypted {
		t.Fatalf("expected Storage.Encrypted to revert to false after cancelling prompt")
	}
	if res.Config.Encrypted {
		t.Fatalf("expected Config.Encrypted to revert to false after cancelling prompt")
	}
}

func TestExitFocusModeWithEscapeAndQ(t *testing.T) {
	tempDir := t.TempDir()
	filePath := filepath.Join(tempDir, "focus_exit_test.md")

	tree := model.NewTree()
	item := tree.InsertBelow("", "Focus Task")
	storage := model.NewStorage(filePath, false)
	_ = storage.Save(tree, "")

	app := AppModel{
		Config:      config.DefaultConfig(),
		Storage:     storage,
		Tree:        tree,
		Mode:        ModeNormal,
		CursorIndex: 0,
		SelectedID:  item.ID,
		TreeView:    NewTreeView(),
		WhichKey:    NewWhichKeyModel(),
	}

	// 1. Focus the item
	app.Tree.ToggleFocus(item.ID)
	if app.Tree.GetFocusedItem() == nil {
		t.Fatalf("expected item to be focused")
	}

	// 2. Press Esc to exit focus mode
	m, _ := app.updateNormal(tea.KeyMsg{Type: tea.KeyEsc})
	app = m.(AppModel)

	if app.Tree.GetFocusedItem() != nil {
		t.Fatalf("expected focus to be cleared after pressing Esc")
	}
	if app.StatusMsg != "Cleared current focus task" {
		t.Fatalf("expected StatusMsg 'Cleared current focus task', got %q", app.StatusMsg)
	}

	// 3. Focus the item again
	app.Tree.ToggleFocus(item.ID)
	if app.Tree.GetFocusedItem() == nil {
		t.Fatalf("expected item to be focused")
	}

	// 4. Press q to exit focus mode (should NOT quit app)
	m, cmd := app.updateNormal(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'q'}})
	if cmd != nil {
		t.Fatalf("expected cmd to be nil on exiting focus mode via 'q', got %v", cmd)
	}
	app = m.(AppModel)

	if app.Tree.GetFocusedItem() != nil {
		t.Fatalf("expected focus to be cleared after pressing 'q'")
	}
	if app.StatusMsg != "Cleared current focus task" {
		t.Fatalf("expected StatusMsg 'Cleared current focus task', got %q", app.StatusMsg)
	}
}


