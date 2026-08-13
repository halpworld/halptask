package ui

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/kenth/halptask/config"
	"github.com/kenth/halptask/model"
	"github.com/kenth/halptask/updater"
)

type configEditorClosedMsg struct {
	err error
}

func openConfigEditorCmd() tea.Cmd {
	editor := os.Getenv("EDITOR")
	if editor == "" {
		editor = os.Getenv("VISUAL")
	}
	if editor == "" {
		editor = "vim"
	}
	cfgPath := config.ConfigFilePath()
	c := exec.Command(editor, cfgPath)
	return tea.ExecProcess(c, func(err error) tea.Msg {
		return configEditorClosedMsg{err: err}
	})
}

type PromptType int

const (
	PromptPassphraseLoad PromptType = iota
	PromptPassphraseSet
	PromptConfirmSaveNewItem
)

type VersionCheckMsg struct {
	Info *updater.ReleaseInfo
	Err  error
}

type UpdateResultMsg struct {
	Version string
	Err     error
}

func checkUpdateCmd(version, repo string) tea.Cmd {
	return func() tea.Msg {
		rel, err := updater.CheckForUpdate(version, repo)
		return VersionCheckMsg{Info: rel, Err: err}
	}
}

func doUpdateCmd(rel *updater.ReleaseInfo) tea.Cmd {
	return func() tea.Msg {
		err := updater.DoUpdate(rel)
		return UpdateResultMsg{Version: rel.Version, Err: err}
	}
}

type AppModel struct {
	Config    *config.Config
	Storage   *model.Storage
	Tree      *model.Tree
	UndoStack []*model.Tree
	RedoStack []*model.Tree

	Mode         AppMode
	CursorIndex  int
	ScrollOffset int
	SelectedID   string

	TextInput   textinput.Model
	SearchInput textinput.Model
	JumpInput   textinput.Model
	PromptInput textinput.Model
	PromptType  PromptType

	WhichKey    WhichKeyModel
	QuickHelp   QuickHelp
	TreeView    TreeView
	StatusBar   StatusBar
	HelpModal   HelpModal
	TagModal    *TagModal
	ConfigModal *ConfigModal
	ArchiveStore *model.ArchiveStore
	ArchiveModal *ArchiveModal

	Passphrase string
	StatusMsg  string
	Width      int
	Height     int

	ZoomedID      string
	HideCompleted bool

	PendingAutoSave bool

	// Key sequence buffer for multi-keystroke commands (e.g. "gg", "dd", "zc")
	KeyBuffer string

	Version         string
	UpdateInfo      *updater.ReleaseInfo
	UpdateAvailable bool
	IsUpdating      bool

	EditingNewItem bool
	NewItemID      string
}

func (m *AppModel) getVisibleItems() []model.VisibleItem {
	return m.Tree.FlattenVisibleFiltered(m.ZoomedID, m.HideCompleted)
}

func InitialModel(cfg *config.Config, storage *model.Storage) (AppModel, tea.Cmd) {
	ti := textinput.New()
	ti.Prompt = "✏️  "
	if cfg != nil && cfg.DefaultItemType == "task" {
		ti.Placeholder = "Enter task text..."
	} else {
		ti.Placeholder = "Enter bullet text..."
	}
	ti.CharLimit = 256

	si := textinput.New()
	si.Prompt = "🔍 "
	si.Placeholder = "Search bullets..."

	ji := textinput.New()
	ji.Prompt = "🔢 Jump to ID: #"
	ji.Placeholder = "1"
	ji.CharLimit = 32

	pi := textinput.New()
	pi.Prompt = "🔑 Passphrase: "
	pi.EchoMode = textinput.EchoPassword
	pi.EchoCharacter = '•'

	showIDs := true
	if cfg != nil {
		showIDs = cfg.ShowItemIDs
	}

	tv := NewTreeView()
	tv.ShowItemIDs = showIDs

	archivePath := cfg.ArchiveFile
	if archivePath == "" {
		archivePath = config.DefaultConfig().ArchiveFile
	}
	archiveStore := model.NewArchiveStore(archivePath, storage.Encrypted)

	m := AppModel{
		Config:          cfg,
		Storage:         storage,
		ArchiveStore:    archiveStore,
		ArchiveModal:    NewArchiveModal(archiveStore),
		Tree:            model.NewTree(),
		UndoStack:       []*model.Tree{},
		RedoStack:       []*model.Tree{},
		Mode:            ModeNormal,
		CursorIndex:     0,
		ScrollOffset:    0,
		TextInput:       ti,
		SearchInput:     si,
		JumpInput:       ji,
		PromptInput:     pi,
		WhichKey:        NewWhichKeyModel(),
		QuickHelp:       NewQuickHelp(),
		TreeView:        tv,
		StatusBar:       NewStatusBar(),
		HelpModal:       NewHelpModal(),
		TagModal:        NewTagModal(cfg.Tags),
		ConfigModal:     NewConfigModal(cfg),
		Width:           80,
		Height:          24,
		Version:         "0.0.6",
		UpdateAvailable: false,
		IsUpdating:      false,
	}

	// Check if target file is encrypted
	isEncrypted, err := model.IsEncryptedFile(storage.FilePath)
	if err == nil && isEncrypted {
		m.Mode = ModePrompt
		m.PromptType = PromptPassphraseLoad
		m.PromptInput.Focus()
		return m, textinput.Blink
	}

	// Otherwise load plain text file
	tree, err := storage.Load("")
	if err == nil {
		m.Tree = tree
		m.ensureValidCursor()
	}

	return m, nil
}

func (m *AppModel) pushUndo() {
	m.UndoStack = append(m.UndoStack, m.Tree.Clone())
	m.RedoStack = nil // Clear redo stack on new change
	m.PendingAutoSave = true
}

func (m *AppModel) undo() {
	if len(m.UndoStack) == 0 {
		m.StatusMsg = "Already at oldest change"
		return
	}
	m.RedoStack = append(m.RedoStack, m.Tree.Clone())
	m.Tree = m.UndoStack[len(m.UndoStack)-1]
	m.UndoStack = m.UndoStack[:len(m.UndoStack)-1]
	m.ensureValidCursor()
	m.StatusMsg = "Undo"
	m.PendingAutoSave = true
}

func (m *AppModel) redo() {
	if len(m.RedoStack) == 0 {
		m.StatusMsg = "Already at newest change"
		return
	}
	m.UndoStack = append(m.UndoStack, m.Tree.Clone())
	m.Tree = m.RedoStack[len(m.RedoStack)-1]
	m.RedoStack = m.RedoStack[:len(m.RedoStack)-1]
	m.ensureValidCursor()
	m.StatusMsg = "Redo"
	m.PendingAutoSave = true
}

