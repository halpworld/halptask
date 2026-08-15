package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/halpworld/halptask/config"
	"github.com/halpworld/halptask/model"
)

type TreeView struct {
	Width        int
	Height       int
	SelectedID   string
	SearchQuery  string
	MatchedIDs   map[string]bool
	IndentSpaces int
	Tree         *model.Tree
	TagConfigs   []config.TagConfig
	ShowItemIDs  bool
}

func NewTreeView() TreeView {
	return TreeView{
		Width:        80,
		Height:       20,
		IndentSpaces: 2,
		MatchedIDs:   make(map[string]bool),
		TagConfigs:   config.GetDefaultTagConfigs(),
		ShowItemIDs:  true,
	}
}

func (tv *TreeView) Render(visible []model.VisibleItem, cursorIndex int, scrollOffset int) string {
	maxLines := tv.Height
	if maxLines <= 0 {
		maxLines = 20
	}

	if len(visible) == 0 {
		emptyStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#565f89")).
			Italic(true)
		msg := emptyStyle.Render("No bullets yet. Press 'o' or '<space> b n' to create!")
		if tv.Width > 0 {
			w := lipgloss.Width(msg)
			if w < tv.Width {
				msg = msg + strings.Repeat(" ", tv.Width-w)
			} else {
				msg = lipgloss.NewStyle().MaxWidth(tv.Width).Render(msg)
			}
			var lines []string
			lines = append(lines, msg)
			for len(lines) < maxLines {
				lines = append(lines, strings.Repeat(" ", tv.Width))
			}
			return strings.Join(lines, "\n")
		}
		return msg
	}

	// Styles
	cursorStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7aa2f7"))

	indentGuideStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#3b4261"))

	foldIconStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#e0af68"))

	// Status styles
	todoBoxStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#787c99")) // Gray empty [ ]

	inProgressBoxStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#ff9e64")) // Orange [~]

	doneBoxStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#9ece6a")) // Green [x]

	// Text styles
	normalTextStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#c0caf5"))

	doneTextStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Strikethrough(true)

	selectedRowStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#2e3c64")).
		Bold(true)

	searchMatchStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#e0af68")).
		Foreground(lipgloss.Color("#1a1b26")).
		Bold(true)

	bulletStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#7dcfff"))

	focusBadgeStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#1a1b26")).
		Background(lipgloss.Color("#7dcfff")).
		Padding(0, 1)

	focusedTextStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7dcfff"))

	groupFocusedTextStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#7dcfff"))

	var lines []string

	// Determine render window bounds based on Height
	start := scrollOffset
	end := scrollOffset + maxLines
	if end > len(visible) {
		end = len(visible)
	}

	for i := start; i < end; i++ {
		v := visible[i]
		item := v.Item
		isSelected := (i == cursorIndex)

		// Check focus status
		isFocused := item.IsFocused
		isAncestorFocused := false
		curr := item.Parent
		for curr != nil {
			if curr.IsFocused {
				isAncestorFocused = true
				break
			}
			curr = curr.Parent
		}

		// Indentation prefix
		indentStr := ""
		if v.Depth > 0 {
			parts := make([]string, v.Depth)
			for d := 0; d < v.Depth; d++ {
				parts[d] = indentGuideStyle.Render("│ ")
			}
			indentStr = strings.Join(parts, "")
		}

		// Cursor prefix
		cursorStr := "  "
		if isSelected {
			cursorStr = cursorStyle.Render("❯ ")
		}

		// Fold icon / Bullet prefix
		var prefix string
		if v.HasChildren {
			if item.Folded {
				childCount := len(item.Children)
				prefix = foldIconStyle.Render(fmt.Sprintf("▶ [%d] ", childCount))
			} else {
				prefix = foldIconStyle.Render("▼ ")
			}
		} else {
			prefix = bulletStyle.Render("• ")
		}

		// Status / Checkbox prefix
		var statusBox string
		if item.IsTask {
			switch item.Status {
			case model.StatusDone:
				statusBox = doneBoxStyle.Render("[x] ")
			case model.StatusInProgress:
				statusBox = inProgressBoxStyle.Render("[~] ")
			case model.StatusTodo:
				statusBox = todoBoxStyle.Render("[ ] ")
			default:
				statusBox = todoBoxStyle.Render("[ ] ")
			}
		}

		// Item ID rendering
		idStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#565f89")).
			Faint(true)

		var idStr string
		if tv.ShowItemIDs && item.ID != "" {
			idStr = idStyle.Render(fmt.Sprintf("#%s ", item.ID))
		}

		// Text rendering
		var formattedText string
		if item.IsTask && item.Status == model.StatusDone {
			formattedText = doneTextStyle.Render(item.Text)
		} else if isFocused {
			formattedText = focusedTextStyle.Render(item.Text)
		} else if isAncestorFocused {
			formattedText = groupFocusedTextStyle.Render(item.Text)
		} else {
			formattedText = normalTextStyle.Render(item.Text)
		}

		// Search highlight if searching
		if tv.SearchQuery != "" && tv.MatchedIDs[item.ID] {
			formattedText = searchMatchStyle.Render(item.Text)
		}

		// Note indicator rendering
		if item.Note != "" {
			noteStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#bb9af7"))
			formattedText += " " + noteStyle.Render("📝")
		}

		// Focus badge if focused or part of focused group
		if isFocused {
			formattedText += " " + focusBadgeStyle.Render("🎯 FOCUS")
		} else if isAncestorFocused {
			ancestorBadge := lipgloss.NewStyle().Foreground(lipgloss.Color("#7dcfff")).Faint(true).Render("[🎯 focus group]")
			formattedText += " " + ancestorBadge
		}

		// Tag rendering (Direct & Inherited)
		tagStr := tv.formatTags(item)
		if tagStr != "" {
			formattedText += " " + tagStr
		}

		lineContent := fmt.Sprintf("%s%s%s%s%s%s", cursorStr, indentStr, prefix, idStr, statusBox, formattedText)

		if tv.Width > 0 {
			visW := lipgloss.Width(lineContent)
			if visW > tv.Width {
				rendered := lipgloss.NewStyle().MaxWidth(tv.Width).Render(lineContent)
				if idx := strings.IndexByte(rendered, '\n'); idx != -1 {
					rendered = rendered[:idx]
				}
				lineContent = rendered
				visW = lipgloss.Width(lineContent)
			}

			if isSelected {
				padding := ""
				if visW < tv.Width {
					padding = strings.Repeat(" ", tv.Width-visW)
				}
				lineContent = selectedRowStyle.Render(lineContent + padding)
			} else if visW < tv.Width {
				lineContent = lineContent + strings.Repeat(" ", tv.Width-visW)
			}
		} else if isSelected {
			lineContent = selectedRowStyle.Render(lineContent)
		}

		lines = append(lines, lineContent)
	}

	if tv.Width > 0 {
		for len(lines) < maxLines {
			lines = append(lines, strings.Repeat(" ", tv.Width))
		}
	}

	return strings.Join(lines, "\n")
}

