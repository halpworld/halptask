package model

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestArchiveStoreUnencrypted(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "halptask_archive_test_*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	archivePath := filepath.Join(tempDir, "archive.dat")
	store := NewArchiveStore(archivePath, false)

	// Load empty store
	entries, err := store.Load("")
	if err != nil {
		t.Fatalf("Failed to load empty archive: %v", err)
	}
	if len(entries) != 0 {
		t.Fatalf("Expected 0 entries, got %d", len(entries))
	}

	// Create test item
	item := NewTask("1", "Archived task 1", StatusDone)
	item.AddTag("work")
	child := NewItem("2", "Child bullet")
	item.Children = append(item.Children, child)

	testEntry := &ArchivedEntry{
		ID:         item.ID,
		ArchivedAt: time.Now().Truncate(time.Second),
		Context:    "Project Alpha > Sprint 1",
		Item:       item,
	}

	// Save entry
	if err := store.Save([]*ArchivedEntry{testEntry}, ""); err != nil {
		t.Fatalf("Failed to save archive: %v", err)
	}

	// Verify file is created and compressed
	info, err := os.Stat(archivePath)
	if err != nil {
		t.Fatalf("Failed to stat archive file: %v", err)
	}
	if info.Size() == 0 {
		t.Fatalf("Archive file is 0 bytes")
	}

	// Load entries back
	loadedEntries, err := store.Load("")
	if err != nil {
		t.Fatalf("Failed to load saved archive: %v", err)
	}
	if len(loadedEntries) != 1 {
		t.Fatalf("Expected 1 loaded entry, got %d", len(loadedEntries))
	}

	entry := loadedEntries[0]
	if entry.ID != "1" || entry.Context != "Project Alpha > Sprint 1" {
		t.Errorf("Unexpected entry data: %+v", entry)
	}
	if entry.Item.Text != "Archived task 1" || !entry.Item.IsTask || entry.Item.Status != StatusDone {
		t.Errorf("Unexpected item data: %+v", entry.Item)
	}
	if len(entry.Item.Children) != 1 || entry.Item.Children[0].Text != "Child bullet" {
		t.Errorf("Unexpected children: %+v", entry.Item.Children)
	}
	if entry.Item.Children[0].Parent != entry.Item {
		t.Errorf("Parent pointer not properly restored for child item")
	}
}

func TestArchiveStoreEncrypted(t *testing.T) {
	tempDir, err := os.MkdirTemp("", "halptask_archive_enc_test_*")
	if err != nil {
		t.Fatalf("Failed to create temp dir: %v", err)
	}
	defer os.RemoveAll(tempDir)

	archivePath := filepath.Join(tempDir, "archive.dat")
	store := NewArchiveStore(archivePath, true)

	item := NewTask("10", "Secret archived task", StatusDone)
	testEntry := &ArchivedEntry{
		ID:         item.ID,
		ArchivedAt: time.Now(),
		Context:    "Confidential",
		Item:       item,
	}

	passphrase := "supersecret123"

	// Save with passphrase
	if err := store.Save([]*ArchivedEntry{testEntry}, passphrase); err != nil {
		t.Fatalf("Failed to save encrypted archive: %v", err)
	}

	// Loading with wrong passphrase should fail
	_, err = store.Load("wrongpass")
	if err == nil {
		t.Fatalf("Expected error when loading with incorrect passphrase")
	}

	// Loading with correct passphrase should succeed
	loaded, err := store.Load(passphrase)
	if err != nil {
		t.Fatalf("Failed to load encrypted archive: %v", err)
	}
	if len(loaded) != 1 || loaded[0].Item.Text != "Secret archived task" {
		t.Errorf("Loaded content mismatch: %+v", loaded)
	}

	// Reencrypt with new passphrase
	newPassphrase := "newpass456"
	if err := store.Reencrypt(passphrase, newPassphrase); err != nil {
		t.Fatalf("Failed to re-encrypt archive: %v", err)
	}

	// Loading with old passphrase should fail now
	_, err = store.Load(passphrase)
	if err == nil {
		t.Fatalf("Expected error loading re-encrypted archive with old passphrase")
	}

	// Loading with new passphrase should succeed
	loadedNew, err := store.Load(newPassphrase)
	if err != nil {
		t.Fatalf("Failed to load re-encrypted archive: %v", err)
	}
	if len(loadedNew) != 1 || loadedNew[0].Item.Text != "Secret archived task" {
		t.Errorf("Loaded re-encrypted content mismatch: %+v", loadedNew)
	}
}
