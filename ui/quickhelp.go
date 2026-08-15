package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

type QuickHelpItem struct {
	Key   string
	Label string
}

type QuickHelp struct {
	Width int
}

func NewQuickHelp() QuickHelp {
	return QuickHelp{Width: 80}
}

// GetCommonShortcuts returns the essential shortcuts displayed in the unintrusive persistent bar.
func GetCommonShortcuts() []QuickHelpItem {
	return []QuickHelpItem{
		{Key: "<space>", Label: "leader"},
		{Key: "j/k", Label: "move"},
		{Key: "oo/oc", Label: "new/child"},
		{Key: "tab", Label: "indent"},
		{Key: "enter", Label: "fold"},
		{Key: "i", Label: "edit"},
		{Key: "t", Label: "task"},
		{Key: "fo", Label: "focus"},
		{Key: "T", Label: "tags"},
		{Key: "?", Label: "help"},
	}
}

func (qh QuickHelp) Render(width int) string {
	if width <= 0 {
		width = 80
	}

	keyStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#e0af68"))

	labelStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#a9b1d6"))

	sepStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#414868"))

	shortcuts := GetCommonShortcuts()
	if width < 60 {
		shortcuts = []QuickHelpItem{
			{Key: "<space>", Label: "leader"},
			{Key: "j/k", Label: "move"},
			{Key: "oo/oc", Label: "new/child"},
			{Key: "enter", Label: "fold"},
			{Key: "?", Label: "help"},
		}
	} else if width < 80 {
		shortcuts = []QuickHelpItem{
			{Key: "<space>", Label: "leader"},
			{Key: "j/k", Label: "move"},
			{Key: "oo/oc", Label: "new/child"},
			{Key: "enter", Label: "fold"},
			{Key: "t", Label: "task"},
			{Key: "?", Label: "help"},
		}
	} else if width >= 160 {
		shortcuts = []QuickHelpItem{
			{Key: "<space>", Label: "leader"},
			{Key: "j/k", Label: "move"},
			{Key: "oo/oc", Label: "new/child"},
			{Key: "tab", Label: "indent"},
			{Key: "enter", Label: "fold"},
			{Key: "i", Label: "edit"},
			{Key: "t", Label: "task"},
			{Key: "fo", Label: "focus"},
			{Key: "T", Label: "tags"},
			{Key: "N", Label: "note"},
			{Key: "u", Label: "undo"},
			{Key: "/", Label: "search"},
			{Key: "gi", Label: "jump"},
			{Key: "ff", Label: "zoom"},
			{Key: "<space> a v", Label: "archive"},
			{Key: "?", Label: "help"},
		}
	} else if width >= 120 {
		shortcuts = []QuickHelpItem{
			{Key: "<space>", Label: "leader"},
			{Key: "j/k", Label: "move"},
			{Key: "oo/oc", Label: "new/child"},
			{Key: "tab", Label: "indent"},
			{Key: "enter", Label: "fold"},
			{Key: "i", Label: "edit"},
			{Key: "t", Label: "task"},
			{Key: "fo", Label: "focus"},
			{Key: "T", Label: "tags"},
			{Key: "N", Label: "note"},
			{Key: "u", Label: "undo"},
			{Key: "/", Label: "search"},
			{Key: "?", Label: "help"},
		}
	}

	var parts []string
	for _, item := range shortcuts {
		k := keyStyle.Render(item.Key)
		l := labelStyle.Render(item.Label)
		parts = append(parts, fmt.Sprintf("%s %s", k, l))
	}

	sep := sepStyle.Render(" │ ")
	content := strings.Join(parts, sep)

	barStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#1f2335")).
		Width(width).
		Align(lipgloss.Center)

	return barStyle.Render(content)
}
