package ui

import (
	"fmt"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/kenth/halptask/config"
)

type ConfigItemType int

const (
	ConfigItemBool ConfigItemType = iota
	ConfigItemEnum
	ConfigItemReadOnly
	ConfigItemAction
)

type ConfigItem struct {
	Category    string
	Key         string
	Label       string
	Type        ConfigItemType
	Value       string
	Options     []string
	Description string
}

type ConfigModal struct {
	Width         int
	Height        int
	SelectedIndex int
	Config        *config.Config
	Items         []ConfigItem
}

func NewConfigModal(cfg *config.Config) *ConfigModal {
	cm := &ConfigModal{
		Width:         80,
		Height:        24,
		SelectedIndex: 0,
		Config:        cfg,
	}
	cm.RefreshItems()
	return cm
}

func (cm *ConfigModal) RefreshItems() {
	if cm.Config == nil {
		cm.Config = config.DefaultConfig()
	}

	autoSaveVal := "[ ] Disabled"
	if cm.Config.AutoSave {
		autoSaveVal = "[✓] Enabled"
	}

	checkUpdatesVal := "[ ] Disabled"
	if cm.Config.CheckUpdates {
		checkUpdatesVal = "[✓] Enabled"
	}

	updateIntervalVal := cm.Config.UpdateInterval
	if updateIntervalVal == "" {
		updateIntervalVal = "daily"
	}

	showWhichKeyVal := "[ ] Disabled"
	if cm.Config.ShowWhichKey {
		showWhichKeyVal = "[✓] Enabled"
	}

	showDashboardVal := "[ ] Disabled"
	if cm.Config.ShowDashboard {
		showDashboardVal = "[✓] Enabled"
	}

	showItemIDsVal := "[ ] Disabled"
	if cm.Config.ShowItemIDs {
		showItemIDsVal = "[✓] Enabled"
	}

	encryptedVal := "[ ] Disabled 🔓"
	if cm.Config.Encrypted {
		encryptedVal = "[✓] Enabled 🔒"
	}

	cm.Items = []ConfigItem{
		// General
		{Category: "General", Key: "auto_save", Label: "Auto-Save Changes", Type: ConfigItemBool, Value: autoSaveVal, Description: "Automatically save changes to data file on edits"},
		{Category: "General", Key: "default_item_type", Label: "Default Item Type", Type: ConfigItemEnum, Value: cm.Config.DefaultItemType, Options: []string{"bullet", "task"}, Description: "Default type ('bullet' or 'task') for new nodes"},
		{Category: "General", Key: "check_updates", Label: "Check for Updates", Type: ConfigItemBool, Value: checkUpdatesVal, Description: "Check GitHub releases for updates"},
		{Category: "General", Key: "update_interval", Label: "Update Frequency", Type: ConfigItemEnum, Value: updateIntervalVal, Options: []string{"daily", "always", "weekly", "never"}, Description: "Frequency to check for updates (daily, always, weekly, never)"},

		// UI & Theme
		{Category: "UI & Appearance", Key: "theme", Label: "Color Theme", Type: ConfigItemEnum, Value: cm.Config.Theme, Options: config.AvailableThemes, Description: "Active visual theme palette"},
		{Category: "UI & Appearance", Key: "show_which_key", Label: "WhichKey Popup", Type: ConfigItemBool, Value: showWhichKeyVal, Description: "Display WhichKey popup menu on leader key"},
		{Category: "UI & Appearance", Key: "show_dashboard", Label: "Show Dashboard Pane", Type: ConfigItemBool, Value: showDashboardVal, Description: "Display side overview dashboard panel (toggle with <space> d)"},
		{Category: "UI & Appearance", Key: "show_item_ids", Label: "Show Task/Bullet Numbers", Type: ConfigItemBool, Value: showItemIDsVal, Description: "Display permanent ID numbers next to task/bullet items"},
		{Category: "UI & Appearance", Key: "indent_spaces", Label: "Indentation Spaces", Type: ConfigItemEnum, Value: fmt.Sprintf("%d", cm.Config.IndentSpaces), Options: []string{"2", "4", "8"}, Description: "Number of spaces per indentation level"},

		// Storage & Security
		{Category: "Storage & Security", Key: "encrypted", Label: "File Encryption", Type: ConfigItemReadOnly, Value: encryptedVal, Description: "AES-256-GCM encryption status (toggle with <space> e e)"},
		{Category: "Storage & Security", Key: "data_file", Label: "Data File Path", Type: ConfigItemReadOnly, Value: cm.Config.DataFile, Description: "Path to Markdown data file"},
		{Category: "Storage & Security", Key: "github_repo", Label: "GitHub Repository", Type: ConfigItemReadOnly, Value: cm.Config.GithubRepo, Description: "Repository for update checks"},
		{Category: "Storage & Security", Key: "open_editor", Label: "Edit Config File", Type: ConfigItemAction, Value: "Press 'e' for $EDITOR", Description: "Launch external editor (~/.config/halptask/config.yaml)"},
	}
}

