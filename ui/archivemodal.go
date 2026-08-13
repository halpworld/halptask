package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/kenth/halptask/model"
)

type ArchiveModal struct {
	Store         *model.ArchiveStore
	Entries       []*model.ArchivedEntry
	Filtered      []*model.ArchivedEntry
	SearchInput   textinput.Model
	CursorIndex   int
	ScrollOffset  int
	Width         int
	Height        int
	Passphrase    string
	StatusMsg     string
	ConfirmDelete bool
	Active        bool
}

func NewArchiveModal(store *model.ArchiveStore) *ArchiveModal {
	si := textinput.New()
	si.Prompt = "🔍 Filter Archive: "
	si.Placeholder = "Search archived items..."

	return &ArchiveModal{
		Store:       store,
		SearchInput: si,
		Width:       80,
		Height:      24,
	}
}

func (m *ArchiveModal) Open(entries []*model.ArchivedEntry, passphrase string) {
	m.Entries = entries
	m.Passphrase = passphrase
	m.CursorIndex = 0
	m.ScrollOffset = 0
	m.ConfirmDelete = false
	m.StatusMsg = ""
	m.SearchInput.SetValue("")
	m.SearchInput.Blur()
	m.Active = true
	m.ApplyFilter()
}

func (m *ArchiveModal) ApplyFilter() {
	query := strings.ToLower(strings.TrimSpace(m.SearchInput.Value()))
	if query == "" {
		m.Filtered = m.Entries
	} else {
		var result []*model.ArchivedEntry
		for _, e := range m.Entries {
			text := ""
			if e.Item != nil {
				text = e.Item.Text
				for _, t := range e.Item.Tags {
					text += " #" + t
				}
			}
			if strings.Contains(strings.ToLower(text), query) ||
				strings.Contains(strings.ToLower(e.Context), query) ||
				strings.Contains(strings.ToLower(e.ID), query) {
				result = append(result, e)
			}
		}
		m.Filtered = result
	}
	m.ensureValidCursor()
}

func (m *ArchiveModal) ensureValidCursor() {
	if len(m.Filtered) == 0 {
		m.CursorIndex = 0
		m.ScrollOffset = 0
		return
	}
	if m.CursorIndex < 0 {
		m.CursorIndex = 0
	}
	if m.CursorIndex >= len(m.Filtered) {
		m.CursorIndex = len(m.Filtered) - 1
	}

	maxVisible := m.Height - 12
	if maxVisible < 3 {
		maxVisible = 3
	}
	if m.CursorIndex < m.ScrollOffset {
		m.ScrollOffset = m.CursorIndex
	} else if m.CursorIndex >= m.ScrollOffset+maxVisible {
		m.ScrollOffset = m.CursorIndex - maxVisible + 1
	}
}

func (m *ArchiveModal) Update(msg tea.KeyMsg) (closeModal bool, restoredEntry *model.ArchivedEntry, statusMsg string) {
	k := msg.String()

	if m.ConfirmDelete {
		switch k {
		case "y", "Y", "enter":
			if len(m.Filtered) > 0 && m.CursorIndex < len(m.Filtered) {
				target := m.Filtered[m.CursorIndex]
				var remaining []*model.ArchivedEntry
				for _, e := range m.Entries {
					if e != target {
						remaining = append(remaining, e)
					}
				}
				m.Entries = remaining
				_ = m.Store.Save(m.Entries, m.Passphrase)
				m.ApplyFilter()
				m.ConfirmDelete = false
				return false, nil, "Permanently deleted entry from archive"
			}
			m.ConfirmDelete = false
			return false, nil, ""
		case "n", "N", "esc":
			m.ConfirmDelete = false
			return false, nil, ""
		default:
			return false, nil, ""
		}
	}

	if m.SearchInput.Focused() {
		switch k {
		case "esc", "enter":
			m.SearchInput.Blur()
			return false, nil, ""
		default:
			var cmd tea.Cmd
			m.SearchInput, cmd = m.SearchInput.Update(msg)
			_ = cmd
			m.ApplyFilter()
			return false, nil, ""
		}
	}

	switch k {
	case "/":
		m.SearchInput.Focus()
		return false, nil, ""
	case "j", "down":
		m.CursorIndex++
		m.ensureValidCursor()
	case "k", "up":
		m.CursorIndex--
		m.ensureValidCursor()
	case "g", "home":
		m.CursorIndex = 0
		m.ensureValidCursor()
	case "G", "end":
		m.CursorIndex = len(m.Filtered) - 1
		m.ensureValidCursor()
	case "r", "enter":
		if len(m.Filtered) > 0 && m.CursorIndex < len(m.Filtered) {
			target := m.Filtered[m.CursorIndex]
			var remaining []*model.ArchivedEntry
			for _, e := range m.Entries {
				if e != target {
					remaining = append(remaining, e)
				}
			}
			m.Entries = remaining
			_ = m.Store.Save(m.Entries, m.Passphrase)
			m.Active = false
			return true, target, fmt.Sprintf("Restored item #%s from archive", target.Item.ID)
		}
	case "d", "x", "delete":
		if len(m.Filtered) > 0 {
			m.ConfirmDelete = true
		}
	case "esc", "q":
		m.Active = false
		return true, nil, ""
	}

	return false, nil, ""
}

