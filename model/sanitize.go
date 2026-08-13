package model

import (
	"regexp"
	"strings"
)

var (
	// Matches OSC color responses e.g. "]11;rgb:2828/2c2c/3434\", "\x1b]10;rgb:1a1a/1b1b/2626\x07", "]11;rgb:12/34/56"
	oscColorRegex = regexp.MustCompile(`(?i)(?:\x1b\]|\])\d+;rgb:[0-9a-f/]+(?:\\|\x07|\x1b\\)?`)

	// Matches partial/incomplete OSC color response prefixes e.g. "]11;rgb:2828/2c2c/3434"
	oscColorPartialRegex = regexp.MustCompile(`(?i)(?:\x1b\]|\])\d+;rgb:[0-9a-f/]*`)

	// Matches CPR (Cursor Position Report) and DA responses e.g. "[53;1R", "\x1b[53;1R", "[?1;2c"
	cprRegex = regexp.MustCompile(`(?:\x1b\[|\[)\??\d+;\d+[cR]`)
)

// SanitizeTerminalEscapeArtifacts removes leaked terminal query escape sequence responses
// such as OSC background color queries ("]11;rgb:2828/2c2c/3434\") and CPR cursor position reports ("[53;1R").
func SanitizeTerminalEscapeArtifacts(text string) string {
	if text == "" {
		return text
	}
	result := text
	result = oscColorRegex.ReplaceAllString(result, "")
	result = oscColorPartialRegex.ReplaceAllString(result, "")
	result = cprRegex.ReplaceAllString(result, "")
	if strings.HasPrefix(result, "\n") && !strings.HasPrefix(text, "\n") {
		result = strings.TrimPrefix(result, "\n")
	}
	return result
}
