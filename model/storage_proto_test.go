package model_test

import (
	"bytes"
	"compress/flate"
	"os"
	"path/filepath"
	"testing"

	"github.com/kenth/halptask/model"
)

func TestSerializeAndParseProtobuf(t *testing.T) {
	tree := model.NewTree()
	r1 := tree.InsertBelow("", "Root bullet #urgent #project")
	r1.NodeID = "node-alpha"
	r1.Version = 42

	c1 := tree.AddChild(r1.ID, "Child task")
	c1.IsTask = true
	c1.Status = model.StatusInProgress
	c1.Folded = true
	c1.NodeID = "node-beta"
	c1.AddTag("feature")

	c2 := tree.AddChild(r1.ID, "Completed subtask")
	c2.IsTask = true
	c2.Status = model.StatusDone

	// 1. Serialize to Protobuf
	data, err := model.SerializeProtobuf(tree)
	if err != nil {
		t.Fatalf("SerializeProtobuf failed: %v", err)
	}

	if len(data) == 0 {
		t.Fatalf("expected non-empty binary payload")
	}

	// 2. Parse back from Protobuf
	parsedTree, err := model.ParseProtobuf(data)
	if err != nil {
		t.Fatalf("ParseProtobuf failed: %v", err)
	}

	if len(parsedTree.Roots) != 1 {
		t.Fatalf("expected 1 root item, got %d", len(parsedTree.Roots))
	}

	pRoot := parsedTree.Roots[0]
	if pRoot.Text != "Root bullet #urgent #project" {
		t.Fatalf("expected text 'Root bullet #urgent #project', got %q", pRoot.Text)
	}
	if pRoot.NodeID != "node-alpha" {
		t.Fatalf("expected NodeID 'node-alpha', got %q", pRoot.NodeID)
	}
	if pRoot.Version != 42 {
		t.Fatalf("expected Version 42, got %d", pRoot.Version)
	}
	if len(pRoot.Children) != 2 {
		t.Fatalf("expected 2 children, got %d", len(pRoot.Children))
	}

	pChild1 := pRoot.Children[0]
	if pChild1.Status != model.StatusInProgress {
		t.Fatalf("expected StatusInProgress, got %s", pChild1.Status)
	}
	if !pChild1.Folded {
		t.Fatalf("expected child1 to be folded")
	}
	if pChild1.NodeID != "node-beta" {
		t.Fatalf("expected NodeID 'node-beta', got %q", pChild1.NodeID)
	}
}

func TestLegacyMarkdownAutoUpgrade(t *testing.T) {
	tempDir := t.TempDir()
	filePath := filepath.Join(tempDir, "legacy.md")

	markdownContent := `- Welcome to HalpTask! <!-- id: 1 -->
  - [ ] Legacy task #v1 <!-- id: 2 -->
  - [x] Legacy completed task <!-- id: 3 -->
`

	if err := os.WriteFile(filePath, []byte(markdownContent), 0644); err != nil {
		t.Fatalf("failed to write legacy markdown file: %v", err)
	}

	storage := model.NewStorage(filePath, false)

	// 1. Load legacy Markdown
	tree, err := storage.Load("")
	if err != nil {
		t.Fatalf("failed to load legacy markdown file: %v", err)
	}

	if len(tree.Roots) != 1 {
		t.Fatalf("expected 1 root, got %d", len(tree.Roots))
	}
	if len(tree.Roots[0].Children) != 2 {
		t.Fatalf("expected 2 children, got %d", len(tree.Roots[0].Children))
	}

	// 2. Save file -> automatically saves as Protobuf v2
	if err := storage.Save(tree, ""); err != nil {
		t.Fatalf("failed to save upgraded tree: %v", err)
	}

	// 3. Re-load file and verify Protobuf parsing
	reloadedTree, err := storage.Load("")
	if err != nil {
		t.Fatalf("failed to reload upgraded file: %v", err)
	}

	if len(reloadedTree.Roots) != 1 || len(reloadedTree.Roots[0].Children) != 2 {
		t.Fatalf("reloaded tree structure mismatch")
	}
}

func TestGetMigratedFilePath(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"data.txt", "data.pb"},
		{"tasks.md", "tasks.pb"},
		{"/path/to/notes.md", "/path/to/notes.pb"},
		{"/path/to/already.pb", "/path/to/already.pb"},
		{"/path/to/custom.halp", "/path/to/custom.halp"},
		{"noext", "noext.pb"},
	}

	for _, tt := range tests {
		got := model.GetMigratedFilePath(tt.input)
		if got != tt.expected {
			t.Errorf("GetMigratedFilePath(%q) = %q, expected %q", tt.input, got, tt.expected)
		}
	}
}