func (m *ArchiveModal) Render(width, height int) string {
	m.Width = width
	m.Height = height

	modalWidth := width - 6
	if modalWidth > 110 {
		modalWidth = 110
	}
	if modalWidth < 50 {
		modalWidth = 50
	}

	modalHeight := height - 4
	if modalHeight < 14 {
		modalHeight = 14
	}

	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#1a1b26")).
		Background(lipgloss.Color("#bb9af7")).
		Padding(0, 1)

	headerText := fmt.Sprintf(" 📦 HALPTASK ARCHIVE BROWSER (%d entries) ", len(m.Entries))
	header := titleStyle.Render(headerText)

	if m.ConfirmDelete {
		deletePromptStyle := lipgloss.NewStyle().
			Border(lipgloss.DoubleBorder()).
			BorderForeground(lipgloss.Color("#f7768e")).
			Padding(1, 2).
			Width(50)

		targetText := ""
		if len(m.Filtered) > 0 && m.CursorIndex < len(m.Filtered) {
			targetText = m.Filtered[m.CursorIndex].Item.Text
		}

		content := fmt.Sprintf(
			"%s\n\nAre you sure you want to PERMANENTLY delete this archived entry?\n\n\"%s\"\n\n%s",
			lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#f7768e")).Render("⚠️ Confirm Permanent Deletion"),
			targetText,
			lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("[y/Enter] Delete   [n/Esc] Cancel"),
		)

		return lipgloss.Place(width, height, lipgloss.Center, lipgloss.Center, deletePromptStyle.Render(content))
	}

	searchBox := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#7aa2f7")).
		Padding(0, 1).
		Width(modalWidth - 4).
		Render(m.SearchInput.View())

	listHeight := modalHeight - 8
	if listHeight < 4 {
		listHeight = 4
	}

	leftWidth := (modalWidth - 6) / 2
	rightWidth := modalWidth - 6 - leftWidth

	// Build List view (left pane)
	var listLines []string
	if len(m.Filtered) == 0 {
		emptyMsg := lipgloss.NewStyle().Italic(true).Foreground(lipgloss.Color("#565f89")).Render(" No archived items found")
		listLines = append(listLines, emptyMsg)
	} else {
		endIdx := m.ScrollOffset + listHeight
		if endIdx > len(m.Filtered) {
			endIdx = len(m.Filtered)
		}
		for i := m.ScrollOffset; i < endIdx; i++ {
			entry := m.Filtered[i]
			isSelected := (i == m.CursorIndex)

			prefix := "  "
			if isSelected {
				prefix = "> "
			}

			icon := "•"
			if entry.Item != nil && entry.Item.IsTask {
				switch entry.Item.Status {
				case model.StatusDone:
					icon = "[x]"
				case model.StatusInProgress:
					icon = "[~]"
				default:
					icon = "[ ]"
				}
			}

			text := ""
			if entry.Item != nil {
				text = entry.Item.Text
			}

			maxLen := leftWidth - 8
			if maxLen < 10 {
				maxLen = 10
			}
			runes := []rune(text)
			if len(runes) > maxLen && maxLen > 3 {
				text = string(runes[:maxLen-3]) + "..."
			}

			lineStr := fmt.Sprintf("%s%s %s", prefix, icon, text)
			lineStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#a9b1d6"))
			if isSelected {
				lineStyle = lipgloss.NewStyle().
					Bold(true).
					Foreground(lipgloss.Color("#1a1b26")).
					Background(lipgloss.Color("#7aa2f7"))
			}

			// ANSI-safe padding
			visW := lipgloss.Width(lineStr)
			if visW < leftWidth-2 {
				lineStr += strings.Repeat(" ", (leftWidth-2)-visW)
			}
			listLines = append(listLines, lineStyle.Render(lineStr))
		}
	}

	leftPaneContent := strings.Join(listLines, "\n")
	leftPaneStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#3b4261")).
		Width(leftWidth).
		Height(listHeight)

	leftPanel := leftPaneStyle.Render(leftPaneContent)

	// Build Detail view (right pane)
	var detailLines []string
	if len(m.Filtered) > 0 && m.CursorIndex < len(m.Filtered) {
		selected := m.Filtered[m.CursorIndex]

		dateStr := selected.ArchivedAt.Format("2006-01-02 15:04")
		detailLines = append(detailLines, lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#7aa2f7")).Render(fmt.Sprintf("ID: #%s", selected.ID)))
		detailLines = append(detailLines, lipgloss.NewStyle().Foreground(lipgloss.Color("#e0af68")).Render(fmt.Sprintf("Archived: %s", dateStr)))
		if selected.Context != "" {
			detailLines = append(detailLines, lipgloss.NewStyle().Foreground(lipgloss.Color("#bb9af7")).Render(fmt.Sprintf("Context: %s", selected.Context)))
		}
		detailLines = append(detailLines, "")
		detailLines = append(detailLines, lipgloss.NewStyle().Bold(true).Render("Subtree Preview:"))

		var renderSubtree func(item *model.Item, depth int)
		renderSubtree = func(item *model.Item, depth int) {
			if item == nil {
				return
			}
			indent := strings.Repeat("  ", depth)
			icon := "•"
			if item.IsTask {
				switch item.Status {
				case model.StatusDone:
					icon = "[x]"
				case model.StatusInProgress:
					icon = "[~]"
				default:
					icon = "[ ]"
				}
			}
			tagStr := ""
			if len(item.Tags) > 0 {
				tagStr = " #" + strings.Join(item.Tags, " #")
			}
			line := fmt.Sprintf("%s%s %s%s", indent, icon, item.Text, tagStr)
			if depth == 0 {
				detailLines = append(detailLines, lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#c0caf5")).Render(line))
			} else {
				detailLines = append(detailLines, lipgloss.NewStyle().Foreground(lipgloss.Color("#a9b1d6")).Render(line))
			}
			for _, child := range item.Children {
				renderSubtree(child, depth+1)
			}
		}

		renderSubtree(selected.Item, 0)
	} else {
		detailLines = append(detailLines, lipgloss.NewStyle().Italic(true).Foreground(lipgloss.Color("#565f89")).Render("Select an item to view preview"))
	}

	rightPaneContent := strings.Join(detailLines, "\n")
	rightPaneStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#3b4261")).
		Width(rightWidth).
		Height(listHeight)

	rightPanel := rightPaneStyle.Render(rightPaneContent)

	mainArea := lipgloss.JoinHorizontal(lipgloss.Top, leftPanel, rightPanel)

	footerHelp := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Render("[r/Enter] Restore to Tree   [d/x] Delete   [/] Filter   [Esc/q] Close")

	modalContent := fmt.Sprintf("%s\n\n%s\n\n%s\n\n%s", header, searchBox, mainArea, footerHelp)

	borderStyle := lipgloss.NewStyle().
		Border(lipgloss.DoubleBorder()).
		BorderForeground(lipgloss.Color("#bb9af7")).
		Padding(0, 1).
		Width(modalWidth)

	return borderStyle.Render(modalContent)
}
