package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/kenth/halptask/config"
	"github.com/kenth/halptask/model"
)

type TagModalState int

const (
	TagModalSelect TagModalState = iota
	TagModalNewName
	TagModalNewEmoji
	TagModalNewColor
)

type TagModal struct {
	Width         int
	Height        int
	State         TagModalState
	SelectedIndex int
	TagConfigs    []config.TagConfig
	Item          *model.Item
	Tree          *model.Tree

	// Fields for creating a new tag
	TextInput  textinput.Model
	NewTagName string
	NewEmoji   string
	NewColor   string
	EmojiIndex int
	ColorIndex int
}

var EmojiOptions = []string{"🏷️", "🐛", "🔥", "✨", "💼", "🏠", "💡", "📌", "👀", "⚡", "🎯", "🚀", "🎨", "🧪", "🔒", "⭐"}

var ColorOptions = []struct {
	Name  string
	Color string
}{
	{"Red", "#f7768e"},
	{"Orange", "#ff9e64"},
	{"Yellow", "#e0af68"},
	{"Green", "#9ece6a"},
	{"Mint", "#73daca"},
	{"Cyan", "#7dcfff"},
	{"Blue", "#7aa2f7"},
	{"Purple", "#bb9af7"},
	{"Pink", "#f7768e"},
	{"White", "#c0caf5"},
	{"Slate", "#565f89"},
	{"Peach", "#e0af68"},
}

func NewTagModal(tagConfigs []config.TagConfig) *TagModal {
	ti := textinput.New()
	ti.Placeholder = "tag name..."
	ti.CharLimit = 20

	return &TagModal{
		State:         TagModalSelect,
		SelectedIndex: 0,
		TagConfigs:    tagConfigs,
		TextInput:     ti,
	}
}

func (tm *TagModal) SetItem(item *model.Item, tree *model.Tree) {
	tm.Item = item
	tm.Tree = tree
	tm.State = TagModalSelect
	tm.SelectedIndex = 0
}