func TestMigrationSystem_FileRenamingAndBackup(t *testing.T) {
	tempDir := t.TempDir()
	originalPath := filepath.Join(tempDir, "my_legacy_tasks.md")
	expectedMigratedPath := filepath.Join(tempDir, "my_legacy_tasks.pb")
	expectedBackupPath := originalPath + ".bak"

	markdownData := `- Legacy Root Item <!-- id: 100 -->
  - [ ] Migration task #urgent <!-- id: 101 -->
`

	if err := os.WriteFile(originalPath, []byte(markdownData), 0644); err != nil {
		t.Fatalf("failed to write original legacy file: %v", err)
	}

	storage := model.NewStorage(originalPath, false)

	// 1. Run MigrateIfNeeded
	migrated, newPath, err := storage.MigrateIfNeeded("")
	if err != nil {
		t.Fatalf("MigrateIfNeeded failed: %v", err)
	}

	if !migrated {
		t.Fatalf("expected migrated == true for legacy .md file")
	}

	if newPath != expectedMigratedPath {
		t.Fatalf("expected newPath %q, got %q", expectedMigratedPath, newPath)
	}

	if storage.FilePath != expectedMigratedPath {
		t.Fatalf("expected storage.FilePath to update to %q, got %q", expectedMigratedPath, storage.FilePath)
	}

	// 2. Verify new .pb file exists and contains valid Protobuf binary data
	if _, err := os.Stat(expectedMigratedPath); os.IsNotExist(err) {
		t.Fatalf("migrated .pb file does not exist at %s", expectedMigratedPath)
	}

	// 3. Verify backup file exists
	if _, err := os.Stat(expectedBackupPath); os.IsNotExist(err) {
		t.Fatalf("backup file does not exist at %s", expectedBackupPath)
	}

	// 4. Verify re-running MigrateIfNeeded on already migrated file returns migrated == false
	migratedAgain, _, err := storage.MigrateIfNeeded("")
	if err != nil {
		t.Fatalf("second MigrateIfNeeded failed: %v", err)
	}
	if migratedAgain {
		t.Fatalf("expected migratedAgain == false for already migrated file")
	}
}

func TestEncryptedProtobuf(t *testing.T) {
	tempDir := t.TempDir()
	filePath := filepath.Join(tempDir, "encrypted.pb")
	passphrase := "secret123"

	storage := model.NewStorage(filePath, true)
	tree := model.NewTree()
	root := tree.InsertBelow("", "Encrypted root note")
	tree.AddChild(root.ID, "Top secret task")

	// Save encrypted
	if err := storage.Save(tree, passphrase); err != nil {
		t.Fatalf("failed to save encrypted file: %v", err)
	}

	// Load with wrong passphrase
	_, err := storage.Load("wrongpass")
	if err == nil {
		t.Fatalf("expected error loading with incorrect passphrase")
	}

	// Load with correct passphrase
	loadedTree, err := storage.Load(passphrase)
	if err != nil {
		t.Fatalf("failed to load encrypted file: %v", err)
	}

	if len(loadedTree.Roots) != 1 || loadedTree.Roots[0].Text != "Encrypted root note" {
		t.Fatalf("encrypted tree content mismatch")
	}
}

func compressData(data []byte) []byte {
	var buf bytes.Buffer
	w, _ := flate.NewWriter(&buf, flate.BestCompression)
	w.Write(data)
	w.Close()
	return buf.Bytes()
}

func BenchmarkStorageSize(b *testing.B) {
	tree := model.NewTree()
	for i := 0; i < 50; i++ {
		root := tree.InsertBelow("", "Project milestone task item with tags #work #urgent")
		for j := 0; j < 5; j++ {
			child := tree.AddChild(root.ID, "Subtask description with details and metadata #sub")
			child.IsTask = true
			child.Status = model.StatusInProgress
		}
	}

	markdownData := []byte(model.SerializeMarkdown(tree))
	protoData, err := model.SerializeProtobuf(tree)
	if err != nil {
		b.Fatalf("SerializeProtobuf failed: %v", err)
	}

	compProto := compressData(protoData)

	b.Logf("Markdown: %d bytes | Protobuf Raw: %d bytes (%.1f%%) | Protobuf Compressed: %d bytes (%.1f%% reduction)",
		len(markdownData), len(protoData),
		(1.0-float64(len(protoData))/float64(len(markdownData)))*100.0,
		len(compProto),
		(1.0-float64(len(compProto))/float64(len(markdownData)))*100.0)
}

func TestYamlConfigFileIgnoredInMigration(t *testing.T) {
	tempDir := t.TempDir()
	yamlPath := filepath.Join(tempDir, "config.yaml")
	yamlContent := "auto_save: true\ndata_file: /tmp/data.pb\n"

	if err := os.WriteFile(yamlPath, []byte(yamlContent), 0644); err != nil {
		t.Fatalf("failed to write yaml file: %v", err)
	}

	storage := model.NewStorage(yamlPath, false)
	migrated, newPath, err := storage.MigrateIfNeeded("")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if migrated {
		t.Fatalf("expected migrated == false for .yaml file")
	}

	if newPath != yamlPath {
		t.Fatalf("expected newPath to remain %q, got %q", yamlPath, newPath)
	}
}
