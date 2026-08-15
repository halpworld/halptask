package ui

import (
	"fmt"
	"path/filepath"

	"github.com/charmbracelet/lipgloss"
	"github.com/halpworld/halptask/model"
)

type AppMode int

const (
	ModeNormal AppMode = iota
	ModeInsert
	ModeSearch
	ModePrompt
	ModeHelp
	ModeTagPicker
	ModeConfig
	ModeJumpToID
	ModeArchive
	ModeNote
)

func (m AppMode) String() string {
	switch m {
	case ModeNormal:
		return "NORMAL"
	case ModeInsert:
		return "INSERT"
	case ModeSearch:
		return "SEARCH"
	case ModePrompt:
		return "PROMPT"
	case ModeHelp:
		return "HELP"
	case ModeTagPicker:
		return "TAGS"
	case ModeConfig:
		return "CONFIG"
	case ModeJumpToID:
		return "JUMP"
	case ModeArchive:
		return "ARCHIVE"
	case ModeNote:
		return "NOTE"
	default:
		return "NORMAL"
	}
}

type StatusBar struct {
	Width int
}

func NewStatusBar() StatusBar {
	return StatusBar{Width: 80}
}

func (sb *StatusBar) Render(mode AppMode, filePath string, isEncrypted bool, stats model.TaskStats, currentLine, totalLines int, statusMsg string, updateBadge string) string {
	width := sb.Width
	if width <= 0 {
		width = 80
	}

	// Styles
	var modeStyle lipgloss.Style
	switch mode {
	case ModeNormal:
		modeStyle = lipgloss.NewStyle().
			Bold(true).
			Background(lipgloss.Color("#7aa2f7")).
			Foreground(lipgloss.Color("#1a1b26")).
			Padding(0, 1)
	case ModeInsert:
		modeStyle = lipgloss.NewStyle().
			Bold(true).
			Background(lipgloss.Color("#9ece6a")).
			Foreground(lipgloss.Color("#1a1b26")).
			Padding(0, 1)
	case ModeSearch:
		modeStyle = lipgloss.NewStyle().
			Bold(true).
			Background(lipgloss.Color("#e0af68")).
			Foreground(lipgloss.Color("#1a1b26")).
			Padding(0, 1)
	default:
		modeStyle = lipgloss.NewStyle().
			Bold(true).
			Background(lipgloss.Color("#bb9af7")).
			Foreground(lipgloss.Color("#1a1b26")).
			Padding(0, 1)
	}

	fileStyle := lipgloss.NewStyle().
		Background(lipgloss.Color("#24283b")).
		Foreground(lipgloss.Color("#c0caf5")).
		Padding(0, 1)

	var encStr string
	if isEncrypted {
		encStr = lipgloss.NewStyle().
			Background(lipgloss.Color("#f7768e")).
			Foreground(lipgloss.Color("#1a1b26")).
			Bold(true).
			Padding(0, 1).
			Render("🔒 ENCRYPTED")
	} else {
		encStr = lipgloss.NewStyle().
			Background(lipgloss.Color("#414868")).
			Foreground(lipgloss.Color("#a9b1d6")).
			Padding(0, 1).
			Render("🔓 PLAIN")
	}

	// Task statistics string
	pct := 0
	if stats.Total > 0 {
		pct = (stats.Done * 100) / stats.Total
	}
	statsStr := fmt.Sprintf("Tasks: %d/%d (%d%%)", stats.Done, stats.Total, pct)
	statsBlock := lipgloss.NewStyle().
		Background(lipgloss.Color("#3b4261")).
		Foreground(lipgloss.Color("#7dcfff")).
		Padding(0, 1).
		Render(statsStr)

	posStr := fmt.Sprintf("%d/%d", currentLine, totalLines)
	posBlock := lipgloss.NewStyle().
		Background(lipgloss.Color("#7aa2f7")).
		Foreground(lipgloss.Color("#1a1b26")).
		Bold(true).
		Padding(0, 1).
		Render(posStr)

	msgStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#e0af68")).
		Italic(true).
		Padding(0, 1)

	modeSection := modeStyle.Render(mode.String())
	fileName := filepath.Base(filePath)
	fileSection := fileStyle.Render(fileName)

	leftFixed := lipgloss.JoinHorizontal(lipgloss.Center, modeSection, fileSection, encStr)
	if updateBadge != "" {
		badgeSection := lipgloss.NewStyle().
			Background(lipgloss.Color("#bb9af7")).
			Foreground(lipgloss.Color("#1a1b26")).
			Bold(true).
			Padding(0, 1).
			Render(updateBadge)
		leftFixed = lipgloss.JoinHorizontal(lipgloss.Center, leftFixed, badgeSection)
	}

	rightSide := lipgloss.JoinHorizontal(lipgloss.Center, statsBlock, posBlock)
	rightWidth := lipgloss.Width(rightSide)
	leftWidth := lipgloss.Width(leftFixed)

	leftSide := leftFixed

	if statusMsg != "" && width >= 65 {
		availMsgWidth := width - leftWidth - rightWidth - 2
		if availMsgWidth >= 8 {
			msgText := statusMsg
			if lipgloss.Width(msgText) > availMsgWidth {
				runes := []rune(msgText)
				if len(runes) > availMsgWidth-3 && availMsgWidth > 3 {
					msgText = string(runes[:availMsgWidth-3]) + "..."
				}
			}
			msgSection := msgStyle.Render(msgText)
			leftSide = lipgloss.JoinHorizontal(lipgloss.Center, leftFixed, msgSection)
			leftWidth = lipgloss.Width(leftSide)
		}
	}

	midSpaces := width - leftWidth - rightWidth
	if midSpaces < 0 {
		// If still too tight on narrow screens, use compact left side
		leftSide = lipgloss.JoinHorizontal(lipgloss.Center, modeSection, encStr)
		leftWidth = lipgloss.Width(leftSide)
		midSpaces = width - leftWidth - rightWidth
		if midSpaces < 0 {
			midSpaces = 0
		}
	}

	middle := lipgloss.NewStyle().
		Background(lipgloss.Color("#1f2335")).
		Width(midSpaces).
		Render("")

	return lipgloss.JoinHorizontal(lipgloss.Center, leftSide, middle, rightSide)
}