func (tm *TagModal) Update(msg tea.Msg) (bool, string) {
	switch tm.State {
	case TagModalSelect:
		switch msg := msg.(type) {
		case tea.KeyMsg:
			k := msg.String()
			switch k {
			case "j", "down":
				if len(tm.TagConfigs) > 0 {
					tm.SelectedIndex = (tm.SelectedIndex + 1) % (len(tm.TagConfigs) + 1) // +1 for "Create New Tag"
				}
			case "k", "up":
				if len(tm.TagConfigs) > 0 {
					tm.SelectedIndex = (tm.SelectedIndex - 1 + len(tm.TagConfigs) + 1) % (len(tm.TagConfigs) + 1)
				}
			case "enter", " ":
				if tm.SelectedIndex == len(tm.TagConfigs) {
					// Start "Create New Tag" wizard
					tm.State = TagModalNewName
					tm.TextInput.SetValue("")
					tm.TextInput.Focus()
					return false, ""
				} else if tm.Item != nil && tm.SelectedIndex < len(tm.TagConfigs) {
					targetTag := tm.TagConfigs[tm.SelectedIndex].Name
					tm.Item.ToggleTag(targetTag)
					return false, ""
				}
			case "n":
				// Shortcut for new tag
				tm.State = TagModalNewName
				tm.TextInput.SetValue("")
				tm.TextInput.Focus()
				return false, ""
			case "esc", "q":
				return true, "" // close modal
			default:
				// Number keys 1-9 shortcut
				if len(k) == 1 && k[0] >= '1' && k[0] <= '9' {
					idx := int(k[0] - '1')
					if idx < len(tm.TagConfigs) && tm.Item != nil {
						tm.Item.ToggleTag(tm.TagConfigs[idx].Name)
					}
				}
			}
		}

	case TagModalNewName:
		switch msg := msg.(type) {
		case tea.KeyMsg:
			switch msg.String() {
			case "enter":
				val := strings.TrimSpace(strings.ToLower(tm.TextInput.Value()))
				if val != "" {
					tm.NewTagName = val
					tm.State = TagModalNewEmoji
					tm.EmojiIndex = 0
				}
				return false, ""
			case "esc":
				tm.State = TagModalSelect
				return false, ""
			}
		}
		var cmd tea.Cmd
		tm.TextInput, cmd = tm.TextInput.Update(msg)
		_ = cmd

	case TagModalNewEmoji:
		switch msg := msg.(type) {
		case tea.KeyMsg:
			switch msg.String() {
			case "h", "left":
				tm.EmojiIndex = (tm.EmojiIndex - 1 + len(EmojiOptions)) % len(EmojiOptions)
			case "l", "right", "tab":
				tm.EmojiIndex = (tm.EmojiIndex + 1) % len(EmojiOptions)
			case "enter":
				tm.NewEmoji = EmojiOptions[tm.EmojiIndex]
				tm.State = TagModalNewColor
				tm.ColorIndex = 0
			case "esc":
				tm.State = TagModalSelect
			}
		}

	case TagModalNewColor:
		switch msg := msg.(type) {
		case tea.KeyMsg:
			switch msg.String() {
			case "h", "left", "k", "up":
				tm.ColorIndex = (tm.ColorIndex - 1 + len(ColorOptions)) % len(ColorOptions)
			case "l", "right", "j", "down":
				tm.ColorIndex = (tm.ColorIndex + 1) % len(ColorOptions)
			case "enter":
				tm.NewColor = ColorOptions[tm.ColorIndex].Color
				newTag := config.TagConfig{
					Name:  tm.NewTagName,
					Emoji: tm.NewEmoji,
					Color: tm.NewColor,
				}
				tm.TagConfigs = append(tm.TagConfigs, newTag)
				if tm.Item != nil {
					tm.Item.AddTag(tm.NewTagName)
				}
				tm.State = TagModalSelect
				tm.SelectedIndex = len(tm.TagConfigs) - 1
			case "esc":
				tm.State = TagModalSelect
			}
		}
	}

	return false, ""
}

