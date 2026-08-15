package model

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseCaptureText(t *testing.T) {
	tests := []struct {
		name       string
		input      string
		wantTitle  string
		wantTags   []string
		wantDue    string
		wantIsTask bool
		wantStatus TaskStatus
	}{
		{
			name:       "simple task text",
			input:      "Fix Redis connection timeout",
			wantTitle:  "Fix Redis connection timeout",
			wantTags:   nil,
			wantDue:    "",
			wantIsTask: true,
			wantStatus: StatusTodo,
		},
		{
			name:       "task with tags and due date",
			input:      "Fix Redis timeout #ops #backend due:tomorrow",
			wantTitle:  "Fix Redis timeout",
			wantTags:   []string{"ops", "backend"},
			wantDue:    "tomorrow",
			wantIsTask: true,
			wantStatus: StatusTodo,
		},
		{
			name:       "task with in-progress markdown prefix",
			input:      "- [~] Deploy staging hotfix #dev",
			wantTitle:  "Deploy staging hotfix",
			wantTags:   []string{"dev"},
			wantDue:    "",
			wantIsTask: true,
			wantStatus: StatusInProgress,
		},
		{
			name:       "task with completed markdown prefix",
			input:      "- [x] Submit expense report",
			wantTitle:  "Submit expense report",
			wantTags:   nil,
			wantDue:    "",
			wantIsTask: true,
			wantStatus: StatusDone,
		},
		{
			name:       "bullet prefix",
			input:      "- General system note",
			wantTitle:  "General system note",
			wantTags:   nil,
			wantDue:    "",
			wantIsTask: true, // will be modified by options if --bullet is passed
			wantStatus: StatusTodo,
		},
		{
			name:       "iso due date with due= syntax",
			input:      "Quarterly review #planning due=2026-09-01",
			wantTitle:  "Quarterly review",
			wantTags:   []string{"planning"},
			wantDue:    "2026-09-01",
			wantIsTask: true,
			wantStatus: StatusTodo,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			title, tags, due, isTask, status := ParseCaptureText(tt.input)
			if title != tt.wantTitle {
				t.Errorf("title mismatch: got %q, want %q", title, tt.wantTitle)
			}
			if len(tags) != len(tt.wantTags) {
				t.Fatalf("tags length mismatch: got %v, want %v", tags, tt.wantTags)
			}
			for i, tag := range tags {
				if tag != tt.wantTags[i] {
					t.Errorf("tag[%d] mismatch: got %q, want %q", i, tag, tt.wantTags[i])
				}
			}
			if due != tt.wantDue {
				t.Errorf("due mismatch: got %q, want %q", due, tt.wantDue)
			}
			if isTask != tt.wantIsTask {
				t.Errorf("isTask mismatch: got %v, want %v", isTask, tt.wantIsTask)
			}
			if status != tt.wantStatus {
				t.Errorf("status mismatch: got %v, want %v", status, tt.wantStatus)
			}
		})
	}
}

func TestFindOrCreateInbox(t *testing.T) {
	t.Run("creates new inbox when tree is empty", func(t *testing.T) {
		tree := NewTree()
		inbox := FindOrCreateInbox(tree, "Inbox")
		if inbox == nil {
			t.Fatal("expected non-nil inbox")
		}
		if inbox.Text != "Inbox" {
			t.Errorf("expected inbox text 'Inbox', got %q", inbox.Text)
		}
		if len(tree.Roots) != 1 {
			t.Fatalf("expected 1 root, got %d", len(tree.Roots))
		}
	})

	t.Run("finds existing root by text", func(t *testing.T) {
		tree := NewTree()
		existing := NewItem("1", "Inbox")
		tree.Roots = append(tree.Roots, existing)

		inbox := FindOrCreateInbox(tree, "Inbox")
		if inbox != existing {
			t.Errorf("expected to find existing inbox pointer")
		}
		if len(tree.Roots) != 1 {
			t.Fatalf("expected roots count to remain 1, got %d", len(tree.Roots))
		}
	})

	t.Run("finds existing root by tag or #inbox", func(t *testing.T) {
		tree := NewTree()
		existing := NewItem("1", "#inbox")
		tree.Roots = append(tree.Roots, existing)

		inbox := FindOrCreateInbox(tree, "Inbox")
		if inbox != existing {
			t.Errorf("expected to find existing #inbox pointer")
		}
	})
}

