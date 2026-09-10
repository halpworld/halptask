package main

import (
	"bytes"
	"encoding/json"
	"github.com/halpworld/halptask/model"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
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
			name:      "long flag --help",
			args:      []string{"--help"},
			wantFlags: CLIFlags{Help: true},
		},
		{
			name:      "short flag -h",
			args:      []string{"-h"},
			wantFlags: CLIFlags{Help: true},
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

// Exercise the real dispatcher and exit codes without opening a terminal.
func TestCLIProcess(t *testing.T) {
	if os.Getenv("HALPTASK_TEST_PROCESS") == "1" {
		for i, arg := range os.Args {
			if arg == "--" {
				os.Args = append([]string{"halptask"}, os.Args[i+1:]...)
				main()
				return
			}
		}
		os.Exit(99)
	}
}

func TestHeadlessCLI(t *testing.T) {
	home := t.TempDir()
	run := func(pass string, wantCode int, args ...string) string {
		t.Helper()
		cmd := exec.Command(os.Args[0], append([]string{"-test.run=^TestCLIProcess$", "--"}, args...)...)
		cmd.Env = append(os.Environ(), "HOME="+home, "USERPROFILE="+home, "HALPTASK_TEST_PROCESS=1", "HALPTASK_PASSPHRASE="+pass)
		var stdout, stderr bytes.Buffer
		cmd.Stdout, cmd.Stderr = &stdout, &stderr
		err := cmd.Run()
		code := 0
		if err != nil {
			if e, ok := err.(*exec.ExitError); ok {
				code = e.ExitCode()
			} else {
				t.Fatal(err)
			}
		}
		if code != wantCode {
			t.Fatalf("%v: exit %d, want %d: %s", args, code, wantCode, stderr.String())
		}
		if strings.Contains(stdout.String(), "\x1b[") {
			t.Fatalf("ANSI output: %q", stdout.String())
		}
		return stdout.String()
	}
	path := filepath.Join(home, "tasks.pb")
	if got := run("", 0, "list", "-f", path, "--json"); strings.TrimSpace(got) != "[]" {
		t.Fatal(got)
	}
	run("", 0, "add", "Fix worker #ops due:tomorrow", "-f", path, "--tag", "urgent", "--tag", "meeting")
	run("", 0, "capture", "Hotfix due:yesterday", "--top", "-f", path)
	run("", 0, "add", "System note", "--bullet", "-f", path)
	run("", 0, "add", "-f", path, "--", "- [~] Working")
	run("", 0, "add", "-f", path, "--", "- [x] Finished due:today")
	if got := run("", 0, "ls", "--count", "-f", path); got != "📋 2 todo, 1 in-progress, 1 overdue\n" {
		t.Fatal(got)
	}
	var tasks []model.TaskItemJSON
	if err := json.Unmarshal([]byte(run("", 0, "list", "--today", "--json", "-f", path)), &tasks); err != nil {
		t.Fatal(err)
	}
	if len(tasks) != 2 || tasks[0].Text != "Hotfix" || tasks[1].Status != model.StatusInProgress {
		t.Fatalf("unexpected today tasks: %+v", tasks)
	}
	tree, err := model.NewStorage(path, false).Load("")
	if err != nil {
		t.Fatal(err)
	}
	if len(tree.Roots) != 1 || len(tree.Roots[0].Children) != 5 {
		t.Fatalf("unexpected tree: %+v", tree.Roots)
	}
	if tags := tree.Roots[0].Children[1].Tags; len(tags) != 3 {
		t.Fatal(tags)
	}
	run("", 1, "add", "--task", "--bullet", "Bad", "-f", path)
	run("", 1, "add", "-f", path)
	run("", 2, "list", "--unknown")
	vault := filepath.Join(home, "vault.pb")
	run("secret", 0, "add", "Secret one", "--encrypt", "-f", vault)
	run("secret", 0, "capture", "Secret two", "-f", vault)
	before, err := os.ReadFile(vault)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.HasPrefix(before, []byte(model.EncryptedHeader)) {
		t.Fatal("vault lost encryption")
	}
	run("wrong", 1, "add", "Must not save", "-f", vault)
	run("", 1, "list", "-f", vault)
	after, err := os.ReadFile(vault)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(before, after) {
		t.Fatal("failed command changed vault")
	}
	if got := run("secret", 0, "list", "--count", "-f", vault); got != "📋 2 todo\n" {
		t.Fatal(got)
	}
}