func (tm *TagModal) Render(width, height int) string {
	modalWidth := width - 12
	if modalWidth < 58 {
		modalWidth = 58
	} else if modalWidth > 72 {
		modalWidth = 72
	}

	modalStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#7aa2f7")).
		Padding(1, 2).
		Width(modalWidth)

	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7aa2f7")).
		Align(lipgloss.Center)

	subTitleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Italic(true)

	if tm.Item == nil {
		return modalStyle.Render("No item selected")
	}

	direct, inherited := tm.Tree.GetEffectiveTags(tm.Item)
	directSet := make(map[string]bool)
	for _, d := range direct {
		directSet[strings.ToLower(d)] = true
	}
	inheritedSet := make(map[string]bool)
	for _, inh := range inherited {
		inheritedSet[strings.ToLower(inh)] = true
	}

	var sb strings.Builder
	sb.WriteString(titleStyle.Render("🏷️  Manage Task Tags & Labels") + "\n")
	itemTitle := tm.Item.Text
	if len(itemTitle) > 40 {
		itemTitle = itemTitle[:37] + "..."
	}
	sb.WriteString(subTitleStyle.Render(fmt.Sprintf("Item: \"%s\"", itemTitle)) + "\n\n")

	switch tm.State {
	case TagModalSelect:
		sb.WriteString(lipgloss.NewStyle().Bold(true).Render("Select tag to toggle (Enter / Space / 1-9):") + "\n\n")

		for i, tc := range tm.TagConfigs {
			prefix := "  "
			if i == tm.SelectedIndex {
				prefix = "❯ "
			}

			tagLower := strings.ToLower(tc.Name)
			statusBadge := "[ ]"
			if directSet[tagLower] {
				statusBadge = lipgloss.NewStyle().Foreground(lipgloss.Color("#9ece6a")).Bold(true).Render("[✓ direct]")
			} else if inheritedSet[tagLower] {
				statusBadge = lipgloss.NewStyle().Foreground(lipgloss.Color("#e0af68")).Faint(true).Render("[↖ inherited]")
			}

			colorStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(tc.Color)).Bold(true)
			tagText := tc.Emoji + " " + tc.Name
			visW := lipgloss.Width(tagText)
			padding := ""
			if visW < 14 {
				padding = strings.Repeat(" ", 14-visW)
			}
			tagBadge := colorStyle.Render(tagText + padding)

			numKey := "    "
			if i < 9 {
				numKey = lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render(fmt.Sprintf("(%d) ", i+1))
			}

			rowLine := fmt.Sprintf("%s%s%s  %s", prefix, numKey, tagBadge, statusBadge)
			sb.WriteString(rowLine + "\n")
		}

		// "Create New Tag" option
		prefixNew := "  "
		if tm.SelectedIndex == len(tm.TagConfigs) {
			prefixNew = lipgloss.NewStyle().Foreground(lipgloss.Color("#7aa2f7")).Bold(true).Render("❯ ")
		}
		sb.WriteString(fmt.Sprintf("\n%s%s\n", prefixNew, lipgloss.NewStyle().Foreground(lipgloss.Color("#bb9af7")).Bold(true).Render("➕ Create New Custom Tag (press 'n')")))
		sb.WriteString("\n" + lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("[j/k] Navigate  [Enter/Space] Toggle  [Esc/q] Close"))

	case TagModalNewName:
		sb.WriteString(lipgloss.NewStyle().Bold(true).Render("Step 1/3: Enter Tag Name:\n\n"))
		sb.WriteString(tm.TextInput.View())
		sb.WriteString("\n\n" + lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("[Enter] Next  [Esc] Cancel"))

	case TagModalNewEmoji:
		sb.WriteString(lipgloss.NewStyle().Bold(true).Render(fmt.Sprintf("Step 2/3: Select Emoji for '#%s':\n\n", tm.NewTagName)))

		var row1, row2 []string
		for idx, em := range EmojiOptions {
			var cell string
			if idx == tm.EmojiIndex {
				cell = lipgloss.NewStyle().Background(lipgloss.Color("#2e3c64")).Bold(true).Render(" " + em + " ")
			} else {
				cell = " " + em + " "
			}
			if idx < 8 {
				row1 = append(row1, cell)
			} else {
				row2 = append(row2, cell)
			}
		}
		sb.WriteString(strings.Join(row1, ""))
		sb.WriteString("\n")
		sb.WriteString(strings.Join(row2, ""))
		sb.WriteString("\n\n" + lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("[h/l] Select  [Enter] Next  [Esc] Cancel"))

	case TagModalNewColor:
		sb.WriteString(lipgloss.NewStyle().Bold(true).Render(fmt.Sprintf("Step 3/3: Select Color for '%s #%s':\n\n", tm.NewEmoji, tm.NewTagName)))

		var row1, row2 []string
		for idx, col := range ColorOptions {
			style := lipgloss.NewStyle().Foreground(lipgloss.Color(col.Color)).Bold(true)
			var cell string
			if idx == tm.ColorIndex {
				cell = lipgloss.NewStyle().Background(lipgloss.Color("#2e3c64")).Render(" " + style.Render(col.Name) + " ")
			} else {
				cell = " " + style.Render(col.Name) + " "
			}
			if idx < 6 {
				row1 = append(row1, cell)
			} else {
				row2 = append(row2, cell)
			}
		}
		sb.WriteString(strings.Join(row1, " "))
		sb.WriteString("\n")
		sb.WriteString(strings.Join(row2, " "))
		sb.WriteString("\n\n" + lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("[h/l] Select Color  [Enter] Save Tag  [Esc] Cancel"))
	}

	return modalStyle.Render(sb.String())
}
