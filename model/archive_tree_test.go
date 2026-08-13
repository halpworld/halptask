package model

import (
	"testing"
)

func TestTreeArchiveItem(t *testing.T) {
	tree := NewTree()
	parent := NewItem("1", "Parent Project")
	child1 := NewTask("2", "Subtask 1", StatusTodo)
	child2 := NewTask("3", "Subtask 2 (to archive)", StatusInProgress)
	grandchild := NewItem("4", "Grandchild bullet")

	child2.Children = append(child2.Children, grandchild)
	parent.Children = append(parent.Children, child1, child2)
	tree.Roots = append(tree.Roots, parent)
	tree.SetParents()

	archivedItem, contextPath := tree.ArchiveItem("3")
	if archivedItem == nil {
		t.Fatalf("Expected item to be archived, got nil")
	}
	if archivedItem.ID != "3" || archivedItem.Text != "Subtask 2 (to archive)" {
		t.Errorf("Unexpected archived item: %+v", archivedItem)
	}
	if contextPath != "Parent Project" {
		t.Errorf("Expected context path 'Parent Project', got '%s'", contextPath)
	}
	if len(archivedItem.Children) != 1 || archivedItem.Children[0].ID != "4" {
		t.Errorf("Expected child item to be retained in archived subtree")
	}

	// Verify item 3 is no longer in active tree
	if tree.FindItem("3") != nil {
		t.Errorf("Item 3 still found in active tree after archiving")
	}
	if len(parent.Children) != 1 || parent.Children[0].ID != "2" {
		t.Errorf("Parent children list not updated properly")
	}
}

func TestTreeArchiveCompleted(t *testing.T) {
	tree := NewTree()
	r1 := NewTask("1", "Done task 1", StatusDone)
	r2 := NewTask("2", "Todo task 2", StatusTodo)
	p := NewItem("3", "Section Header")
	c1 := NewTask("4", "Done subtask", StatusDone)
	c2 := NewTask("5", "In progress subtask", StatusInProgress)

	p.Children = append(p.Children, c1, c2)
	tree.Roots = append(tree.Roots, r1, r2, p)
	tree.SetParents()

	entries := tree.ArchiveCompleted()
	if len(entries) != 2 {
		t.Fatalf("Expected 2 archived entries, got %d", len(entries))
	}

	foundR1 := false
	foundC1 := false
	for _, e := range entries {
		if e.ID == "1" {
			foundR1 = true
			if e.Context != "" {
				t.Errorf("Root item should have empty context, got '%s'", e.Context)
			}
		}
		if e.ID == "4" {
			foundC1 = true
			if e.Context != "Section Header" {
				t.Errorf("Expected context 'Section Header', got '%s'", e.Context)
			}
		}
	}

	if !foundR1 || !foundC1 {
		t.Errorf("Did not find all expected completed tasks in entries")
	}

	// Verify active tree state
	if tree.FindItem("1") != nil || tree.FindItem("4") != nil {
		t.Errorf("Done tasks still present in active tree after ArchiveCompleted")
	}
	if tree.FindItem("2") == nil || tree.FindItem("5") == nil {
		t.Errorf("Active unfinished tasks missing from tree")
	}
}
