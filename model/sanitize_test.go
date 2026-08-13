package model

import "testing"

func TestSanitizeTerminalEscapeArtifacts(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		expected string
	}{
		{
			name:     "User screenshot gibberish",
			input:    `]11;rgb:2828/2c2c/3434\[53;1R`,
			expected: ``,
		},
		{
			name:     "Gibberish before real note content",
			input:    `]11;rgb:2828/2c2c/3434\[53;1R` + "\n" + `My real note title`,
			expected: `My real note title`,
		},
		{
			name:     "Stray OSC 10 response",
			input:    `]10;rgb:1a1a/1b1b/2626\Hello world`,
			expected: `Hello world`,
		},
		{
			name:     "Stray CPR response",
			input:    `[53;1RHello world`,
			expected: `Hello world`,
		},
		{
			name:     "Normal note unchanged",
			input:    `Task #3 - Mark task in-progress with 'ts' or leader 't p'`,
			expected: `Task #3 - Mark task in-progress with 'ts' or leader 't p'`,
		},
		{
			name:     "Task markdown link unchanged",
			input:    `Referencing [Task One](#123) and [Task Two](123)`,
			expected: `Referencing [Task One](#123) and [Task Two](123)`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := SanitizeTerminalEscapeArtifacts(tt.input)
			if got != tt.expected {
				t.Errorf("SanitizeTerminalEscapeArtifacts(%q) = %q, expected %q", tt.input, got, tt.expected)
			}
		})
	}
}