// Update handles keypresses in the ConfigModal.
// Returns (closeModal, statusMsg, openExternalEditor).
func (cm *ConfigModal) Update(msg tea.Msg) (bool, string, bool) {
	if len(cm.Items) == 0 {
		cm.RefreshItems()
	}

	switch msg := msg.(type) {
	case tea.KeyMsg:
		k := msg.String()
		switch k {
		case "j", "down":
			if len(cm.Items) > 0 {
				cm.SelectedIndex = (cm.SelectedIndex + 1) % len(cm.Items)
			}
			return false, "", false
		case "k", "up":
			if len(cm.Items) > 0 {
				cm.SelectedIndex = (cm.SelectedIndex - 1 + len(cm.Items)) % len(cm.Items)
			}
			return false, "", false
		case "e":
			return false, "", true // Signal to launch external editor
		case "esc", "q":
			return true, "", false // Close modal
		case "enter", " ", "l", "right":
			return cm.activateItem(cm.SelectedIndex, 1)
		case "h", "left":
			return cm.activateItem(cm.SelectedIndex, -1)
		default:
			// Number keys 1-9 direct selection
			if len(k) == 1 && k[0] >= '1' && k[0] <= '9' {
				idx := int(k[0] - '1')
				if idx < len(cm.Items) {
					cm.SelectedIndex = idx
					return cm.activateItem(idx, 1)
				}
			}
		}
	}
	return false, "", false
}

func (cm *ConfigModal) activateItem(idx int, dir int) (bool, string, bool) {
	if idx < 0 || idx >= len(cm.Items) {
		return false, "", false
	}
	item := cm.Items[idx]
	statusMsg := ""

	switch item.Key {
	case "auto_save":
		cm.Config.AutoSave = !cm.Config.AutoSave
		if cm.Config.AutoSave {
			statusMsg = "Auto-Save ENABLED"
		} else {
			statusMsg = "Auto-Save DISABLED"
		}
	case "check_updates":
		cm.Config.CheckUpdates = !cm.Config.CheckUpdates
		if cm.Config.CheckUpdates {
			statusMsg = "Check Updates ENABLED"
			if strings.ToLower(cm.Config.UpdateInterval) == "never" || strings.ToLower(cm.Config.UpdateInterval) == "off" {
				cm.Config.UpdateInterval = "daily"
			}
		} else {
			statusMsg = "Check Updates DISABLED"
		}
	case "update_interval":
		opts := []string{"daily", "always", "weekly", "never"}
		currIdx := 0
		for i, opt := range opts {
			if opt == strings.ToLower(cm.Config.UpdateInterval) {
				currIdx = i
				break
			}
		}
		nextIdx := (currIdx + dir + len(opts)) % len(opts)
		cm.Config.UpdateInterval = opts[nextIdx]
		if strings.ToLower(cm.Config.UpdateInterval) == "never" {
			cm.Config.CheckUpdates = false
		} else {
			cm.Config.CheckUpdates = true
		}
		statusMsg = fmt.Sprintf("Update frequency set to: %s", cm.Config.UpdateInterval)
	case "default_item_type":
		if cm.Config.DefaultItemType == "task" {
			cm.Config.DefaultItemType = "bullet"
		} else {
			cm.Config.DefaultItemType = "task"
		}
		statusMsg = fmt.Sprintf("Default item type: %s", cm.Config.DefaultItemType)
	case "theme":
		cm.Config.Theme = config.CycleTheme(cm.Config.Theme)
		statusMsg = fmt.Sprintf("Theme changed to %s", cm.Config.Theme)
	case "show_which_key":
		cm.Config.ShowWhichKey = !cm.Config.ShowWhichKey
		if cm.Config.ShowWhichKey {
			statusMsg = "WhichKey Popup ENABLED"
		} else {
			statusMsg = "WhichKey Popup DISABLED"
		}
	case "show_dashboard":
		cm.Config.ShowDashboard = !cm.Config.ShowDashboard
		if cm.Config.ShowDashboard {
			statusMsg = "Dashboard pane ENABLED"
		} else {
			statusMsg = "Dashboard pane DISABLED"
		}
	case "show_item_ids":
		cm.Config.ShowItemIDs = !cm.Config.ShowItemIDs
		if cm.Config.ShowItemIDs {
			statusMsg = "Task/Bullet numbers ENABLED"
		} else {
			statusMsg = "Task/Bullet numbers DISABLED"
		}
	case "indent_spaces":
		if cm.Config.IndentSpaces == 2 {
			cm.Config.IndentSpaces = 4
		} else if cm.Config.IndentSpaces == 4 {
			cm.Config.IndentSpaces = 8
		} else {
			cm.Config.IndentSpaces = 2
		}
		statusMsg = fmt.Sprintf("Indentation set to %d spaces", cm.Config.IndentSpaces)
	case "open_editor":
		return false, "", true
	}

	_ = config.SaveConfig(cm.Config)
	cm.RefreshItems()
	return false, statusMsg, false
}

