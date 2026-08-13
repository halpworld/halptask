package ui

import (
	"fmt"
	"regexp"
	"strings"

	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/kenth/halptask/model"
)

type NoteModalMode int

const (
	NoteModeView NoteModalMode = iota
	NoteModeEdit
)

type NoteLink struct {
	TargetID string
	Display  string
	StartPos int
	EndPos   int
}

type NoteModal struct {
	Width      int
	Height     int
	Item       *model.Item
	Tree       *model.Tree
	Mode       NoteModalMode
	TextArea   textarea.Model
	Viewport   viewport.Model
	Links      []NoteLink
	ActiveLink int
	StatusMsg  string
}

func NewNoteModal(width, height int) *NoteModal {
	ta := textarea.New()
	ta.Placeholder = "Write markdown note here...\nSupports task links: #123, [Task Title](#123), [Task Title](123), task:123"
	ta.ShowLineNumbers = true

	vp := viewport.New(10, 10)

	nm := &NoteModal{
		Width:      width,
		Height:     height,
		Mode:       NoteModeView,
		TextArea:   ta,
		Viewport:   vp,
		ActiveLink: 0,
	}
	nm.SetSize(width, height)
	return nm
}

func (nm *NoteModal) SetSize(width, height int) {
	nm.Width = width
	nm.Height = height

	modalWidth := width - 10
	if modalWidth < 30 {
		modalWidth = 30
	}
	modalHeight := height - 6
	if modalHeight < 10 {
		modalHeight = 10
	}

	contentWidth := modalWidth - 4
	contentHeight := modalHeight - 4

	nm.TextArea.SetWidth(contentWidth)
	nm.TextArea.SetHeight(contentHeight)

	nm.Viewport.Width = contentWidth
	nm.Viewport.Height = contentHeight
}

func (nm *NoteModal) SetItem(item *model.Item, tree *model.Tree) {
	nm.Item = item
	nm.Tree = tree
	nm.Mode = NoteModeView
	nm.StatusMsg = ""
	nm.ActiveLink = 0

	if item != nil {
		nm.TextArea.SetValue(item.Note)
	} else {
		nm.TextArea.SetValue("")
	}
	nm.refreshLinksAndRender()

	// If empty note, default directly to Edit mode for convenience
	if item != nil && strings.TrimSpace(item.Note) == "" {
		nm.Mode = NoteModeEdit
		nm.TextArea.Focus()
	} else {
		nm.Mode = NoteModeView
		nm.TextArea.Blur()
	}
}

// ExtractLinks parses task links in formats:
// 1. [Label](#123) or [Label](123)
// 2. #123 or task:123
func ExtractLinks(text string) []NoteLink {
	var links []NoteLink
	if text == "" {
		return links
	}

	// Pattern 1: Markdown links [label](#id) or [label](id) or [label](task:id)
	mdLinkRegex := regexp.MustCompile(`\[([^\]]+)\]\((?:#|task:)?([a-zA-Z0-9_-]+)\)`)
	matches := mdLinkRegex.FindAllStringSubmatchIndex(text, -1)

	matchedRanges := [][2]int{}

	for _, m := range matches {
		if len(m) >= 6 {
			fullStart, fullEnd := m[0], m[1]
			label := text[m[2]:m[3]]
			targetID := text[m[4]:m[5]]
			links = append(links, NoteLink{
				TargetID: targetID,
				Display:  fmt.Sprintf("[%s](#%s)", label, targetID),
				StartPos: fullStart,
				EndPos:   fullEnd,
			})
			matchedRanges = append(matchedRanges, [2]int{fullStart, fullEnd})
		}
	}

	// Pattern 2: Standalone #id or task:id not inside markdown link bracket
	standaloneRegex := regexp.MustCompile(`(?:^|[\s,.(])(?:#|task:)([a-zA-Z0-9_-]+)`)
	matches2 := standaloneRegex.FindAllStringSubmatchIndex(text, -1)

	for _, m := range matches2 {
		if len(m) >= 4 {
			targetID := text[m[2]:m[3]]
			fullStart, fullEnd := m[0], m[1]

			// Adjust start if prefix character was captured
			prefixChar := text[m[0]:m[2]]
			if idx := strings.Index(prefixChar, "#"); idx != -1 {
				fullStart = m[0] + idx
			} else if idx := strings.Index(prefixChar, "task:"); idx != -1 {
				fullStart = m[0] + idx
			}

			// Check overlap with markdown links
			overlap := false
			for _, r := range matchedRanges {
				if fullStart >= r[0] && fullEnd <= r[1] {
					overlap = true
					break
				}
			}
			if !overlap {
				links = append(links, NoteLink{
					TargetID: targetID,
					Display:  fmt.Sprintf("#%s", targetID),
					StartPos: fullStart,
					EndPos:   fullEnd,
				})
			}
		}
	}

	return links
}

