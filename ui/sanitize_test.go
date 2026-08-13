package ui

import (
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

func TestIsTerminalEscapeResponseMsg(t *testing.T) {
	tests := []struct {
		name     string
		msg      tea.KeyMsg
		expected bool
	}{
		{
			name:     "OSC 11 background color response runes",
			msg:      tea.KeyMsg{Runes: []rune("]11;rgb:2828/2c2c/3434\\")},
			expected: true,
		},
		{
			name:     "CPR cursor position report runes",
			msg:      tea.KeyMsg{Runes: []rune("[53;1R")},
			expected: true,
		},
		{
			name:     "Escaped CPR response",
			msg:      tea.KeyMsg{Runes: []rune("\x1b[53;1R")},
			expected: true,
		},
		{
			name:     "Normal letter typed",
			msg:      tea.KeyMsg{Runes: []rune("a")},
			expected: false,
		},
		{
			name:     "Normal enter key",
			msg:      tea.KeyMsg{Type: tea.KeyEnter},
			expected: false,
		},
		{
			name:     "Normal esc key",
			msg:      tea.KeyMsg{Type: tea.KeyEsc},
			expected: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := IsTerminalEscapeResponseMsg(tt.msg)
			if got != tt.expected {
				t.Errorf("IsTerminalEscapeResponseMsg(%v) = %v, expected %v", tt.msg, got, tt.expected)
			}
		})
	}
}
