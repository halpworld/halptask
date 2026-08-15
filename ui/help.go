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
				{"h / l", "Close fold / Open fold"},
				{"gg / G", "Go to top / bottom"},
				{"gi / <space>gi", "Jump to item by ID 🔢"},
				{"ff", "Zoom focused subtree"},
				{"Tab / S-Tab", "Indent / Unindent bullet"},
				{"J / K", "Move bullet down / up"},
			},
		},
		{
			Title: "Bullet & Task Operations",
			Items: [][2]string{
				{"o / O", "New bullet below / above"},
				{"i / a / e", "Edit bullet text"},
				{"c", "Clear & edit text"},
				{"t", "Toggle task / cycle status"},
				{"Esc / q / fo", "Exit Focus Mode 🎯"},
				{"N / <space>n", "Open task note 📝"},
				{"<space> t D", "Toggle default item type"},
				{"T / <space>ta", "Manage task tags & labels"},
				{"fc", "Toggle hide completed"},
				{"da", "Delete completed tasks"},
				{"dd / x", "Delete bullet & subtree"},
			},
		},
		{
			Title: "Folds, Save & Search",
			Items: [][2]string{
				{"Enter / za", "Toggle fold state"},
				{"zc / zo", "Close fold / Open fold"},
				{"zM / zR", "Close all / Open all folds"},
				{"ww", "Quick save data file"},
				{"/", "Search bullet points"},
				{"u / Ctrl+r", "Undo / Redo changes"},
			},
		},
		{
			Title: "Archive Operations",
			Items: [][2]string{
				{"<space> a a", "Archive selected item"},
				{"<space> a c", "Archive completed tasks"},
				{"<space> a v/r", "Open Archive View modal"},
			},
		},
		{
			Title: "Leader Commands (<space>)",
			Items: [][2]string{
				{"<space> b ...", "Bullet actions (new, edit, move)"},
				{"<space> t ...", "Task actions (toggle, status)"},
				{"<space> a ...", "Archive actions (view modal)"},
				{"<space> c ...", "Config dashboard & toggles"},
				{"<space> z ...", "Fold actions (open, close)"},
				{"<space> e ...", "Encryption settings"},
				{"<space> w / s", "Save data file to disk"},
				{"<space> q", "Quit HalpTask"},
			},
		},
	}

	modalWidth := width - 6
	if modalWidth > 116 {
		modalWidth = 116
	}
	if modalWidth < 50 {
		modalWidth = 50
	}

	footer := lipgloss.NewStyle().Foreground(lipgloss.Color("#565f89")).Render("Press Esc, ?, or q to close this help window.")

	var bodyContent string

	if modalWidth >= 80 {
		// Dual column layout
		colWidth := (modalWidth - 6) / 2
		if colWidth < 36 {
			colWidth = 36
		}

		dualKeyStyle := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#e0af68")).Width(14)

		var col1Lines []string
		for _, sec := range []struct {
			Title string
			Items [][2]string
		}{sections[0], sections[1]} {
			col1Lines = append(col1Lines, catHeaderStyle.Render(sec.Title))
			for _, item := range sec.Items {
				k := dualKeyStyle.Render(item[0])
				d := descStyle.Render(item[1])
				col1Lines = append(col1Lines, fmt.Sprintf("  %s %s", k, d))
			}
			col1Lines = append(col1Lines, "")
		}

		var col2Lines []string
		for _, sec := range []struct {
			Title string
			Items [][2]string
		}{sections[2], sections[3], sections[4]} {
			col2Lines = append(col2Lines, catHeaderStyle.Render(sec.Title))
			for _, item := range sec.Items {
				k := dualKeyStyle.Render(item[0])
				d := descStyle.Render(item[1])
				col2Lines = append(col2Lines, fmt.Sprintf("  %s %s", k, d))
			}
			col2Lines = append(col2Lines, "")
		}

		for i, l := range col1Lines {
			col1Lines[i] = padLineToWidth(l, colWidth)
		}
		for i, l := range col2Lines {
			col2Lines[i] = padLineToWidth(l, colWidth)
		}

		leftCol := strings.Join(col1Lines, "\n")
		rightCol := strings.Join(col2Lines, "\n")

		bodyContent = lipgloss.JoinHorizontal(lipgloss.Top, leftCol, "  ", rightCol)
	} else {
		// Single column layout for compact terminals
		singleKeyStyle := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#e0af68")).Width(18)
		var contentLines []string
		for _, sec := range sections {
			contentLines = append(contentLines, catHeaderStyle.Render(sec.Title))
			for _, item := range sec.Items {
				k := singleKeyStyle.Render(item[0])
				d := descStyle.Render(item[1])
				contentLines = append(contentLines, fmt.Sprintf("  %s %s", k, d))
			}
			contentLines = append(contentLines, "")
		}
		bodyContent = strings.Join(contentLines, "\n")
	}

	boxStyle := lipgloss.NewStyle().
		Border(lipgloss.DoubleBorder()).
		BorderForeground(lipgloss.Color("#bb9af7")).
		Padding(1, 2).
		Width(modalWidth)

	fullText := fmt.Sprintf("%s\n\n%s\n\n%s", header, bodyContent, footer)
	return boxStyle.Render(fullText)
}