func (cm *ConfigModal) Render(width, height int) string {
	modalWidth := width - 8
	if modalWidth > 72 {
		modalWidth = 72
	}
	if modalWidth < 52 {
		modalWidth = 52
	}
	if modalWidth > width-4 {
		modalWidth = width - 4
	}
	if modalWidth < 20 {
		modalWidth = 20
	}

	modalStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#bb9af7")).
		Padding(1, 2).
		Width(modalWidth)

	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#bb9af7")).
		Align(lipgloss.Center)

	catHeaderStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7aa2f7"))

	descStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Italic(true)

	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89"))

	var sb strings.Builder
	sb.WriteString(titleStyle.Render("⚙️   HalpTask App Configuration") + "\n\n")

	currentCategory := ""
	for i, item := range cm.Items {
		if item.Category != currentCategory {
			if currentCategory != "" {
				sb.WriteString("\n")
			}
			currentCategory = item.Category
			sb.WriteString(catHeaderStyle.Render(" "+currentCategory) + "\n")
		}

		prefix := "  "
		if i == cm.SelectedIndex {
			prefix = lipgloss.NewStyle().Foreground(lipgloss.Color("#bb9af7")).Bold(true).Render("❯ ")
		}

		numKey := ""
		if i < 9 {
			numKey = lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render(fmt.Sprintf("(%d) ", i+1))
		} else {
			numKey = "    "
		}

		// Calculate visual padding for label
		lblText := item.Label
		visLblLen := lipgloss.Width(lblText)
		padding := ""
		if visLblLen < 20 {
			padding = strings.Repeat(" ", 20-visLblLen)
		}

		lblStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#c0caf5"))
		if i == cm.SelectedIndex {
			lblStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#7dcfff")).Bold(true)
		}

		valStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#e0af68")).Bold(true)
		if item.Type == ConfigItemBool {
			if strings.Contains(item.Value, "Enabled") {
				valStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#9ece6a")).Bold(true)
			} else {
				valStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89"))
			}
		} else if item.Type == ConfigItemEnum {
			valStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#e0af68")).Bold(true)
		} else if item.Type == ConfigItemAction {
			valStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#7aa2f7")).Underline(true)
		}

		valStr := item.Value
		if item.Type == ConfigItemEnum {
			valStr = fmt.Sprintf("< %s >", item.Value)
		}

		// Truncate long value string if modal width is narrow
		maxValLen := modalWidth - 30
		if maxValLen < 15 {
			maxValLen = 15
		}
		if lipgloss.Width(valStr) > maxValLen {
			if len(valStr) > maxValLen {
				valStr = "..." + valStr[len(valStr)-maxValLen+3:]
			}
		}

		row := fmt.Sprintf("%s%s%s%s %s", prefix, numKey, lblStyle.Render(lblText), padding, valStyle.Render(valStr))
		sb.WriteString(row + "\n")
	}

	// Active item description box
	dividerLen := modalWidth - 4
	if dividerLen < 10 {
		dividerLen = 10
	}
	sb.WriteString("\n" + lipgloss.NewStyle().Foreground(lipgloss.Color("#414868")).Render(strings.Repeat("─", dividerLen)) + "\n")
	if cm.SelectedIndex >= 0 && cm.SelectedIndex < len(cm.Items) {
		descText := "💡 " + cm.Items[cm.SelectedIndex].Description
		if lipgloss.Width(descText) > modalWidth-4 {
			runes := []rune(descText)
			maxR := modalWidth - 4
			if len(runes) > maxR && maxR > 3 {
				descText = string(runes[:maxR-3]) + "..."
			}
		}
		sb.WriteString(descStyle.Render(descText) + "\n")
	} else {
		sb.WriteString("\n")
	}

	helpText := "[j/k] Move  [Space/Enter] Toggle  [e] Edit YAML  [Esc/q] Close"
	if lipgloss.Width(helpText) > modalWidth-4 {
		helpText = "[j/k] Move  [Space] Toggle  [e] Edit  [Esc] Close"
	}
	sb.WriteString(helpStyle.Render(helpText))

	return modalStyle.Render(sb.String())
}