func (nm *NoteModal) refreshLinksAndRender() {
	noteText := ""
	if nm.Item != nil {
		noteText = nm.Item.Note
	}
	nm.Links = ExtractLinks(noteText)
	if nm.ActiveLink >= len(nm.Links) {
		if len(nm.Links) > 0 {
			nm.ActiveLink = len(nm.Links) - 1
		} else {
			nm.ActiveLink = 0
		}
	}

	rendered := nm.RenderMarkdownView(noteText)
	nm.Viewport.SetContent(rendered)
}

func (nm *NoteModal) RenderMarkdownView(text string) string {
	if strings.TrimSpace(text) == "" {
		emptyStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#565f89")).
			Italic(true)
		return emptyStyle.Render("(No note content yet. Press 'e' or 'i' to edit this note.)")
	}

	// Lipgloss styles for rendering markdown
	h1Style := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#7aa2f7"))
	h2Style := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#bb9af7"))
	h3Style := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#7dcfff"))
	bulletStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#9ece6a"))
	codeStyle := lipgloss.NewStyle().Background(lipgloss.Color("#24283b")).Foreground(lipgloss.Color("#9ece6a"))
	quoteStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#e0af68")).Italic(true)

	normalLinkStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#7dcfff")).
		Underline(true)

	activeLinkStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#7dcfff")).
		Foreground(lipgloss.Color("#1a1b26")).
		Bold(true).
		Underline(true)

	lines := strings.Split(text, "\n")
	var formattedLines []string

	inCodeBlock := false

	for _, line := range lines {
		trimmed := strings.TrimSpace(line)

		// Code block toggle
		if strings.HasPrefix(trimmed, "```") {
			inCodeBlock = !inCodeBlock
			formattedLines = append(formattedLines, codeStyle.Render(line))
			continue
		}

		if inCodeBlock {
			formattedLines = append(formattedLines, codeStyle.Render("  "+line))
			continue
		}

		// Headers
		if strings.HasPrefix(trimmed, "# ") {
			formattedLines = append(formattedLines, h1Style.Render(trimmed))
			continue
		}
		if strings.HasPrefix(trimmed, "## ") {
			formattedLines = append(formattedLines, h2Style.Render(trimmed))
			continue
		}
		if strings.HasPrefix(trimmed, "### ") {
			formattedLines = append(formattedLines, h3Style.Render(trimmed))
			continue
		}

		// Quotes
		if strings.HasPrefix(trimmed, "> ") {
			formattedLines = append(formattedLines, quoteStyle.Render(line))
			continue
		}

		// Bullets
		if strings.HasPrefix(trimmed, "- ") || strings.HasPrefix(trimmed, "* ") || strings.HasPrefix(trimmed, "+ ") {
			prefix := trimmed[:2]
			rest := trimmed[2:]
			line = bulletStyle.Render(prefix) + rest
		}

		// Format links in line
		for linkIdx, link := range nm.Links {
			linkText := link.Display
			targetLabel := link.Display
			if strings.HasPrefix(linkText, "[") {
				// extract label from [label](#id)
				if closeBracket := strings.Index(linkText, "]"); closeBracket != -1 {
					targetLabel = linkText[1:closeBracket]
				}
			}

			// Check if link target exists in tree
			targetExists := false
			if nm.Tree != nil && nm.Tree.FindItem(link.TargetID) != nil {
				targetExists = true
			}

			var renderedLink string
			if linkIdx == nm.ActiveLink {
				if targetExists {
					renderedLink = activeLinkStyle.Render(fmt.Sprintf("▶ %s (#%s)", targetLabel, link.TargetID))
				} else {
					renderedLink = activeLinkStyle.Render(fmt.Sprintf("▶ %s (#%s?)", targetLabel, link.TargetID))
				}
			} else {
				if targetExists {
					renderedLink = normalLinkStyle.Render(fmt.Sprintf("%s (#%s)", targetLabel, link.TargetID))
				} else {
					renderedLink = normalLinkStyle.Render(fmt.Sprintf("%s (#%s?)", targetLabel, link.TargetID))
				}
			}

			// Substitute link text representation
			if strings.Contains(line, link.Display) {
				line = strings.Replace(line, link.Display, renderedLink, 1)
			} else if strings.Contains(line, "#"+link.TargetID) {
				line = strings.Replace(line, "#"+link.TargetID, renderedLink, 1)
			}
		}

		formattedLines = append(formattedLines, line)
	}

	return strings.Join(formattedLines, "\n")
}

