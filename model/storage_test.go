package model

import (
	"path/filepath"
	"testing"
)

func TestTreeOperations(t *testing.T) {
	tree := NewTree()
	r1 := tree.InsertBelow("", "Root 1")
	r2 := tree.InsertBelow(r1.ID, "Root 2")
	c1 := tree.AddChild(r1.ID, "Child 1.1")

	if len(tree.Roots) != 2 {
		t.Fatalf("expected 2 roots, got %d", len(tree.Roots))
	}
	if len(r1.Children) != 1 {
		t.Fatalf("expected 1 child for r1, got %d", len(r1.Children))
	}
	if c1.Text != "Child 1.1" {
		t.Fatalf("unexpected child text %s", c1.Text)
	}

	// Indent r2 to become child of r1
	ok := tree.Indent(r2.ID)
	if !ok {
		t.Fatalf("failed to indent r2")
	}
	if len(tree.Roots) != 1 {
		t.Fatalf("expected 1 root after indenting r2, got %d", len(tree.Roots))
	}
	if len(r1.Children) != 2 {
		t.Fatalf("expected 2 children for r1, got %d", len(r1.Children))
	}

	// Cycle task status
	tree.ToggleTask(c1.ID)
	if !c1.IsTask || c1.Status != StatusTodo {
		t.Fatalf("expected task status todo, got isTask=%v status=%s", c1.IsTask, c1.Status)
	}
	tree.CycleStatus(c1.ID)
	if c1.Status != StatusInProgress {
		t.Fatalf("expected status in_progress, got %s", c1.Status)
	}
	tree.CycleStatus(c1.ID)
	if c1.Status != StatusDone {
		t.Fatalf("expected status done, got %s", c1.Status)
	}
}

func TestMarkdownSerializeAndParse(t *testing.T) {
	tree := NewTree()
	r1 := tree.InsertBelow("", "Bullet 1")
	t1 := tree.InsertBelow(r1.ID, "Task Todo")
	tree.ToggleTask(t1.ID)

	t2 := tree.AddChild(r1.ID, "Subtask Done")
	tree.SetStatus(t2.ID, StatusDone)

	t3 := tree.AddChild(r1.ID, "Subtask In Progress")
	tree.SetStatus(t3.ID, StatusInProgress)

	r1.Folded = true

	md := SerializeMarkdown(tree)
	parsedTree := ParseMarkdown(md)

	if len(parsedTree.Roots) != 2 {
		t.Fatalf("expected 2 roots in parsed tree, got %d", len(parsedTree.Roots))
	}

	parsedR1 := parsedTree.Roots[0]
	if !parsedR1.Folded {
		t.Fatalf("expected parsed root to be folded")
	}
	if len(parsedR1.Children) != 2 {
		t.Fatalf("expected 2 children for parsed root, got %d", len(parsedR1.Children))
	}
	if parsedR1.Children[0].Status != StatusDone {
		t.Fatalf("expected status done for first child, got %s", parsedR1.Children[0].Status)
	}
	if parsedR1.Children[1].Status != StatusInProgress {
		t.Fatalf("expected status in_progress for second child, got %s", parsedR1.Children[1].Status)
	}
}

func TestEncryptionDecryption(t *testing.T) {
	plainText := "- [ ] First task\n  - [x] Subtask done\n"
	passphrase := "secret123"

	encrypted, err := encryptContent(plainText, passphrase)
	if err != nil {
		t.Fatalf("encryption failed: %v", err)
	}

	decrypted, err := decryptContent(encrypted, passphrase)
	if err != nil {
		t.Fatalf("decryption failed: %v", err)
	}

	if decrypted != plainText {
		t.Fatalf("decrypted content mismatch: expected %q, got %q", plainText, decrypted)
	}

	_, err = decryptContent(encrypted, "wrongpass")
	if err == nil {
		t.Fatalf("expected error when decrypting with wrong password")
	}
}

func TestPermanentIDsPersistence(t *testing.T) {
	inputMD := `- Root Item <!-- id: 10 -->
  - Child Task [ ] #work <!-- id: 11 -->
`
	tree := ParseMarkdown(inputMD)
	if len(tree.Roots) != 1 {
		t.Fatalf("expected 1 root, got %d", len(tree.Roots))
	}
	r := tree.Roots[0]
	if r.ID != "10" {
		t.Fatalf("expected root ID '10', got %q", r.ID)
	}
	if len(r.Children) != 1 {
		t.Fatalf("expected 1 child, got %d", len(r.Children))
	}
	c := r.Children[0]
	if c.ID != "11" {
		t.Fatalf("expected child ID '11', got %q", c.ID)
	}

	serialized := SerializeMarkdown(tree)
	if !testing.Verbose() {
		// verify serialized contains id tags
		if !containsSub(serialized, "<!-- id: 10 -->") || !containsSub(serialized, "<!-- id: 11 -->") {
			t.Fatalf("serialized markdown missing id tags: %s", serialized)
		}
	}
}

