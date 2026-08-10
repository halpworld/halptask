package main

import (
	"testing"
)

func TestParseFlags(t *testing.T) {
	tests := []struct {
		name        string
		args        []string
		wantFlags   CLIFlags
		expectError bool
	}{
		{
			name:      "default no flags",
			args:      []string{},
			wantFlags: CLIFlags{},
		},
		{
			name:      "long flag --file",
			args:      []string{"--file", "tasks.txt"},
			wantFlags: CLIFlags{FilePath: "tasks.txt"},
		},
		{
			name:      "short flag -f",
			args:      []string{"-f", "tasks.txt"},
			wantFlags: CLIFlags{FilePath: "tasks.txt"},
		},
		{
			name:      "long flag --encrypt",
			args:      []string{"--encrypt"},
			wantFlags: CLIFlags{Encrypt: true},
		},
		{
			name:      "short flag -e",
			args:      []string{"-e"},
			wantFlags: CLIFlags{Encrypt: true},
		},
		{
			name:      "long flag --version",
			args:      []string{"--version"},
			wantFlags: CLIFlags{Version: true},
		},
		{
			name:      "short flag -v",
			args:      []string{"-v"},
			wantFlags: CLIFlags{Version: true},
		},
		{
			name:      "long flag --update",
			args:      []string{"--update"},
			wantFlags: CLIFlags{Update: true},
		},
		{
			name:      "short flag -u",
			args:      []string{"-u"},
			wantFlags: CLIFlags{Update: true},
		},
		{
			name:      "long flag --check-update",
			args:      []string{"--check-update"},
			wantFlags: CLIFlags{CheckUpdate: true},
		},
		{
			name:      "short flag -c",
			args:      []string{"-c"},
			wantFlags: CLIFlags{CheckUpdate: true},
		},
		{
			name:      "long flag --repo",
			args:      []string{"--repo", "owner/repo"},
			wantFlags: CLIFlags{Repo: "owner/repo"},
		},
		{
			name:      "short flag -r",
			args:      []string{"-r", "owner/repo"},
			wantFlags: CLIFlags{Repo: "owner/repo"},
		},
		{
			name:      "combined flags -e -v -f custom.txt",
			args:      []string{"-e", "-v", "-f", "custom.txt"},
			wantFlags: CLIFlags{Encrypt: true, Version: true, FilePath: "custom.txt"},
		},
		{
			name:        "unknown flag --unknown",
			args:        []string{"--unknown"},
			expectError: true,
		},
		{
			name:        "unknown short flag -x",
			args:        []string{"-x"},
			expectError: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			flags, _, err := parseFlags(tt.args)
			if tt.expectError {
				if err == nil {
					t.Errorf("expected error for args %v, got nil", tt.args)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error for args %v: %v", tt.args, err)
			}
			if *flags != tt.wantFlags {
				t.Errorf("flags mismatch for %v:\n got: %+v\nwant: %+v", tt.args, *flags, tt.wantFlags)
			}
		})
	}
}
