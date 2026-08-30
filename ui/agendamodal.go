package ui

import (
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/textinput"
	"github.com/charmbracelet/lipgloss"
	"github.com/halpworld/halptask/model"
)

type AgendaSectionType int

const (
	SectionOverdue AgendaSectionType = iota
	SectionToday
	SectionThisWeek
	SectionUpcoming
	SectionCompleted
)

type AgendaEntry struct {
	Item          *model.Item
	Section       AgendaSectionType
	ParentPath    string
	InheritedTags []string
}

type AgendaModal struct {
	Tree         *model.Tree
	Entries      []AgendaEntry
	Filtered     []AgendaEntry
	SearchInput  textinput.Model
	CursorIndex  int
	ScrollOffset int
	Width        int
	Height       int
	Active       bool
	StatusMsg    string
}

func NewAgendaModal() *AgendaModal {
	si := textinput.New()
	si.Prompt = "🔍 Filter Agenda: "
	si.Placeholder = "Search tasks, tags, context..."

	return &AgendaModal{
		SearchInput: si,
		Width:       80,
		Height:      24,
	}
}

func (m *AgendaModal) Open(tree *model.Tree) {
	m.Tree = tree
	m.CursorIndex = 0
	m.ScrollOffset = 0
	m.StatusMsg = ""
	m.SearchInput.SetValue("")
	m.SearchInput.Blur()
	m.Active = true
	m.RebuildEntries()
}

func (m *AgendaModal) RebuildEntries() {
	if m.Tree == nil {
		m.Entries = nil
		m.Filtered = nil
		return
	}

	m.Tree.SetParents()
	now := time.Now()
	todayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())

	var overdue []AgendaEntry
	var today []AgendaEntry
	var thisWeek []AgendaEntry
	var upcoming []AgendaEntry
	var completed []AgendaEntry

	var recurse func(items []*model.Item)
	recurse = func(items []*model.Item) {
		for _, it := range items {
			if it.IsTask && it.DueDate > 0 {
				dueDate := time.Unix(it.DueDate, 0).In(now.Location())
				dueDayStart := time.Date(dueDate.Year(), dueDate.Month(), dueDate.Day(), 0, 0, 0, 0, dueDate.Location())
				daysDiff := int(dueDayStart.Sub(todayStart).Hours() / 24)

				parentPath := m.Tree.GetParentPath(it)
				allTags := m.Tree.GetAllTags(it)

				entry := AgendaEntry{
					Item:          it,
					ParentPath:    parentPath,
					InheritedTags: allTags,
				}

				if it.Status == model.StatusDone {
					entry.Section = SectionCompleted
					completed = append(completed, entry)
				} else if daysDiff < 0 {
					entry.Section = SectionOverdue
					overdue = append(overdue, entry)
				} else if daysDiff == 0 {
					entry.Section = SectionToday
					today = append(today, entry)
				} else if daysDiff < 7 {
					entry.Section = SectionThisWeek
					thisWeek = append(thisWeek, entry)
				} else {
					entry.Section = SectionUpcoming
					upcoming = append(upcoming, entry)
				}
			}
			if len(it.Children) > 0 {
				recurse(it.Children)
			}
		}
	}

	recurse(m.Tree.Roots)

	var all []AgendaEntry
	all = append(all, overdue...)
	all = append(all, today...)
	all = append(all, thisWeek...)
	all = append(all, upcoming...)
	all = append(all, completed...)

	m.Entries = all
	m.ApplyFilter()
}

func (m *AgendaModal) ApplyFilter() {
	query := strings.ToLower(strings.TrimSpace(m.SearchInput.Value()))
	if query == "" {
		m.Filtered = m.Entries
	} else {
		var result []AgendaEntry
		for _, e := range m.Entries {
			text := e.Item.Text
			for _, t := range e.InheritedTags {
				text += " #" + t
			}
			if strings.Contains(strings.ToLower(text), query) ||
				strings.Contains(strings.ToLower(e.ParentPath), query) ||
				strings.Contains(strings.ToLower(e.Item.ID), query) {
				result = append(result, e)
			}
		}
		m.Filtered = result
	}
	m.ensureValidCursor()
}

func (m *AgendaModal) ensureValidCursor() {
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

	maxVisible := m.Height - 10
	if maxVisible < 3 {
		maxVisible = 3
	}
	if m.CursorIndex < m.ScrollOffset {
		m.ScrollOffset = m.CursorIndex
	} else if m.CursorIndex >= m.ScrollOffset+maxVisible {
		m.ScrollOffset = m.CursorIndex - maxVisible + 1
	}
}

func (m *AgendaModal) SelectedItem() *model.Item {
	if len(m.Filtered) == 0 || m.CursorIndex < 0 || m.CursorIndex >= len(m.Filtered) {
		return nil
	}
	return m.Filtered[m.CursorIndex].Item
}

func (m *AgendaModal) Next() {
	if len(m.Filtered) == 0 {
		return
	}
	if m.CursorIndex < len(m.Filtered)-1 {
		m.CursorIndex++
		m.ensureValidCursor()
	}
}

func (m *AgendaModal) Prev() {
	if len(m.Filtered) == 0 {
		return
	}
	if m.CursorIndex > 0 {
		m.CursorIndex--
		m.ensureValidCursor()
	}
}