// Update returns (newModel, cmd, targetJumpID)
// targetJumpID is non-empty when user follows a task link!
func (nm *NoteModal) Update(msg tea.Msg) (*NoteModal, tea.Cmd, string) {
	var cmd tea.Cmd

	switch nm.Mode {
	case NoteModeView:
		if keyMsg, ok := msg.(tea.KeyMsg); ok {
			switch keyMsg.String() {
			case "e", "i":
				nm.Mode = NoteModeEdit
				nm.TextArea.Focus()
				nm.StatusMsg = "Editing note (Esc or Ctrl+S to save)"
				return nm, textarea.Blink, ""

			case "tab", "l", "n", "right", "down":
				if len(nm.Links) > 0 {
					nm.ActiveLink = (nm.ActiveLink + 1) % len(nm.Links)
					nm.refreshLinksAndRender()
				}
				return nm, nil, ""

			case "shift+tab", "h", "p", "left", "up":
				if len(nm.Links) > 0 {
					nm.ActiveLink = (nm.ActiveLink - 1 + len(nm.Links)) % len(nm.Links)
					nm.refreshLinksAndRender()
				}
				return nm, nil, ""

			case "enter":
				if len(nm.Links) > 0 && nm.ActiveLink >= 0 && nm.ActiveLink < len(nm.Links) {
					targetID := nm.Links[nm.ActiveLink].TargetID
					return nm, nil, targetID
				}
			}
		}
		nm.Viewport, cmd = nm.Viewport.Update(msg)
		return nm, cmd, ""

	case NoteModeEdit:
		if keyMsg, ok := msg.(tea.KeyMsg); ok {
			switch keyMsg.String() {
			case "esc", "ctrl+s":
				if nm.Item != nil {
					nm.Item.Note = nm.TextArea.Value()
				}
				nm.Mode = NoteModeView
				nm.TextArea.Blur()
				nm.refreshLinksAndRender()
				nm.StatusMsg = "Note saved!"
				return nm, nil, ""

			case "ctrl+c":
				if nm.Item != nil {
					nm.TextArea.SetValue(nm.Item.Note)
				}
				nm.Mode = NoteModeView
				nm.TextArea.Blur()
				nm.refreshLinksAndRender()
				nm.StatusMsg = "Edits cancelled"
				return nm, nil, ""
			}
		}
		nm.TextArea, cmd = nm.TextArea.Update(msg)
		return nm, cmd, ""
	}

	return nm, nil, ""
}

func (nm *NoteModal) View() string {
	modalWidth := nm.Width - 10
	if modalWidth < 35 {
		modalWidth = 35
	}
	modalHeight := nm.Height - 6
	if modalHeight < 12 {
		modalHeight = 12
	}

	titleText := "📝 Task Note"
	if nm.Item != nil {
		itemType := "Bullet"
		if nm.Item.IsTask {
			itemType = "Task"
		}
		titleText = fmt.Sprintf("📝 Note: %s #%s - %s", itemType, nm.Item.ID, nm.Item.Text)
	}

	modeBadge := "[VIEW MODE]"
	badgeStyle := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#7aa2f7"))
	if nm.Mode == NoteModeEdit {
		modeBadge = "[EDIT MODE]"
		badgeStyle = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("#ff9e64"))
	}

	titleStyle := lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("#c0caf5"))

	headerLine := fmt.Sprintf("%s %s", titleStyle.Render(titleText), badgeStyle.Render(modeBadge))

	var bodyView string
	if nm.Mode == NoteModeView {
		bodyView = nm.Viewport.View()
	} else {
		bodyView = nm.TextArea.View()
	}

	helpText := "[e/i] Edit  [Tab] Next Link  [Enter] Follow Link  [Esc] Close"
	if nm.Mode == NoteModeEdit {
		helpText = "[Esc / Ctrl+S] Save Note  [Ctrl+C] Cancel"
	}

	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#565f89")).
		Italic(true)

	if nm.StatusMsg != "" {
		statusStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#9ece6a")).Bold(true)
		helpText = fmt.Sprintf("%s  │  %s", statusStyle.Render(nm.StatusMsg), helpStyle.Render(helpText))
	} else {
		helpText = helpStyle.Render(helpText)
	}

	content := fmt.Sprintf("%s\n\n%s\n\n%s", headerLine, bodyView, helpText)

	dialogStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color("#7aa2f7")).
		Padding(1, 2).
		Width(modalWidth).
		Height(modalHeight)

	dialog := dialogStyle.Render(content)

	return lipgloss.Place(
		nm.Width,
		nm.Height,
		lipgloss.Center,
		lipgloss.Center,
		dialog,
	)
}