func (tv *TreeView) formatTags(item *model.Item) string {
	if tv.Tree == nil || item == nil {
		return ""
	}
	direct, inherited := tv.Tree.GetEffectiveTags(item)
	if len(direct) == 0 && len(inherited) == 0 {
		return ""
	}

	tagConfigMap := make(map[string]config.TagConfig)
	for _, tc := range tv.TagConfigs {
		tagConfigMap[strings.ToLower(tc.Name)] = tc
	}

	var badges []string

	// Direct tags (solid badge)
	for _, dt := range direct {
		tagLower := strings.ToLower(dt)
		tc, ok := tagConfigMap[tagLower]
		emoji := "🏷️"
		colorHex := "#7aa2f7"
		if ok {
			if tc.Emoji != "" {
				emoji = tc.Emoji
			}
			if tc.Color != "" {
				colorHex = tc.Color
			}
		}
		style := lipgloss.NewStyle().Foreground(lipgloss.Color(colorHex)).Bold(true)
		badges = append(badges, style.Render(fmt.Sprintf("[%s %s]", emoji, dt)))
	}

	// Inherited tags (subtle/faint inherited badge with ↖)
	for _, it := range inherited {
		tagLower := strings.ToLower(it)
		tc, ok := tagConfigMap[tagLower]
		emoji := "🏷️"
		colorHex := "#7aa2f7"
		if ok {
			if tc.Emoji != "" {
				emoji = tc.Emoji
			}
			if tc.Color != "" {
				colorHex = tc.Color
			}
		}
		style := lipgloss.NewStyle().Foreground(lipgloss.Color(colorHex)).Faint(true)
		badges = append(badges, style.Render(fmt.Sprintf("[↖%s %s]", emoji, it)))
	}

	return strings.Join(badges, " ")
}
