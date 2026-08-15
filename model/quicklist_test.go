package model

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestParseDueDate(t *testing.T) {
	refTime := time.Date(2026, 8, 15, 12, 0, 0, 0, time.UTC) // Saturday, Aug 15, 2026

	tests := []struct {
		input    string
		wantYear int
		wantMon  time.Month
		wantDay  int
		wantOK   bool
	}{
		{"today", 2026, 8, 15, true},
		{"tdy", 2026, 8, 15, true},
		{"tomorrow", 2026, 8, 16, true},
		{"tmrw", 2026, 8, 16, true},
		{"yesterday", 2026, 8, 14, true},
		{"ydy", 2026, 8, 14, true},
		{"mon", 2026, 8, 17, true},    // next Monday (Aug 17)
		{"monday", 2026, 8, 17, true}, // next Monday
		{"sunday", 2026, 8, 16, true}, // tomorrow Sunday
		{"+1d", 2026, 8, 16, true},
		{"+2d", 2026, 8, 17, true},
		{"+1w", 2026, 8, 22, true},
		{"2026-08-20", 2026, 8, 20, true},
		{"2026/09/01", 2026, 9, 1, true},
		{"08-30", 2026, 8, 30, true},
		{"8/30", 2026, 8, 30, true},
		{"invalid-date", 0, 0, 0, false},
		{"", 0, 0, 0, false},
	}

	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			got, ok := ParseDueDate(tt.input, refTime)
			if ok != tt.wantOK {
				t.Fatalf("ParseDueDate(%q) ok = %v, want %v", tt.input, ok, tt.wantOK)
			}
			if ok {
				if got.Year() != tt.wantYear || got.Month() != tt.wantMon || got.Day() != tt.wantDay {
					t.Errorf("ParseDueDate(%q) = %v, want %04d-%02d-%02d", tt.input, got.Format("2006-01-02"), tt.wantYear, tt.wantMon, tt.wantDay)
				}
			}
		})
	}
}

func TestExtractDueDate(t *testing.T) {
	clean, rawDue, _, hasDue := ExtractDueDate("Fix login page bug #auth due:tomorrow")
	if !hasDue {
		t.Fatal("expected hasDue true")
	}
	if clean != "Fix login page bug #auth" {
		t.Errorf("unexpected clean title: %q", clean)
	}
	if rawDue != "tomorrow" {
		t.Errorf("unexpected rawDue: %q", rawDue)
	}
}

func TestRunQuickList(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "halptask_ql_test")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	dataPath := filepath.Join(tempDir, "data.pb")
	storage := NewStorage(dataPath, false)

	tree := NewTree()
	inbox := NewItem("1", "Inbox")

	t1 := NewTask("2", "Active task 1 #ops due:today", StatusTodo)
	t2 := NewTask("3", "In progress task #dev", StatusInProgress)
	t3 := NewTask("4", "Overdue task #urgent due:2020-01-01", StatusTodo)
	t4 := NewTask("5", "Done task", StatusDone)

	inbox.Children = append(inbox.Children, t1, t2, t3, t4)
	tree.Roots = append(tree.Roots, inbox)
	tree.EnsureIDs()
	tree.SetParents()

	if err := storage.Save(tree, ""); err != nil {
		t.Fatalf("failed to seed test data: %v", err)
	}

	// 1. Test --count
	t.Run("CountOnly", func(t *testing.T) {
		var buf bytes.Buffer
		opts := QuickListOptions{
			FilePath:  dataPath,
			CountOnly: true,
		}
		if err := RunQuickList(nil, opts, &buf); err != nil {
			t.Fatalf("RunQuickList CountOnly failed: %v", err)
		}
		out := strings.TrimSpace(buf.String())
		if !strings.Contains(out, "📋") || !strings.Contains(out, "2 todo") || !strings.Contains(out, "1 in-progress") || !strings.Contains(out, "1 overdue") {
			t.Errorf("unexpected count output: %q", out)
		}
	})

	// 2. Test --json
	t.Run("JSONOutput", func(t *testing.T) {
		var buf bytes.Buffer
		opts := QuickListOptions{
			FilePath:   dataPath,
			JSONOutput: true,
			All:        true,
		}
		if err := RunQuickList(nil, opts, &buf); err != nil {
			t.Fatalf("RunQuickList JSONOutput failed: %v", err)
		}
		var tasks []TaskItemJSON
		if err := json.Unmarshal(buf.Bytes(), &tasks); err != nil {
			t.Fatalf("failed to parse json output: %v\nOutput: %s", err, buf.String())
		}
		if len(tasks) != 4 {
			t.Fatalf("expected 4 tasks in json output, got %d", len(tasks))
		}
	})

	// 3. Test --today
	t.Run("TodayFilter", func(t *testing.T) {
		var buf bytes.Buffer
		opts := QuickListOptions{
			FilePath: dataPath,
			Today:    true,
			NoColor:  true,
		}
		if err := RunQuickList(nil, opts, &buf); err != nil {
			t.Fatalf("RunQuickList Today failed: %v", err)
		}
		out := buf.String()
		// Should include t1 (due today), t2 (in progress), and t3 (overdue)
		if !strings.Contains(out, "Active task 1") || !strings.Contains(out, "In progress task") || !strings.Contains(out, "Overdue task") {
			t.Errorf("unexpected today output:\n%s", out)
		}
		// Should NOT include t4 (done)
		if strings.Contains(out, "Done task") {
			t.Errorf("today output should not include done task:\n%s", out)
		}
	})

	// 4. Test --in-progress
	t.Run("InProgressFilter", func(t *testing.T) {
		var buf bytes.Buffer
		opts := QuickListOptions{
			FilePath:   dataPath,
			InProgress: true,
			NoColor:    true,
		}
		if err := RunQuickList(nil, opts, &buf); err != nil {
			t.Fatalf("RunQuickList InProgress failed: %v", err)
		}
		out := buf.String()
		if !strings.Contains(out, "In progress task") {
			t.Errorf("expected in progress task in output:\n%s", out)
		}
		if strings.Contains(out, "Active task 1") || strings.Contains(out, "Overdue task") {
			t.Errorf("in-progress output contained non-in-progress tasks:\n%s", out)
		}
	})

	// 5. Test Plain Output Formatting
	t.Run("PlainListing", func(t *testing.T) {
		var buf bytes.Buffer
		opts := QuickListOptions{
			FilePath: dataPath,
			NoColor:  true,
		}
		if err := RunQuickList(nil, opts, &buf); err != nil {
			t.Fatalf("RunQuickList plain failed: %v", err)
		}
		out := buf.String()
		if !strings.Contains(out, "HalpTask •") || !strings.Contains(out, "[ ]") || !strings.Contains(out, "[~]") {
			t.Errorf("unexpected listing format:\n%s", out)
		}
	})
}
