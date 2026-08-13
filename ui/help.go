package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

type HelpModal struct {
	Width  int
	Height int
}

func NewHelpModal() HelpModal {
	return HelpModal{Width: 80, Height: 24}
}

func (h *HelpModal) Render(width, height int) string {
	if width <= 0 {
		width = 80
	}

	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#1a1b26")).
		Background(lipgloss.Color("#bb9af7")).
		Padding(0, 1)

	header := titleStyle.Render(" HalpTask Keybindings Cheat Sheet ")

	catHeaderStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#7aa2f7")).
		Underline(true)

	keyStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#e0af68")).
		Width(16)

	descStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#c0caf5"))

	sections := []struct {
		Title string
		Items [][2]string
	}{
		{
			Title: "Vim Navigation & View",
			Items: [][2]string{
				{"j / k", "Move cursor down / up"},
				{"h / l", "Close fold (parent) / Open fold (child)"},
				{"gg / G", "Go to top / bottom of document"},
				{"gi / <space> g i", "Jump to item by ID 🔢"},
				{"ff", "Zoom / Hoist focused subtree"},
				{"Tab / Shift+Tab", "Indent / Unindent bullet point"},
				{"J / K", "Move bullet down / up"},
			},
		},
		{
			Title: "Bullet & Task Operations",
			Items: [][2]string{
				{"o / O", "Create new bullet below / above"},
				{"i / a / e", "Edit current bullet text"},
				{"c", "Clear line text & enter insert mode"},
				{"t", "Toggle bullet into task / Cycle status"},
				{"Esc / q / fo / tf", "Exit Focus Mode / Clear focus 🎯"},
				{"N / <space> n", "Open / edit task markdown note 📝"},
				{"<space> t D", "Toggle default item type (bullet/task)"},
				{"T / <space> t a", "Manage task tags & labels (emojis & colors)"},
				{"fc", "Toggle hide/show completed tasks"},
				{"da", "Delete all completed tasks"},
				{"dd / x", "Delete current bullet & sub-bullets"},
			},
		},
		{
			Title: "Folds, Save & Search",
			Items: [][2]string{
				{"Enter / za", "Toggle fold state"},
				{"zc / zo", "Close fold / Open fold"},
				{"zM / zR", "Close all folds / Open all folds"},
				{"ww", "Quick save data file to disk"},
				{"/", "Search bullet points"},
				{"u / Ctrl+r", "Undo / Redo changes"},
			},
		},
		{
			Title: "Archive Operations",
			Items: [][2]string{
				{"<space> a a", "Archive selected item & subtree"},
				{"<space> a c", "Archive all completed tasks"},
				{"<space> a v / r", "Open interactive Archive View modal"},
			},
		},
		{
			Title: "Leader Commands (<space>)",
			Items: [][2]string{
				{"<space> b ...", "Bullet actions (new, edit, indent, move)"},
				{"<space> t ...", "Task actions (toggle, done, in-progress)"},
				{"<space> a ...", "Archive actions (archive item/completed, view modal)"},
				{"<space> c ...", "Config dashboard & toggles (theme, auto-save, edit)"},
				{"<space> z ...", "Fold actions (close, open, toggle all)"},
				{"<space> e ...", "Encryption settings (toggle, passphrase)"},
				{"<space> w / s", "Save data file to disk"},
				{"<space> q", "Quit HalpTask"},
			},
		},
	}

	var contentLines []string
	for _, sec := range sections {
		contentLines = append(contentLines, catHeaderStyle.Render(sec.Title))
		for _, item := range sec.Items {
			k := keyStyle.Render(item[0])
			d := descStyle.Render(item[1])
			contentLines = append(contentLines, fmt.Sprintf("  %s %s", k, d))
		}
		contentLines = append(contentLines, "")
	}

	contentLines = append(contentLines, lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("Press Esc or ? to close this help window."))

	boxStyle := lipgloss.NewStyle().
		Border(lipgloss.DoubleBorder()).
		BorderForeground(lipgloss.Color("#bb9af7")).
		Padding(1, 2).
		Width(width - 6)

	fullText := fmt.Sprintf("%s\n\n%s", header, strings.Join(contentLines, "\n"))
	return boxStyle.Render(fullText)
}