func (m *AppModel) ensureValidCursor() {
	visible := m.getVisibleItems()
	if len(visible) == 0 {
		m.CursorIndex = 0
		m.SelectedID = ""
		return
	}
	if m.CursorIndex < 0 {
		m.CursorIndex = 0
	}
	if m.CursorIndex >= len(visible) {
		m.CursorIndex = len(visible) - 1
	}
	m.SelectedID = visible[m.CursorIndex].Item.ID

	// Adjust scroll window
	maxVisibleLines := m.Height - 5 // reserved for header/quickhelp/status/whichkey
	if maxVisibleLines < 5 {
		maxVisibleLines = 5
	}
	if m.CursorIndex < m.ScrollOffset {
		m.ScrollOffset = m.CursorIndex
	} else if m.CursorIndex >= m.ScrollOffset+maxVisibleLines {
		m.ScrollOffset = m.CursorIndex - maxVisibleLines + 1
	}
}

func (m *AppModel) isDefaultTask() bool {
	if m.Config != nil && strings.ToLower(m.Config.DefaultItemType) == "task" {
		return true
	}
	return false
}

func (m *AppModel) applyDefaultItemType(item *model.Item) {
	if item == nil {
		return
	}
	if m.isDefaultTask() {
		item.IsTask = true
		item.Status = model.StatusTodo
	} else {
		item.IsTask = false
		item.Status = model.StatusNone
	}
}

func (m *AppModel) toggleDefaultItemType() {
	if m.Config == nil {
		m.Config = config.DefaultConfig()
	}
	if m.Config.DefaultItemType == "task" {
		m.Config.DefaultItemType = "bullet"
		m.StatusMsg = "Default item type: Bullet •"
	} else {
		m.Config.DefaultItemType = "task"
		m.StatusMsg = "Default item type: Task [ ]"
	}
	_ = config.SaveConfig(m.Config)
}

func (m *AppModel) prepareNewItemInsert(newItem *model.Item) {
	m.applyDefaultItemType(newItem)
	m.ensureValidCursor()
	visible := m.getVisibleItems()
	for i, v := range visible {
		if v.Item.ID == newItem.ID {
			m.CursorIndex = i
			break
		}
	}
	m.ensureValidCursor()
	m.Mode = ModeInsert
	m.EditingNewItem = true
	m.NewItemID = newItem.ID
	if m.isDefaultTask() {
		m.TextInput.Placeholder = "Enter task text..."
	} else {
		m.TextInput.Placeholder = "Enter bullet text..."
	}
	m.TextInput.SetValue("")
	m.TextInput.Focus()
}

func (m *AppModel) removeNewItem() {
	targetID := m.NewItemID
	if targetID == "" {
		targetID = m.SelectedID
	}
	if len(m.UndoStack) > 0 {
		m.Tree = m.UndoStack[len(m.UndoStack)-1]
		m.UndoStack = m.UndoStack[:len(m.UndoStack)-1]
	} else if targetID != "" {
		m.Tree.Delete(targetID)
	}
	m.EditingNewItem = false
	m.NewItemID = ""
	m.ensureValidCursor()
}

func (m *AppModel) restoreArchivedItem(item *model.Item) {
	if item == nil {
		return
	}
	m.pushUndo()
	restored := item.Clone()
	if m.SelectedID != "" {
		target := m.Tree.FindItem(m.SelectedID)
		if target != nil {
			if target.Parent == nil {
				idx := -1
				for i, r := range m.Tree.Roots {
					if r.ID == m.SelectedID {
						idx = i
						break
					}
				}
				if idx != -1 {
					m.Tree.Roots = append(m.Tree.Roots[:idx+1], append([]*model.Item{restored}, m.Tree.Roots[idx+1:]...)...)
				} else {
					m.Tree.Roots = append(m.Tree.Roots, restored)
				}
			} else {
				siblings := target.Parent.Children
				idx := -1
				for i, s := range siblings {
					if s.ID == m.SelectedID {
						idx = i
						break
					}
				}
				if idx != -1 {
					target.Parent.Children = append(siblings[:idx+1], append([]*model.Item{restored}, siblings[idx+1:]...)...)
				} else {
					target.Parent.Children = append(siblings, restored)
				}
			}
		} else {
			m.Tree.Roots = append(m.Tree.Roots, restored)
		}
	} else {
		m.Tree.Roots = append(m.Tree.Roots, restored)
	}
	m.Tree.EnsureIDs()
	m.Tree.SetParents()
	m.ensureValidCursor()
	m.PendingAutoSave = true
}

func (m AppModel) Init() tea.Cmd {
	if config.ShouldCheckForUpdate(m.Config) {
		return checkUpdateCmd(m.Version, m.Config.GithubRepo)
	}
	return nil
}

func (m AppModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd
	var newModel tea.Model = m

	switch msg := msg.(type) {
	case VersionCheckMsg:
		if m.Config != nil {
			m.Config.LastUpdateCheck = time.Now().Format(time.RFC3339)
			_ = config.SaveConfig(m.Config)
		}
		if msg.Err == nil && msg.Info != nil {
			m.UpdateInfo = msg.Info
			if msg.Info.NewRepo != "" && m.Config != nil && msg.Info.NewRepo != m.Config.GithubRepo {
				m.Config.GithubRepo = msg.Info.NewRepo
				_ = config.SaveConfig(m.Config)
			}
			if msg.Info.HasUpdate {
				m.UpdateAvailable = true
				m.StatusMsg = fmt.Sprintf("✨ New version v%s available! Press <space> U to update", msg.Info.Version)
			}
		}
		return m, nil

	case UpdateResultMsg:
		m.IsUpdating = false
		if msg.Err != nil {
			m.StatusMsg = fmt.Sprintf("Update failed: %v", msg.Err)
		} else {
			m.UpdateAvailable = false
			m.StatusMsg = fmt.Sprintf("🎉 Successfully updated halptask to v%s! Please restart.", msg.Version)
		}
		return m, nil

	case configEditorClosedMsg:
		cfg, err := config.LoadConfig()
		if err == nil {
			m.Config = cfg
			if m.ConfigModal != nil {
				m.ConfigModal.Config = cfg
				m.ConfigModal.RefreshItems()
			}
			m.StatusMsg = "Config reloaded from " + config.ConfigFilePath()
		} else {
			m.StatusMsg = "Error reloading config: " + err.Error()
		}
		return m, nil

	case tea.WindowSizeMsg:
		m.Width = msg.Width
		m.Height = msg.Height
		m.TreeView.Width = msg.Width
		m.TreeView.Height = msg.Height - 6
		m.WhichKey.Width = msg.Width
		m.QuickHelp.Width = msg.Width
		m.StatusBar.Width = msg.Width
		m.HelpModal.Width = msg.Width
		m.HelpModal.Height = msg.Height
		if m.ConfigModal != nil {
			m.ConfigModal.Width = msg.Width
			m.ConfigModal.Height = msg.Height
		}
		newModel = m

	case tea.KeyMsg:
		if msg.String() == "ctrl+c" {
			_ = m.saveFile()
			return m, tea.Quit
		}
		switch m.Mode {
		case ModeNormal:
			newModel, cmd = m.updateNormal(msg)
		case ModeInsert:
			newModel, cmd = m.updateInsert(msg)
		case ModeSearch:
			newModel, cmd = m.updateSearch(msg)
		case ModeJumpToID:
			newModel, cmd = m.updateJumpToID(msg)
		case ModePrompt:
			newModel, cmd = m.updatePrompt(msg)
		case ModeHelp:
			if msg.String() == "esc" || msg.String() == "?" || msg.String() == "q" {
				m.Mode = ModeNormal
			}
			newModel, cmd = m, nil
		case ModeTagPicker:
			closeModal, _ := m.TagModal.Update(msg)
			m.PendingAutoSave = true
			if closeModal {
				m.Mode = ModeNormal
				if m.Config != nil {
					m.Config.Tags = m.TagModal.TagConfigs
					_ = config.SaveConfig(m.Config)
				}
				_ = m.saveFile()
			}
			newModel, cmd = m, nil
		case ModeConfig:
			if m.ConfigModal == nil {
				m.ConfigModal = NewConfigModal(m.Config)
			}
			closeModal, statusMsg, openEditor := m.ConfigModal.Update(msg)
			if statusMsg != "" {
				m.StatusMsg = statusMsg
			}
			if openEditor {
				m.Mode = ModeNormal
				return m, openConfigEditorCmd()
			}
			if closeModal {
				m.Mode = ModeNormal
			}
			newModel, cmd = m, nil
		case ModeArchive:
			if m.ArchiveModal == nil {
				m.ArchiveModal = NewArchiveModal(m.ArchiveStore)
			}
			closeModal, restoredEntry, statusMsg := m.ArchiveModal.Update(msg)
			if statusMsg != "" {
				m.StatusMsg = statusMsg
			}
			if restoredEntry != nil && restoredEntry.Item != nil {
				m.restoreArchivedItem(restoredEntry.Item)
			}
			if closeModal {
				m.Mode = ModeNormal
			}
			newModel, cmd = m, nil
		}
	}

	if am, ok := newModel.(AppModel); ok {
		if am.PendingAutoSave {
			am.PendingAutoSave = false
			am.autoSave()
		}
		return am, cmd
	}

	return newModel, cmd
}

