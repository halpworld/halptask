package ui

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/halpworld/halptask/model"
)

func TestArchiveModalOperations(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "halptask_archive_modal_test_*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	archivePath := filepath.Join(tempDir, "archive.dat")
	store := model.NewArchiveStore(archivePath, false)

	item1 := model.NewTask("1", "Archived Task One", model.StatusDone)
	item2 := model.NewItem("2", "Archived Bullet Two")

	entries := []*model.ArchivedEntry{
		{ID: "1", ArchivedAt: time.Now(), Context: "Work", Item: item1},
		{ID: "2", ArchivedAt: time.Now(), Context: "Home", Item: item2},
	}
	if err := store.Save(entries, ""); err != nil {
		t.Fatalf("Failed to save store: %v", err)
	}

	modal := NewArchiveModal(store)
	modal.Open(entries, "")

	if len(modal.Filtered) != 2 {
		t.Fatalf("Expected 2 filtered entries, got %d", len(modal.Filtered))
	}

	// Test navigation down
	modal.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'j'}})
	if modal.CursorIndex != 1 {
		t.Errorf("Expected cursor index 1 after 'j', got %d", modal.CursorIndex)
	}

	// Test restore item 2
	closeModal, restored, status := modal.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'r'}})
	if !closeModal {
		t.Errorf("Expected closeModal to be true on restore")
	}
	if restored == nil || restored.ID != "2" {
		t.Errorf("Expected restored entry ID 2, got %+v", restored)
	}
	if status == "" {
		t.Errorf("Expected non-empty status message on restore")
	}

	// Re-open modal and verify remaining items
	loaded, err := store.Load("")
	if err != nil {
		t.Fatalf("Failed to load store: %v", err)
	}
	if len(loaded) != 1 || loaded[0].ID != "1" {
		t.Fatalf("Expected 1 remaining entry with ID 1, got %+v", loaded)
	}

	modal.Open(loaded, "")
	// Test delete prompt triggers
	modal.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'d'}})
	if !modal.ConfirmDelete {
		t.Errorf("Expected ConfirmDelete to be true after pressing 'd'")
	}

	// Confirm delete with 'y'
	modal.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	if modal.ConfirmDelete {
		t.Errorf("Expected ConfirmDelete to be false after confirming delete")
	}

	loadedAfterDelete, err := store.Load("")
	if err != nil {
		t.Fatalf("Failed to load store after delete: %v", err)
	}
	if len(loadedAfterDelete) != 0 {
		t.Errorf("Expected 0 entries after deletion, got %d", len(loadedAfterDelete))
	}
}
