package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/halpworld/halptask/config"
	"github.com/halpworld/halptask/model"
)

type DashboardView struct {
	Width      int
	Height     int
	TagConfigs []config.TagConfig
}

func NewDashboardView() DashboardView {
	return DashboardView{
		Width:      30,
		Height:     20,
		TagConfigs: config.GetDefaultTagConfigs(),
	}
}

func (db *DashboardView) Render(tree *model.Tree, tagConfigs []config.TagConfig) string {
	boxWidth := db.Width
	if boxWidth <= 0 {
		boxWidth = 30
	}
	boxHeight := db.Height
	if boxHeight <= 0 {
		boxHeight = 20
	}
	if tagConfigs != nil {
		db.TagConfigs = tagConfigs
	}

	contentWidth := boxWidth - 2
	if contentWidth < 8 {
		contentWidth = 8
	}

	// LipGloss Styles - Muted & Subtle side panel
	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7aa2f7")).
		Width(contentWidth).
		Align(lipgloss.Left)

	sectionHeaderStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#bb9af7"))

	inProgressIconStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#ff9e64")) // Orange [~]

	taskTextStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#a9b1d6"))

	parentHintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Italic(true)

	statLabelStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89"))

	doneCountStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#9ece6a"))

	inProgCountStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#ff9e64"))

	todoCountStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#787c99"))

	emptyStateStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Italic(true)

	panelStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#2a2e42")).
		Width(contentWidth).
		Height(boxHeight - 2)

	var lines []string

	// Header
	lines = append(lines, titleStyle.Render("⚡ DASHBOARD"))
	lines = append(lines, "")

	// 1. Overview Statistics Section
	stats := model.TaskStats{}
	if tree != nil {
		stats = tree.GetStats()
	}

	pct := 0
	if stats.Total > 0 {
		pct = (stats.Done * 100) / stats.Total
	}

	lines = append(lines, sectionHeaderStyle.Render("📊 OVERVIEW"))
	statLine := fmt.Sprintf("%s %s │ %s %s │ %s %s",
		todoCountStyle.Render(fmt.Sprintf("[ ] %d", stats.Todo)),
		statLabelStyle.Render("todo"),
		inProgCountStyle.Render(fmt.Sprintf("[~] %d", stats.InProgress)),
		statLabelStyle.Render("active"),
		doneCountStyle.Render(fmt.Sprintf("[x] %d", stats.Done)),
		statLabelStyle.Render("done"),
	)
	lines = append(lines, truncateOrPad(statLine, contentWidth))

	// Visual Progress Bar
	barWidth := contentWidth - 7
	if barWidth > 15 {
		barWidth = 15
	}
	if barWidth < 4 {
		barWidth = 4
	}
	filledLen := 0
	if stats.Total > 0 {
		filledLen = (stats.Done * barWidth) / stats.Total
	}
	emptyLen := barWidth - filledLen
	if emptyLen < 0 {
		emptyLen = 0
	}
	progressBar := fmt.Sprintf("[%s%s] %d%%",
		doneCountStyle.Render(strings.Repeat("█", filledLen)),
		statLabelStyle.Render(strings.Repeat("░", emptyLen)),
		pct,
	)
	lines = append(lines, truncateOrPad(progressBar, contentWidth))
	lines = append(lines, "")

	// 2. In-Progress Tasks Section
	lines = append(lines, sectionHeaderStyle.Render(fmt.Sprintf("🔄 IN PROGRESS (%d)", stats.InProgress)))

	var inProgressTasks []model.TaskWithContext
	if tree != nil {
		inProgressTasks = tree.GetInProgressTasks()
	}

	if len(inProgressTasks) == 0 {
		lines = append(lines, emptyStateStyle.Render("✨ No tasks in progress"))
		lines = append(lines, emptyStateStyle.Render("  Press 't' to set status"))
	} else {
		for _, taskCtx := range inProgressTasks {
			taskItem := taskCtx.Item

			// Task Line: [~] Task text
			icon := inProgressIconStyle.Render("[~] ")
			txt := taskTextStyle.Render(taskItem.Text)
			row1 := fmt.Sprintf("%s%s", icon, txt)
			lines = append(lines, truncateOrPad(row1, contentWidth))

			// Subtask Hint Line (if it has a parent context)
			if taskCtx.ParentPath != "" {
				hintText := fmt.Sprintf("  ↖ under: %s", taskCtx.ParentPath)
				lines = append(lines, truncateOrPad(parentHintStyle.Render(hintText), contentWidth))
			} else if taskItem.Parent != nil && taskItem.Parent.Text != "" {
				hintText := fmt.Sprintf("  ↖ under: %s", taskItem.Parent.Text)
				lines = append(lines, truncateOrPad(parentHintStyle.Render(hintText), contentWidth))
			}
		}
	}

	// 3. Active Tags Section (if vertical space is available in large terminals)
	maxContentLines := boxHeight - 2
	if maxContentLines < 1 {
		maxContentLines = 1
	}

	if tree != nil && len(lines)+4 <= maxContentLines {
		tagCounts := make(map[string]int)
		var collectTags func(items []*model.Item)
		collectTags = func(items []*model.Item) {
			for _, it := range items {
				all := tree.GetAllTags(it)
				for _, t := range all {
					tagCounts[strings.ToLower(t)]++
				}
				if len(it.Children) > 0 {
					collectTags(it.Children)
				}
			}
		}
		collectTags(tree.Roots)

		if len(tagCounts) > 0 {
			lines = append(lines, "")
			lines = append(lines, sectionHeaderStyle.Render(fmt.Sprintf("🏷️  ACTIVE TAGS (%d)", len(tagCounts))))

			tagConfigMap := make(map[string]config.TagConfig)
			for _, tc := range db.TagConfigs {
				tagConfigMap[strings.ToLower(tc.Name)] = tc
			}

			type tagCountItem struct {
				name  string
				count int
			}
			var sortedTags []tagCountItem
			for name, count := range tagCounts {
				sortedTags = append(sortedTags, tagCountItem{name: name, count: count})
			}
			for i := 0; i < len(sortedTags)-1; i++ {
				for j := i + 1; j < len(sortedTags); j++ {
					if sortedTags[i].count < sortedTags[j].count || (sortedTags[i].count == sortedTags[j].count && sortedTags[i].name > sortedTags[j].name) {
						sortedTags[i], sortedTags[j] = sortedTags[j], sortedTags[i]
					}
				}
			}

			for _, tci := range sortedTags {
				if len(lines) >= maxContentLines {
					break
				}
				tc, ok := tagConfigMap[tci.name]
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
				tagStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(colorHex)).Bold(true)
				cntStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89"))
				tagRow := fmt.Sprintf("%s %s %s", emoji, tagStyle.Render("#"+tci.name), cntStyle.Render(fmt.Sprintf("(%d)", tci.count)))
				lines = append(lines, truncateOrPad(tagRow, contentWidth))
			}
		}
	}

	if len(lines) > maxContentLines {
		lines = lines[:maxContentLines]
	} else {
		for len(lines) < maxContentLines {
			lines = append(lines, "")
		}
	}

	// Ensure each line is padded to contentWidth
	for i, l := range lines {
		lines[i] = padLineToWidth(l, contentWidth)
	}

	return panelStyle.Render(strings.Join(lines, "\n"))
}

func truncateOrPad(s string, width int) string {
	if idx := strings.IndexByte(s, '\n'); idx != -1 {
		s = s[:idx]
	}
	w := lipgloss.Width(s)
	if w > width {
		rendered := lipgloss.NewStyle().MaxWidth(width).Render(s)
		if idx := strings.IndexByte(rendered, '\n'); idx != -1 {
			rendered = rendered[:idx]
		}
		return rendered
	}
	return s
}

func padLineToWidth(s string, width int) string {
	if idx := strings.IndexByte(s, '\n'); idx != -1 {
		s = s[:idx]
	}
	w := lipgloss.Width(s)
	if w < width {
		return s + strings.Repeat(" ", width-w)
	}
	if w > width {
		rendered := lipgloss.NewStyle().MaxWidth(width).Render(s)
		if idx := strings.IndexByte(rendered, '\n'); idx != -1 {
			rendered = rendered[:idx]
		}
		return rendered
	}
	return s
}