func (m AppModel) updateNormal(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	k := msg.String()

	// Clear status message on key press
	m.StatusMsg = ""

	// Handle WhichKey popup navigation
	isPrefixTriggerKey := k == " " || k == "o" || k == "d" || k == "z" || k == "g" || k == "w" || k == "f"
	if m.WhichKey.Active || isPrefixTriggerKey {
		if k == "esc" {
			m.WhichKey.Active = false
			m.WhichKey.PrefixKeys = nil
			m.KeyBuffer = ""
			return m, nil
		}

		if !m.WhichKey.Active && isPrefixTriggerKey {
			m.WhichKey.Active = true
			m.WhichKey.PrefixKeys = []string{k}
			m.KeyBuffer = k
			return m, nil
		}

		// Keystroke while WhichKey active
		m.WhichKey.PrefixKeys = append(m.WhichKey.PrefixKeys, k)

		actionExecuted, actionCmd := m.tryExecuteKeyBinding(m.WhichKey.PrefixKeys)
		if actionExecuted {
			m.WhichKey.Active = false
			m.WhichKey.PrefixKeys = nil
			m.KeyBuffer = ""
			return m, actionCmd
		}

		// Check double-character command matches (e.g. "gg", "dd", "zc", "zo", "za", "zM", "zR", "ww", "fc", "da", "ff", "oo", "oc")
		if len(m.WhichKey.PrefixKeys) == 2 && m.WhichKey.PrefixKeys[0] != " " {
			seq := m.WhichKey.PrefixKeys[0] + m.WhichKey.PrefixKeys[1]
			m.WhichKey.Active = false
			m.WhichKey.PrefixKeys = nil
			m.KeyBuffer = ""

			switch seq {
			case "gg":
				m.CursorIndex = 0
				m.ensureValidCursor()
				return m, nil
			case "gi":
				m.Mode = ModeJumpToID
				m.JumpInput.SetValue("")
				m.JumpInput.Focus()
				return m, textinput.Blink
			case "dd":
				if m.SelectedID != "" {
					m.pushUndo()
					nextID := m.Tree.Delete(m.SelectedID)
					m.ensureValidCursor()
					if nextID != "" {
						visible := m.getVisibleItems()
						for i, v := range visible {
							if v.Item.ID == nextID {
								m.CursorIndex = i
								break
							}
						}
					}
					m.ensureValidCursor()
					m.StatusMsg = "Deleted bullet"
				}
				return m, nil
			case "zc":
				if m.SelectedID != "" {
					m.Tree.Fold(m.SelectedID)
					m.ensureValidCursor()
				}
				return m, nil
			case "zo":
				if m.SelectedID != "" {
					m.Tree.Unfold(m.SelectedID)
					m.ensureValidCursor()
				}
				return m, nil
			case "za":
				if m.SelectedID != "" {
					m.Tree.ToggleFold(m.SelectedID)
					m.ensureValidCursor()
				}
				return m, nil
			case "zM":
				m.Tree.FoldAll()
				m.ensureValidCursor()
				return m, nil
			case "zR":
				m.Tree.UnfoldAll()
				m.ensureValidCursor()
				return m, nil
			case "ww":
				_ = m.saveFile()
				m.StatusMsg = "File saved"
				return m, nil
			case "fc":
				m.HideCompleted = !m.HideCompleted
				if m.HideCompleted {
					m.StatusMsg = "Hiding completed tasks [x]"
				} else {
					m.StatusMsg = "Showing all tasks"
				}
				m.ensureValidCursor()
				return m, nil
			case "da":
				m.pushUndo()
				count := m.Tree.DeleteCompleted()
				m.ensureValidCursor()
				m.StatusMsg = fmt.Sprintf("Cleared %d completed task(s)", count)
				return m, nil
			case "ff":
				if m.ZoomedID == "" && m.SelectedID != "" {
					m.ZoomedID = m.SelectedID
					item := m.Tree.FindItem(m.ZoomedID)
					if item != nil {
						m.StatusMsg = fmt.Sprintf("Zoomed in: %s", item.Text)
					}
				} else {
					m.ZoomedID = ""
					m.StatusMsg = "Unzoomed (full view)"
				}
				m.ensureValidCursor()
				return m, nil
			case "oo":
				m.pushUndo()
				newItem := m.Tree.InsertBelow(m.SelectedID, "")
				m.prepareNewItemInsert(newItem)
				return m, textinput.Blink
			case "oc":
				m.pushUndo()
				newItem := m.Tree.AddChild(m.SelectedID, "")
				m.prepareNewItemInsert(newItem)
				return m, textinput.Blink
			}
		}

		// Check if prefix keys still match any known command
		_, options := m.WhichKey.GetOptions(GetAllKeyBindings())
		if len(options) == 0 {
			m.WhichKey.Active = false
			m.WhichKey.PrefixKeys = nil
			m.KeyBuffer = ""
		}
		return m, nil
	}

	// Handle single or double keystrokes in normal mode
	m.KeyBuffer += k

	// Check multi-key direct matches (e.g. "gg", "dd", "zc", "zo", "za", "zM", "zR", "ts")
	switch m.KeyBuffer {
	case "gg":
		m.CursorIndex = 0
		m.ensureValidCursor()
		m.KeyBuffer = ""
		return m, nil
	case "dd":
		if m.SelectedID != "" {
			m.pushUndo()
			nextID := m.Tree.Delete(m.SelectedID)
			m.ensureValidCursor()
			if nextID != "" {
				visible := m.getVisibleItems()
				for i, v := range visible {
					if v.Item.ID == nextID {
						m.CursorIndex = i
						break
					}
				}
			}
			m.ensureValidCursor()
			m.StatusMsg = "Deleted bullet"
		}
		m.KeyBuffer = ""
		return m, nil
	case "zc":
		if m.SelectedID != "" {
			m.Tree.Fold(m.SelectedID)
			m.ensureValidCursor()
		}
		m.KeyBuffer = ""
		return m, nil
	case "zo":
		if m.SelectedID != "" {
			m.Tree.Unfold(m.SelectedID)
			m.ensureValidCursor()
		}
		m.KeyBuffer = ""
		return m, nil
	case "za":
		if m.SelectedID != "" {
			m.Tree.ToggleFold(m.SelectedID)
			m.ensureValidCursor()
		}
		m.KeyBuffer = ""
		return m, nil
	case "zM":
		m.Tree.FoldAll()
		m.ensureValidCursor()
		m.KeyBuffer = ""
		return m, nil
	case "zR":
		m.Tree.UnfoldAll()
		m.ensureValidCursor()
		m.KeyBuffer = ""
		return m, nil
	case "ww":
		_ = m.saveFile()
		m.StatusMsg = "File saved"
		m.KeyBuffer = ""
		return m, nil
	case "fc":
		m.HideCompleted = !m.HideCompleted
		if m.HideCompleted {
			m.StatusMsg = "Hiding completed tasks [x]"
		} else {
			m.StatusMsg = "Showing all tasks"
		}
		m.ensureValidCursor()
		m.KeyBuffer = ""
		return m, nil
	case "da":
		m.pushUndo()
		count := m.Tree.DeleteCompleted()
		m.ensureValidCursor()
		m.StatusMsg = fmt.Sprintf("Cleared %d completed task(s)", count)
		m.KeyBuffer = ""
		return m, nil
	case "ff":
		if m.ZoomedID == "" && m.SelectedID != "" {
			m.ZoomedID = m.SelectedID
			item := m.Tree.FindItem(m.ZoomedID)
			if item != nil {
				m.StatusMsg = fmt.Sprintf("Zoomed in: %s", item.Text)
			}
		} else {
			m.ZoomedID = ""
			m.StatusMsg = "Unzoomed (full view)"
		}
		m.ensureValidCursor()
		m.KeyBuffer = ""
		return m, nil
	case "oo":
		m.pushUndo()
		newItem := m.Tree.InsertBelow(m.SelectedID, "")
		m.prepareNewItemInsert(newItem)
		m.KeyBuffer = ""
		return m, textinput.Blink
	case "oc":
		m.pushUndo()
		newItem := m.Tree.AddChild(m.SelectedID, "")
		m.prepareNewItemInsert(newItem)
		m.KeyBuffer = ""
		return m, textinput.Blink
	}

	// Single key handlers
	switch k {
	case "j", "down":
		m.CursorIndex++
		m.ensureValidCursor()
		m.KeyBuffer = ""
	case "k", "up":
		m.CursorIndex--
		m.ensureValidCursor()
		m.KeyBuffer = ""
	case "h", "left":
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil && len(item.Children) > 0 && !item.Folded {
				m.Tree.Fold(m.SelectedID)
				m.ensureValidCursor()
			} else if item != nil && item.Parent != nil {
				// Jump to parent
				visible := m.getVisibleItems()
				for i, v := range visible {
					if v.Item.ID == item.Parent.ID {
						m.CursorIndex = i
						break
					}
				}
				m.ensureValidCursor()
			}
		}
		m.KeyBuffer = ""
	case "l", "right":
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil && len(item.Children) > 0 && item.Folded {
				m.Tree.Unfold(m.SelectedID)
				m.ensureValidCursor()
			} else if item != nil && len(item.Children) > 0 {
				// Jump to first child
				visible := m.getVisibleItems()
				for i, v := range visible {
					if v.Item.ID == item.Children[0].ID {
						m.CursorIndex = i
						break
					}
				}
				m.ensureValidCursor()
			}
		}
		m.KeyBuffer = ""
	case "G":
		visible := m.getVisibleItems()
		if len(visible) > 0 {
			m.CursorIndex = len(visible) - 1
			m.ensureValidCursor()
		}
		m.KeyBuffer = ""
	case "O":
		m.pushUndo()
		newItem := m.Tree.InsertAbove(m.SelectedID, "")
		m.prepareNewItemInsert(newItem)
		m.KeyBuffer = ""
		return m, textinput.Blink
	case "enter":
		if m.SelectedID != "" {
			m.Tree.ToggleFold(m.SelectedID)
			m.ensureValidCursor()
		}
		m.KeyBuffer = ""
	case "i", "a", "e":
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil {
				m.pushUndo()
				m.Mode = ModeInsert
				m.EditingNewItem = false
				m.NewItemID = ""
				if item.IsTask {
					m.TextInput.Placeholder = "Enter task text..."
				} else {
					m.TextInput.Placeholder = "Enter bullet text..."
				}
				m.TextInput.SetValue(item.Text)
				m.TextInput.Focus()
				m.KeyBuffer = ""
				return m, textinput.Blink
			}
		}
		m.KeyBuffer = ""
	case "c":
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil {
				m.pushUndo()
				m.Mode = ModeInsert
				m.EditingNewItem = false
				m.NewItemID = ""
				if item.IsTask {
					m.TextInput.Placeholder = "Enter task text..."
				} else {
					m.TextInput.Placeholder = "Enter bullet text..."
				}
				m.TextInput.SetValue("")
				m.TextInput.Focus()
				m.KeyBuffer = ""
				return m, textinput.Blink
			}
		}
		m.KeyBuffer = ""
	case "x":
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.Delete(m.SelectedID)
			m.ensureValidCursor()
			m.StatusMsg = "Deleted bullet"
		}
		m.KeyBuffer = ""
	case "t":
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.CycleStatus(m.SelectedID)
			m.ensureValidCursor()
		}
		m.KeyBuffer = ""
	case "tab":
		if m.SelectedID != "" {
			m.pushUndo()
			if m.Tree.Indent(m.SelectedID) {
				m.ensureValidCursor()
				m.StatusMsg = "Indented"
			}
		}
		m.KeyBuffer = ""
	case "shift+tab":
		if m.SelectedID != "" {
			m.pushUndo()
			if m.Tree.Unindent(m.SelectedID) {
				m.ensureValidCursor()
				m.StatusMsg = "Unindented"
			}
		}
		m.KeyBuffer = ""
	case "J":
		if m.SelectedID != "" {
			m.pushUndo()
			if m.Tree.MoveDown(m.SelectedID) {
				m.CursorIndex++
				m.ensureValidCursor()
			}
		}
		m.KeyBuffer = ""
	case "K":
		if m.SelectedID != "" {
			m.pushUndo()
			if m.Tree.MoveUp(m.SelectedID) {
				m.CursorIndex--
				m.ensureValidCursor()
			}
		}
		m.KeyBuffer = ""
	case "T":
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil {
				m.pushUndo()
				m.Mode = ModeTagPicker
				m.TagModal.SetItem(item, m.Tree)
				m.KeyBuffer = ""
				return m, nil
			}
		}
		m.KeyBuffer = ""
	case "u":
		m.undo()
		m.KeyBuffer = ""
	case "ctrl+r":
		m.redo()
		m.KeyBuffer = ""
	case "/":
		m.Mode = ModeSearch
		m.SearchInput.Focus()
		m.KeyBuffer = ""
		return m, textinput.Blink
	case "?":
		m.Mode = ModeHelp
		m.KeyBuffer = ""
	case "q":
		_ = m.saveFile()
		return m, tea.Quit
	default:
		// If key buffer has accumulated keys but no match yet, check length
		if len(m.KeyBuffer) > 2 {
			m.KeyBuffer = ""
		}
	}

	return m, nil
}