func TestRunQuickCapture(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "halptask_qc_test")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	dataPath := filepath.Join(tempDir, "data.pb")

	// 1. Basic Quick Capture
	opts := QuickCaptureOptions{
		FilePath: dataPath,
		RawText:  "Fix Redis connection timeout in worker #ops due:tomorrow",
	}

	item, msg, err := RunQuickCapture(nil, opts)
	if err != nil {
		t.Fatalf("RunQuickCapture failed: %v", err)
	}
	if item == nil {
		t.Fatal("expected non-nil item")
	}
	if !strings.Contains(item.Text, "Fix Redis connection timeout in worker") {
		t.Errorf("unexpected item text: %q", item.Text)
	}
	if len(item.Tags) != 1 || item.Tags[0] != "ops" {
		t.Errorf("unexpected tags: %v", item.Tags)
	}
	if !strings.Contains(msg, "✔ Added to Inbox") || !strings.Contains(msg, "[#ops]") || !strings.Contains(msg, "[Due: Tomorrow]") {
		t.Errorf("unexpected confirm message: %q", msg)
	}

	// 2. Prepend with --top and explicit --tag
	optsTop := QuickCaptureOptions{
		FilePath:   dataPath,
		RawText:    "Deploy urgent hotfix",
		Tags:       []string{"urgent", "prod"},
		PrependTop: true,
	}
	itemTop, msgTop, err := RunQuickCapture(nil, optsTop)
	if err != nil {
		t.Fatalf("RunQuickCapture top failed: %v", err)
	}
	if !strings.Contains(msgTop, "Added to top of Inbox") {
		t.Errorf("expected top message, got: %q", msgTop)
	}

	// Load storage and verify ordering
	storage := NewStorage(dataPath, false)
	tree, err := storage.Load("")
	if err != nil {
		t.Fatalf("failed to reload storage: %v", err)
	}
	inbox := FindOrCreateInbox(tree, "Inbox")
	if len(inbox.Children) != 2 {
		t.Fatalf("expected 2 inbox children, got %d", len(inbox.Children))
	}
	if inbox.Children[0].ID != itemTop.ID {
		t.Errorf("expected top item to be first child")
	}

	// 3. Bullet Point with --bullet
	optsBullet := QuickCaptureOptions{
		FilePath: dataPath,
		RawText:  "Architecture discussion notes",
		IsBullet: true,
	}
	itemBullet, msgBullet, err := RunQuickCapture(nil, optsBullet)
	if err != nil {
		t.Fatalf("RunQuickCapture bullet failed: %v", err)
	}
	if itemBullet.IsTask {
		t.Errorf("expected isTask false for bullet point")
	}
	if !strings.Contains(msgBullet, "Added bullet to Inbox") {
		t.Errorf("expected bullet message, got: %q", msgBullet)
	}

	// 4. Encrypted Quick Capture
	encPath := filepath.Join(tempDir, "encrypted.pb")
	passphrase := "secret123"
	optsEnc := QuickCaptureOptions{
		FilePath:   encPath,
		Encrypt:    true,
		Passphrase: passphrase,
		RawText:    "Confidential security key rotation #security",
	}
	_, _, err = RunQuickCapture(nil, optsEnc)
	if err != nil {
		t.Fatalf("RunQuickCapture encrypted failed: %v", err)
	}

	encStorage := NewStorage(encPath, true)
	encTree, err := encStorage.Load(passphrase)
	if err != nil {
		t.Fatalf("failed to load encrypted tree: %v", err)
	}
	encInbox := FindOrCreateInbox(encTree, "Inbox")
	if len(encInbox.Children) != 1 || !strings.Contains(encInbox.Children[0].Text, "Confidential security key rotation") {
		t.Errorf("unexpected encrypted inbox content")
	}
}