func (m *AgendaModal) Render() string {
	modalWidth := m.Width - 6
	if modalWidth < 60 {
		modalWidth = 60
	}
	if modalWidth > 110 {
		modalWidth = 110
	}

	modalHeight := m.Height - 4
	if modalHeight < 15 {
		modalHeight = 15
	}

	contentWidth := modalWidth - 4
	if contentWidth < 40 {
		contentWidth = 40
	}

	borderStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#7aa2f7")).
		Padding(0, 1).
		Width(modalWidth)

	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7aa2f7"))

	overdueSecStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#f7768e"))

	todaySecStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#e0af68"))

	weekSecStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#9ece6a"))

	upcomingSecStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7dcfff"))

	completedSecStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#565f89"))

	selectedRowStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#2e3c64")).
		Bold(true)

	cursorStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7aa2f7"))

	todoBoxStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#787c99"))

	inProgressBoxStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#ff9e64"))

	doneBoxStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#9ece6a"))

	contextStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Italic(true)

	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Faint(true)

	var lines []string

	// Header
	header := titleStyle.Render("📅 AGENDA & SCHEDULE VIEW")
	lines = append(lines, header)

	// Filter Input
	lines = append(lines, m.SearchInput.View())
	lines = append(lines, "")

	if len(m.Filtered) == 0 {
		emptyStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#565f89")).
			Italic(true)
		lines = append(lines, emptyStyle.Render("No scheduled tasks found. Use 'D' or 'due:tomorrow' to attach due dates!"))
		lines = append(lines, "")
	} else {
		// Render items with section indicators
		maxListLines := modalHeight - 9
		if maxListLines < 5 {
			maxListLines = 5
		}

		end := m.ScrollOffset + maxListLines
		if end > len(m.Filtered) {
			end = len(m.Filtered)
		}

		now := time.Now()
		var lastSection AgendaSectionType = -1

		for i := m.ScrollOffset; i < end; i++ {
			entry := m.Filtered[i]
			it := entry.Item
			isSelected := (i == m.CursorIndex)

			// Section header if transitioned
			if entry.Section != lastSection {
				lastSection = entry.Section
				var secTitle string
				switch entry.Section {
				case SectionOverdue:
					secTitle = overdueSecStyle.Render("🔴 OVERDUE")
				case SectionToday:
					secTitle = todaySecStyle.Render("🟡 DUE TODAY")
				case SectionThisWeek:
					secTitle = weekSecStyle.Render("🟢 THIS WEEK")
				case SectionUpcoming:
					secTitle = upcomingSecStyle.Render("🔵 UPCOMING")
				case SectionCompleted:
					secTitle = completedSecStyle.Render("🏁 COMPLETED")
				}
				lines = append(lines, fmt.Sprintf("── %s %s", secTitle, strings.Repeat("─", max(2, contentWidth-lipgloss.Width(secTitle)-5))))
			}

			// Cursor
			cur := "  "
			if isSelected {
				cur = cursorStyle.Render("❯ ")
			}

			// Status box
			var statusBox string
			switch it.Status {
			case model.StatusDone:
				statusBox = doneBoxStyle.Render("[x] ")
			case model.StatusInProgress:
				statusBox = inProgressBoxStyle.Render("[~] ")
			default:
				statusBox = todoBoxStyle.Render("[ ] ")
			}

			// Due badge
			badge, cat := model.FormatDueBadge(it.DueDate, it.Status == model.StatusDone, now)
			var badgeStr string
			switch cat {
			case "overdue":
				badgeStr = overdueSecStyle.Render(badge)
			case "today":
				badgeStr = todaySecStyle.Render(badge)
			case "done":
				badgeStr = completedSecStyle.Render(badge)
			default:
				badgeStr = upcomingSecStyle.Render(badge)
			}

			// Item Text & ID
			idStr := lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Faint(true).Render(fmt.Sprintf("#%s ", it.ID))
			taskText := it.Text
			if it.Status == model.StatusDone {
				taskText = lipgloss.NewStyle().Strikethrough(true).Foreground(lipgloss.Color("#565f89")).Render(taskText)
			} else {
				taskText = lipgloss.NewStyle().Foreground(lipgloss.Color("#c0caf5")).Render(taskText)
			}

			ctxStr := ""
			if entry.ParentPath != "" {
				ctxStr = " " + contextStyle.Render(fmt.Sprintf("(↖ %s)", entry.ParentPath))
			}

			row := fmt.Sprintf("%s%s%s%s %s%s", cur, cur[:0], idStr, statusBox, taskText, ctxStr)
			if badgeStr != "" {
				row += " " + badgeStr
			}

			visW := lipgloss.Width(row)
			if visW > contentWidth {
				row = lipgloss.NewStyle().MaxWidth(contentWidth).Render(row)
				if idx := strings.IndexByte(row, '\n'); idx != -1 {
					row = row[:idx]
				}
				visW = lipgloss.Width(row)
			}

			if isSelected {
				pad := ""
				if visW < contentWidth {
					pad = strings.Repeat(" ", contentWidth-visW)
				}
				row = selectedRowStyle.Render(row + pad)
			}
			lines = append(lines, row)
		}
	}

	// Status / Footer message
	lines = append(lines, "")
	if m.StatusMsg != "" {
		lines = append(lines, lipgloss.NewStyle().Foreground(lipgloss.Color("#9ece6a")).Bold(true).Render(m.StatusMsg))
	} else {
		footer := hintStyle.Render("j/k: Navigate • Enter: Jump to Tree • t: Cycle Status • D: Edit Due Date • /: Search • Esc/q: Close")
		lines = append(lines, footer)
	}

	return borderStyle.Render(strings.Join(lines, "\n"))
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