func (m *AppModel) tryExecuteKeyBinding(keys []string) (bool, tea.Cmd) {
	keySeq := strings.Join(keys, " ")

	switch keySeq {
	case "  b n": // Space > Bullets > New below
		m.pushUndo()
		newItem := m.Tree.InsertBelow(m.SelectedID, "")
		m.prepareNewItemInsert(newItem)
		return true, nil
	case "  b N": // Space > Bullets > New above
		m.pushUndo()
		newItem := m.Tree.InsertAbove(m.SelectedID, "")
		m.prepareNewItemInsert(newItem)
		return true, nil
	case "  b c": // Space > Bullets > Add child
		if m.SelectedID != "" {
			m.pushUndo()
			newItem := m.Tree.AddChild(m.SelectedID, "")
			m.prepareNewItemInsert(newItem)
			return true, nil
		}
	case "  b e": // Space > Bullets > Edit
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil {
				m.pushUndo()
				m.Mode = ModeInsert
				m.EditingNewItem = false
				m.NewItemID = ""
				if item.IsTask {
					m.TextInput.Placeholder = "Enter task text..."
				} else {
					m.TextInput.Placeholder = "Enter bullet text..."
				}
				m.TextInput.SetValue(item.Text)
				m.TextInput.Focus()
				return true, textinput.Blink
			}
		}
	case "  b d": // Space > Bullets > Delete
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.Delete(m.SelectedID)
			m.ensureValidCursor()
			m.StatusMsg = "Deleted bullet"
			return true, nil
		}
	case "  b i": // Indent
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.Indent(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  b o": // Unindent
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.Unindent(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  b j": // Move down
		if m.SelectedID != "" {
			m.pushUndo()
			if m.Tree.MoveDown(m.SelectedID) {
				m.CursorIndex++
				m.ensureValidCursor()
			}
			return true, nil
		}
	case "  b k": // Move up
		if m.SelectedID != "" {
			m.pushUndo()
			if m.Tree.MoveUp(m.SelectedID) {
				m.CursorIndex--
				m.ensureValidCursor()
			}
			return true, nil
		}
	case "  b t": // Toggle task
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.ToggleTask(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  t t": // Toggle task
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.ToggleTask(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  t c": // Cycle task status
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.CycleStatus(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  t d": // Mark Done [x]
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.SetStatus(m.SelectedID, model.StatusDone)
			m.ensureValidCursor()
			m.StatusMsg = "Marked done [x]"
			return true, nil
		}
	case "  t p": // Mark In Progress [~]
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.SetStatus(m.SelectedID, model.StatusInProgress)
			m.ensureValidCursor()
			m.StatusMsg = "Marked in-progress [~]"
			return true, nil
		}
	case "  t s": // Mark Todo [ ]
		if m.SelectedID != "" {
			m.pushUndo()
			m.Tree.SetStatus(m.SelectedID, model.StatusTodo)
			m.ensureValidCursor()
			m.StatusMsg = "Marked todo [ ]"
			return true, nil
		}
	case "  t a": // Manage tags / labels
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil {
				m.pushUndo()
				m.Mode = ModeTagPicker
				m.TagModal.SetItem(item, m.Tree)
				return true, nil
			}
		}
	case "  t D", "  b D", "  t m", "  b m": // Toggle default creation item type
		m.toggleDefaultItemType()
		return true, nil
	case "  c c": // Open Config Dashboard
		m.Mode = ModeConfig
		if m.ConfigModal == nil {
			m.ConfigModal = NewConfigModal(m.Config)
		}
		m.ConfigModal.RefreshItems()
		return true, nil
	case "  c a": // Toggle Auto-Save
		if m.Config == nil {
			m.Config = config.DefaultConfig()
		}
		m.Config.AutoSave = !m.Config.AutoSave
		_ = config.SaveConfig(m.Config)
		if m.Config.AutoSave {
			m.StatusMsg = "Auto-Save ENABLED"
		} else {
			m.StatusMsg = "Auto-Save DISABLED"
		}
		return true, nil
	case "  c d": // Toggle Default Item Type
		m.toggleDefaultItemType()
		return true, nil
	case "  d", "  c D": // Toggle Overview Dashboard
		if m.Config == nil {
			m.Config = config.DefaultConfig()
		}
		m.Config.ShowDashboard = !m.Config.ShowDashboard
		_ = config.SaveConfig(m.Config)
		if m.Config.ShowDashboard {
			m.StatusMsg = "Dashboard pane ENABLED"
		} else {
			m.StatusMsg = "Dashboard pane DISABLED"
		}
		return true, nil
	case "  c t": // Cycle Visual Theme
		if m.Config == nil {
			m.Config = config.DefaultConfig()
		}
		m.Config.Theme = config.CycleTheme(m.Config.Theme)
		_ = config.SaveConfig(m.Config)
		m.StatusMsg = fmt.Sprintf("Theme changed to %s", m.Config.Theme)
		return true, nil
	case "  c w": // Toggle WhichKey Popup
		if m.Config == nil {
			m.Config = config.DefaultConfig()
		}
		m.Config.ShowWhichKey = !m.Config.ShowWhichKey
		_ = config.SaveConfig(m.Config)
		if m.Config.ShowWhichKey {
			m.StatusMsg = "WhichKey Popup ENABLED"
		} else {
			m.StatusMsg = "WhichKey Popup DISABLED"
		}
		return true, nil
	case "  c e": // Open Config File in $EDITOR
		return true, openConfigEditorCmd()
	case "  z c":
		if m.SelectedID != "" {
			m.Tree.Fold(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  z o":
		if m.SelectedID != "" {
			m.Tree.Unfold(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  z a":
		if m.SelectedID != "" {
			m.Tree.ToggleFold(m.SelectedID)
			m.ensureValidCursor()
			return true, nil
		}
	case "  z M":
		m.Tree.FoldAll()
		m.ensureValidCursor()
		return true, nil
	case "  z R":
		m.Tree.UnfoldAll()
		m.ensureValidCursor()
		return true, nil
	case "  a a": // Archive selected item
		if m.SelectedID != "" {
			m.pushUndo()
			targetItem, parentPath := m.Tree.ArchiveItem(m.SelectedID)
			if targetItem != nil {
				entry := &model.ArchivedEntry{
					ID:         targetItem.ID,
					ArchivedAt: time.Now(),
					Context:    parentPath,
					Item:       targetItem,
				}
				entries, _ := m.ArchiveStore.Load(m.Passphrase)
				entries = append([]*model.ArchivedEntry{entry}, entries...)
				_ = m.ArchiveStore.Save(entries, m.Passphrase)
				_ = m.saveFile()
				m.ensureValidCursor()
				m.StatusMsg = fmt.Sprintf("Archived item #%s (%s)", targetItem.ID, targetItem.Text)
			}
			return true, nil
		}
	case "  a c": // Archive completed tasks
		m.pushUndo()
		newEntries := m.Tree.ArchiveCompleted()
		if len(newEntries) == 0 {
			m.StatusMsg = "No completed tasks to archive"
			return true, nil
		}
		entries, _ := m.ArchiveStore.Load(m.Passphrase)
		entries = append(newEntries, entries...)
		_ = m.ArchiveStore.Save(entries, m.Passphrase)
		_ = m.saveFile()
		m.ensureValidCursor()
		m.StatusMsg = fmt.Sprintf("Archived %d completed task(s)", len(newEntries))
		return true, nil
	case "  a v", "  a r": // View / restore archive
		entries, err := m.ArchiveStore.Load(m.Passphrase)
		if err != nil {
			m.StatusMsg = "Failed to load archive: " + err.Error()
			return true, nil
		}
		if m.ArchiveModal == nil {
			m.ArchiveModal = NewArchiveModal(m.ArchiveStore)
		}
		m.ArchiveModal.Open(entries, m.Passphrase)
		m.Mode = ModeArchive
		return true, nil
	case "  e e": // Toggle encryption
		m.Storage.Encrypted = !m.Storage.Encrypted
		m.ArchiveStore.Encrypted = m.Storage.Encrypted
		if m.Storage.Encrypted && m.Passphrase == "" {
			m.Mode = ModePrompt
			m.PromptType = PromptPassphraseSet
			m.PromptInput.SetValue("")
			m.PromptInput.Focus()
		} else {
			_ = m.saveFile()
			entries, err := m.ArchiveStore.Load(m.Passphrase)
			if err == nil {
				_ = m.ArchiveStore.Save(entries, m.Passphrase)
			}
			if m.Storage.Encrypted {
				m.StatusMsg = "Encryption ENABLED 🔒"
			} else {
				m.StatusMsg = "Encryption DISABLED 🔓"
			}
		}
		return true, nil
	case "  e p": // Change passphrase
		m.Mode = ModePrompt
		m.PromptType = PromptPassphraseSet
		m.PromptInput.SetValue("")
		m.PromptInput.Focus()
		return true, nil
	case "  w", "  s":
		_ = m.saveFile()
		return true, nil
	case "  g i", "  j i", "  g g i":
		m.Mode = ModeJumpToID
		m.JumpInput.SetValue("")
		m.JumpInput.Focus()
		return true, textinput.Blink
	case "  /":
		m.Mode = ModeSearch
		m.SearchInput.Focus()
		return true, nil
	case "  ?":
		m.Mode = ModeHelp
		return true, nil
	case "  U": // Check / install update
		if m.UpdateInfo != nil && m.UpdateInfo.HasUpdate {
			canUpdate, realPath, reason := updater.CanUpdate()
			if !canUpdate {
				m.StatusMsg = fmt.Sprintf("Cannot update binary at %s: %s", realPath, reason)
				return true, nil
			}
			m.IsUpdating = true
			m.StatusMsg = fmt.Sprintf("Updating halptask to v%s...", m.UpdateInfo.Version)
			return true, doUpdateCmd(m.UpdateInfo)
		}
		m.StatusMsg = "Checking for updates..."
		repo := "arkalon76/halptask"
		if m.Config != nil && m.Config.GithubRepo != "" {
			repo = m.Config.GithubRepo
		}
		return true, checkUpdateCmd(m.Version, repo)
	case "  q":
		_ = m.saveFile()
		// Return true so updating stops, quit will be dispatched
		return true, nil
	}

	return false, nil
}

func (m AppModel) updateInsert(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd
	switch msg.String() {
	case "enter":
		text := strings.TrimSpace(m.TextInput.Value())
		if m.SelectedID != "" {
			item := m.Tree.FindItem(m.SelectedID)
			if item != nil && item.Text != text {
				item.Text = text
				m.PendingAutoSave = true
			}
		}
		m.EditingNewItem = false
		m.NewItemID = ""
		m.Mode = ModeNormal
		m.TextInput.Blur()
		return m, nil
	case "esc":
		if m.EditingNewItem {
			typedCount := utf8.RuneCountInString(m.TextInput.Value())
			if typedCount > 5 {
				m.Mode = ModePrompt
				m.PromptType = PromptConfirmSaveNewItem
				return m, nil
			}
			m.removeNewItem()
			m.Mode = ModeNormal
			m.TextInput.Blur()
			return m, nil
		}
		m.Mode = ModeNormal
		m.TextInput.Blur()
		return m, nil
	}

	m.TextInput, cmd = m.TextInput.Update(msg)
	return m, cmd
}

func (m AppModel) updateSearch(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd
	switch msg.String() {
	case "esc", "enter":
		m.Mode = ModeNormal
		m.SearchInput.Blur()
		return m, nil
	}

	m.SearchInput, cmd = m.SearchInput.Update(msg)
	query := m.SearchInput.Value()
	m.TreeView.SearchQuery = query

	if query != "" {
		matchedIDs := m.Tree.Search(query)
		m.TreeView.MatchedIDs = make(map[string]bool)
		for _, id := range matchedIDs {
			m.TreeView.MatchedIDs[id] = true
		}
		if len(matchedIDs) > 0 {
			visible := m.getVisibleItems()
			for i, v := range visible {
				if v.Item.ID == matchedIDs[0] {
					m.CursorIndex = i
					break
				}
			}
			m.ensureValidCursor()
		}
	} else {
		m.TreeView.MatchedIDs = make(map[string]bool)
	}

	return m, cmd
}

func (m AppModel) updateJumpToID(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd
	switch msg.String() {
	case "esc":
		m.Mode = ModeNormal
		m.JumpInput.Blur()
		return m, nil
	case "enter":
		targetID := strings.TrimSpace(m.JumpInput.Value())
		targetID = strings.TrimPrefix(targetID, "#")
		m.JumpInput.Blur()
		if targetID == "" {
			m.Mode = ModeNormal
			return m, nil
		}
		item := m.Tree.FindItem(targetID)
		if item != nil {
			curr := item.Parent
			for curr != nil {
				curr.Folded = false
				curr = curr.Parent
			}
			visible := m.getVisibleItems()
			found := false
			for i, v := range visible {
				if v.Item.ID == targetID {
					m.CursorIndex = i
					found = true
					break
				}
			}
			m.ensureValidCursor()
			if found {
				m.StatusMsg = fmt.Sprintf("Jumped to item #%s", targetID)
			} else {
				m.StatusMsg = fmt.Sprintf("Item #%s is not visible", targetID)
			}
		} else {
			m.StatusMsg = fmt.Sprintf("Item ID #%s not found", targetID)
		}
		m.Mode = ModeNormal
		return m, nil
	default:
		m.JumpInput, cmd = m.JumpInput.Update(msg)
		return m, cmd
	}
}

func (m AppModel) updatePrompt(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	if m.PromptType == PromptConfirmSaveNewItem {
		switch msg.String() {
		case "y", "Y", "enter":
			text := strings.TrimSpace(m.TextInput.Value())
			itemTypeStr := "bullet"
			if m.SelectedID != "" {
				item := m.Tree.FindItem(m.SelectedID)
				if item != nil {
					item.Text = text
					if item.IsTask {
						itemTypeStr = "task"
					}
					m.PendingAutoSave = true
				}
			}
			m.EditingNewItem = false
			m.NewItemID = ""
			m.Mode = ModeNormal
			m.TextInput.Blur()
			m.ensureValidCursor()
			m.StatusMsg = fmt.Sprintf("Saved %s", itemTypeStr)
			return m, nil
		case "n", "N":
			itemTypeStr := "bullet"
			if m.SelectedID != "" {
				if item := m.Tree.FindItem(m.SelectedID); item != nil && item.IsTask {
					itemTypeStr = "task"
				}
			}
			m.removeNewItem()
			m.Mode = ModeNormal
			m.TextInput.Blur()
			m.ensureValidCursor()
			m.StatusMsg = fmt.Sprintf("Discarded %s", itemTypeStr)
			return m, nil
		case "esc":
			m.Mode = ModeInsert
			m.TextInput.Focus()
			return m, textinput.Blink
		default:
			return m, nil
		}
	}

	var cmd tea.Cmd
	switch msg.String() {
	case "enter":
		val := m.PromptInput.Value()
		if m.PromptType == PromptPassphraseLoad {
			tree, err := m.Storage.Load(val)
			if err != nil {
				m.StatusMsg = "Incorrect passphrase! Try again."
				m.PromptInput.SetValue("")
				return m, textinput.Blink
			}
			m.Tree = tree
			m.Passphrase = val
			m.Storage.Encrypted = true
			m.Mode = ModeNormal
			m.ensureValidCursor()
			m.StatusMsg = "Decrypted & loaded successfully 🔓"
			return m, nil
		} else if m.PromptType == PromptPassphraseSet {
			m.Passphrase = val
			m.Storage.Encrypted = true
			m.Mode = ModeNormal
			_ = m.saveFile()
			m.StatusMsg = "Passphrase set & encrypted 🔒"
			return m, nil
		}
	case "esc":
		if m.PromptType == PromptPassphraseLoad {
			// Quit if user cancels load prompt
			return m, tea.Quit
		}
		m.Mode = ModeNormal
		return m, nil
	}

	m.PromptInput, cmd = m.PromptInput.Update(msg)
	return m, cmd
}

func (m *AppModel) autoSave() {
	if !m.Config.AutoSave {
		return
	}
	if m.Storage.Encrypted && m.Passphrase == "" {
		m.StatusMsg = "Auto-save paused: passphrase required"
		return
	}
	err := m.Storage.Save(m.Tree, m.Passphrase)
	if err != nil {
		m.StatusMsg = "Auto-save error: " + err.Error()
	}
}

func (m *AppModel) saveFile() error {
	err := m.Storage.Save(m.Tree, m.Passphrase)
	if err != nil {
		m.StatusMsg = "Save error: " + err.Error()
		return err
	}
	m.StatusMsg = "Saved " + m.Storage.FilePath
	return nil
}

func (m AppModel) renderTitleBar() string {
	ver := m.Version
	if ver != "" && !strings.HasPrefix(ver, "v") {
		ver = "v" + ver
	}

	var candidates []string

	if m.UpdateAvailable && m.UpdateInfo != nil && m.UpdateInfo.Version != "" {
		latest := m.UpdateInfo.Version
		if !strings.HasPrefix(latest, "v") {
			latest = "v" + latest
		}
		candidates = append(candidates,
			fmt.Sprintf(" HALPTASK %s (%s available)  •  Bullet & Task Manager ", ver, latest),
			fmt.Sprintf(" HALPTASK %s (update: %s)  •  Bullet & Task Manager ", ver, latest),
			fmt.Sprintf(" HALPTASK %s  •  Bullet & Task Manager ", ver),
			fmt.Sprintf(" HALPTASK %s ", ver),
			" HALPTASK ",
		)
	} else if ver != "" {
		candidates = append(candidates,
			fmt.Sprintf(" HALPTASK %s  •  Bullet & Task Manager ", ver),
			fmt.Sprintf(" HALPTASK %s ", ver),
			" HALPTASK ",
		)
	} else {
		candidates = append(candidates,
			" HALPTASK  •  Bullet & Task Manager ",
			" HALPTASK ",
		)
	}

	titleText := candidates[len(candidates)-1]
	for _, cand := range candidates {
		if lipgloss.Width(cand) <= m.Width {
			titleText = cand
			break
		}
	}

	headerStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#1a1b26")).
		Background(lipgloss.Color("#7aa2f7")).
		Width(m.Width).
		Align(lipgloss.Center)

	return headerStyle.Render(titleText)
}

func (m AppModel) View() string {
	if m.Width == 0 || m.Height == 0 {
		return "Loading HalpTask..."
	}

	if m.Mode == ModePrompt {
		if m.PromptType == PromptConfirmSaveNewItem {
			promptStyle := lipgloss.NewStyle().
				Border(lipgloss.DoubleBorder()).
				BorderForeground(lipgloss.Color("#e0af68")).
				Padding(1, 3).
				Width(56)

			itemTypeStr := "bullet"
			if m.SelectedID != "" {
				if item := m.Tree.FindItem(m.SelectedID); item != nil && item.IsTask {
					itemTypeStr = "task"
				}
			}

			title := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#7aa2f7")).Render(fmt.Sprintf("❓ Save new %s?", itemTypeStr))

			previewText := m.TextInput.Value()
			if utf8.RuneCountInString(previewText) > 35 {
				runes := []rune(previewText)
				previewText = string(runes[:32]) + "..."
			}
			textPreview := lipgloss.NewStyle().Italic(true).Foreground(lipgloss.Color("#c0caf5")).Render(fmt.Sprintf("%q", previewText))

			msg := fmt.Sprintf("Do you want to save this %s?\n\nItem: %s", itemTypeStr, textPreview)

			help := lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("[y/Enter] Save   [n] Discard   [Esc] Cancel")

			content := fmt.Sprintf("%s\n\n%s\n\n%s", title, msg, help)
			return lipgloss.Place(m.Width, m.Height, lipgloss.Center, lipgloss.Center, promptStyle.Render(content))
		}

		promptStyle := lipgloss.NewStyle().
			Border(lipgloss.DoubleBorder()).
			BorderForeground(lipgloss.Color("#f7768e")).
			Padding(1, 3).
			Width(60)

		title := "🔒 Encrypted File Passphrase"
		if m.PromptType == PromptPassphraseSet {
			title = "🔐 Set Encryption Passphrase"
		}

		content := fmt.Sprintf("%s\n\n%s", lipgloss.NewStyle().Bold(true).Render(title), m.PromptInput.View())
		if m.StatusMsg != "" {
			content += "\n\n" + lipgloss.NewStyle().Foreground(lipgloss.Color("#f7768e")).Render(m.StatusMsg)
		}
		return lipgloss.Place(m.Width, m.Height, lipgloss.Center, lipgloss.Center, promptStyle.Render(content))
	}

	if m.Mode == ModeHelp {
		return lipgloss.Place(m.Width, m.Height, lipgloss.Center, lipgloss.Center, m.HelpModal.Render(m.Width, m.Height))
	}

	if m.Mode == ModeTagPicker {
		return lipgloss.Place(m.Width, m.Height, lipgloss.Center, lipgloss.Center, m.TagModal.Render(m.Width, m.Height))
	}

	if m.Mode == ModeConfig {
		if m.ConfigModal == nil {
			m.ConfigModal = NewConfigModal(m.Config)
		}
		return lipgloss.Place(m.Width, m.Height, lipgloss.Center, lipgloss.Center, m.ConfigModal.Render(m.Width, m.Height))
	}

	if m.Mode == ModeArchive {
		if m.ArchiveModal == nil {
			m.ArchiveModal = NewArchiveModal(m.ArchiveStore)
		}
		return lipgloss.Place(m.Width, m.Height, lipgloss.Center, lipgloss.Center, m.ArchiveModal.Render(m.Width, m.Height))
	}

	// Normal View Layout: Header / Tree View (+ Dashboard) / Text Input / WhichKey / Status Bar
	header := m.renderTitleBar()

	m.TreeView.Tree = m.Tree
	if m.Config != nil {
		m.TreeView.TagConfigs = m.Config.Tags
	}

	visible := m.getVisibleItems()

	// Calculate content height reserved for TreeView & Dashboard
	reservedHeight := 3 // Header(1) + QuickHelp(1) + StatusBar(1)
	if m.Mode == ModeInsert || m.Mode == ModeSearch {
		reservedHeight += 3
	}
	if m.WhichKey.Active {
		wkStr := m.WhichKey.Render(GetAllKeyBindings(), m.Width)
		reservedHeight += strings.Count(wkStr, "\n") + 1
	}

	contentHeight := m.Height - reservedHeight
	if contentHeight < 5 {
		contentHeight = 5
	}

	var mainArea string
	if m.Config != nil && m.Config.ShowDashboard && m.Width >= 50 {
		dashWidth := int(float64(m.Width) * 0.35)
		if dashWidth > 42 {
			dashWidth = 42
		}
		if dashWidth < 26 {
			dashWidth = 26
		}
		treeWidth := m.Width - dashWidth

		treeInnerWidth := treeWidth - 2
		treeInnerHeight := contentHeight - 2
		if treeInnerWidth < 10 {
			treeInnerWidth = 10
		}
		if treeInnerHeight < 1 {
			treeInnerHeight = 1
		}

		m.TreeView.Width = treeInnerWidth
		m.TreeView.Height = treeInnerHeight
		rawTreeContent := m.TreeView.Render(visible, m.CursorIndex, m.ScrollOffset)

		treePanelStyle := lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#3b4261")).
			Width(treeInnerWidth).
			Height(treeInnerHeight)

		treeContent := treePanelStyle.Render(rawTreeContent)

		dbView := NewDashboardView()
		dbView.Width = dashWidth
		dbView.Height = contentHeight
		dashContent := dbView.Render(m.Tree, m.Config.Tags)

		mainArea = lipgloss.JoinHorizontal(lipgloss.Top, treeContent, dashContent)
	} else {
		m.TreeView.Width = m.Width
		m.TreeView.Height = contentHeight
		mainArea = m.TreeView.Render(visible, m.CursorIndex, m.ScrollOffset)
	}

	if m.Config != nil {
		m.TreeView.ShowItemIDs = m.Config.ShowItemIDs
	}

	var midSection string
	if m.Mode == ModeInsert {
		insertBox := lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#9ece6a")).
			Padding(0, 1).
			Width(m.Width - 4).
			Render(m.TextInput.View())
		midSection = insertBox
	} else if m.Mode == ModeSearch {
		searchBox := lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#e0af68")).
			Padding(0, 1).
			Width(m.Width - 4).
			Render(m.SearchInput.View())
		midSection = searchBox
	} else if m.Mode == ModeJumpToID {
		jumpBox := lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#7aa2f7")).
			Padding(0, 1).
			Width(m.Width - 4).
			Render(m.JumpInput.View())
		midSection = jumpBox
	}

	var whichKeyStr string
	if m.WhichKey.Active {
		whichKeyStr = m.WhichKey.Render(GetAllKeyBindings(), m.Width)
	}

	stats := m.Tree.GetStats()
	filePath := ""
	isEncrypted := false
	if m.Storage != nil {
		filePath = m.Storage.FilePath
		isEncrypted = m.Storage.Encrypted
	}
	updateBadge := ""
	if m.UpdateAvailable && m.UpdateInfo != nil {
		updateBadge = fmt.Sprintf("✨ v%s available", m.UpdateInfo.Version)
	}
	statusBarStr := m.StatusBar.Render(m.Mode, filePath, isEncrypted, stats, m.CursorIndex+1, len(visible), m.StatusMsg, updateBadge)

	var viewParts []string
	viewParts = append(viewParts, header)
	viewParts = append(viewParts, mainArea)

	if midSection != "" {
		viewParts = append(viewParts, midSection)
	}
	if whichKeyStr != "" {
		viewParts = append(viewParts, whichKeyStr)
	}
	viewParts = append(viewParts, m.QuickHelp.Render(m.Width))
	viewParts = append(viewParts, statusBarStr)

	return strings.Join(viewParts, "\n")
}
