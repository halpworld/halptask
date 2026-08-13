package ui

import (
	"regexp"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
)

var (
	termResponseKeyRegex = regexp.MustCompile(`(?i)(?:\x1b\]|\])\d+;rgb:|(?:\x1b\[|\[)\??\d+;\d+[cR]`)
)

// IsTerminalEscapeResponseMsg returns true if the Bubbletea key message is a leaked terminal escape response
// such as background color queries ("]11;rgb:2828/2c2c/3434\") or CPR cursor position reports ("[53;1R").
func IsTerminalEscapeResponseMsg(msg tea.KeyMsg) bool {
	s := msg.String()
	runesStr := string(msg.Runes)

	if strings.HasPrefix(s, "]11;") || strings.HasPrefix(s, "]10;") || strings.HasPrefix(s, "]4;") || strings.HasPrefix(s, "]12;") ||
		strings.HasPrefix(s, "\x1b]11;") || strings.HasPrefix(s, "\x1b]10;") || strings.HasPrefix(s, "\x1b]4;") || strings.HasPrefix(s, "\x1b]12;") {
		return true
	}

	if (strings.HasPrefix(s, "[") || strings.HasPrefix(s, "\x1b[")) && strings.HasSuffix(s, "R") && strings.Contains(s, ";") {
		return true
	}

	if termResponseKeyRegex.MatchString(s) || termResponseKeyRegex.MatchString(runesStr) {
		return true
	}

	return false
}