func TestEnsureIDsForUnassignedItems(t *testing.T) {
	inputMD := `- First unassigned item
- Second unassigned item <!-- id: 5 -->
- Third unassigned item
`
	tree := ParseMarkdown(inputMD)
	if len(tree.Roots) != 3 {
		t.Fatalf("expected 3 roots, got %d", len(tree.Roots))
	}
	if tree.Roots[1].ID != "5" {
		t.Fatalf("expected second root ID '5', got %q", tree.Roots[1].ID)
	}
	// Unassigned items should get IDs 6 and 7 (higher than max existing 5)
	if tree.Roots[0].ID != "6" {
		t.Fatalf("expected first root ID '6', got %q", tree.Roots[0].ID)
	}
	if tree.Roots[2].ID != "7" {
		t.Fatalf("expected third root ID '7', got %q", tree.Roots[2].ID)
	}
}

func TestFocusModePersistence(t *testing.T) {
	tree := NewTree()
	r1 := tree.InsertBelow("", "Root 1")
	c1 := tree.AddChild(r1.ID, "Child 1.1")

	// Set focus on c1
	focused := tree.ToggleFocus(c1.ID)
	if focused == nil || !focused.IsFocused {
		t.Fatalf("expected child to be focused")
	}

	if tree.GetFocusedItem() != c1 {
		t.Fatalf("expected GetFocusedItem() to return c1")
	}

	// Test Protobuf serialization and deserialization
	pbData, err := SerializeProtobuf(tree)
	if err != nil {
		t.Fatalf("failed to serialize protobuf: %v", err)
	}

	parsedPbTree, err := ParseProtobuf(pbData)
	if err != nil {
		t.Fatalf("failed to parse protobuf: %v", err)
	}

	focusedFromPb := parsedPbTree.GetFocusedItem()
	if focusedFromPb == nil || focusedFromPb.ID != c1.ID {
		t.Fatalf("expected focused item ID %s from Protobuf, got %v", c1.ID, focusedFromPb)
	}

	// Test Markdown serialization and deserialization
	md := SerializeMarkdown(tree)
	parsedMdTree := ParseMarkdown(md)

	focusedFromMd := parsedMdTree.GetFocusedItem()
	if focusedFromMd == nil || !focusedFromMd.IsFocused {
		t.Fatalf("expected focused item from Markdown parsing")
	}

	// Test toggling focus off
	tree.ToggleFocus(c1.ID)
	if tree.GetFocusedItem() != nil {
		t.Fatalf("expected GetFocusedItem() to be nil after clearing focus")
	}
}

func containsSub(s, substr string) bool {
	return len(s) >= len(substr) && (s == substr || len(s) > len(substr) && searchString(s, substr))
}

func searchString(s, substr string) bool {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}

func TestEmptyProtobufTreeLoad(t *testing.T) {
	tempDir := t.TempDir()
	filePath := filepath.Join(tempDir, "data.txt") // non .pb extension
	storage := NewStorage(filePath, false)

	emptyTree := NewTree()
	err := storage.Save(emptyTree, "")
	if err != nil {
		t.Fatalf("failed to save empty tree: %v", err)
	}

	loadedTree, err := storage.Load("")
	if err != nil {
		t.Fatalf("failed to load empty tree: %v", err)
	}
	if len(loadedTree.Roots) != 0 {
		t.Fatalf("expected 0 roots for empty protobuf tree, got %d", len(loadedTree.Roots))
	}
}

func TestParseMarkdownUTF8Bullet(t *testing.T) {
	content := "• UTF-8 Bullet Test"
	tree := ParseMarkdown(content)
	if len(tree.Roots) != 1 {
		t.Fatalf("expected 1 root, got %d", len(tree.Roots))
	}
	if tree.Roots[0].Text != "UTF-8 Bullet Test" {
		t.Fatalf("expected text 'UTF-8 Bullet Test', got %q", tree.Roots[0].Text)
	}
}

func TestEnsureIDsDeduplication(t *testing.T) {
	tree := NewTree()
	i1 := NewItem("1", "First Item")
	i2 := NewItem("1", "Duplicate ID Item") // Duplicate ID
	tree.Roots = append(tree.Roots, i1, i2)

	tree.EnsureIDs()

	if tree.Roots[0].ID == tree.Roots[1].ID {
		t.Fatalf("expected distinct IDs, got duplicate %q", tree.Roots[0].ID)
	}
}
