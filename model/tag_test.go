package model_test

import (
	"testing"

	"github.com/halpworld/halptask/model"
)

func TestItemTags(t *testing.T) {
	item := model.NewTask("1", "Fix bug", model.StatusTodo)
	if item.HasDirectTag("bug") {
		t.Errorf("Expected no direct tag initially")
	}

	item.AddTag("bug")
	item.AddTag("URGENT")
	if !item.HasDirectTag("bug") || !item.HasDirectTag("urgent") {
		t.Errorf("Expected direct tags 'bug' and 'urgent'")
	}

	item.ToggleTag("bug")
	if item.HasDirectTag("bug") {
		t.Errorf("Expected 'bug' tag to be toggled off")
	}

	clone := item.Clone()
	if !clone.HasDirectTag("urgent") {
		t.Errorf("Expected clone to preserve tags")
	}
}

func TestTreeTagInheritanceAndMovement(t *testing.T) {
	tree := model.NewTree()
	parent := model.NewTask("p1", "Parent Project", model.StatusTodo)
	child := model.NewTask("c1", "Child Subtask", model.StatusTodo)
	parent.Children = append(parent.Children, child)
	tree.Roots = append(tree.Roots, parent)
	tree.SetParents()

	// Initial state: no tags
	direct, inherited := tree.GetEffectiveTags(child)
	if len(direct) != 0 || len(inherited) != 0 {
		t.Errorf("Expected no direct or inherited tags, got direct: %v, inherited: %v", direct, inherited)
	}

	// Step 1: Parent gets labeled #work
	parent.AddTag("work")
	direct, inherited = tree.GetEffectiveTags(child)
	if len(direct) != 0 {
		t.Errorf("Child should have no direct tags, got: %v", direct)
	}
	if len(inherited) != 1 || inherited[0] != "work" {
		t.Errorf("Child should inherit 'work' tag from parent, got: %v", inherited)
	}

	// Step 2: Child gets its own direct tag #bug
	child.AddTag("bug")
	direct, inherited = tree.GetEffectiveTags(child)
	if len(direct) != 1 || direct[0] != "bug" {
		t.Errorf("Child should have direct tag 'bug', got: %v", direct)
	}
	if len(inherited) != 1 || inherited[0] != "work" {
		t.Errorf("Child should still inherit 'work' from parent, got: %v", inherited)
	}

	// Step 3: Move child out of parent (Unindent to root)
	tree.Unindent("c1")
	direct, inherited = tree.GetEffectiveTags(child)
	if len(inherited) != 0 {
		t.Errorf("Child moved to root should lose inherited 'work' tag, got inherited: %v", inherited)
	}
	if len(direct) != 1 || direct[0] != "bug" {
		t.Errorf("Child moved to root should retain direct tag 'bug', got direct: %v", direct)
	}

	// Step 4: Indent child back under parent
	tree.Indent("c1")
	direct, inherited = tree.GetEffectiveTags(child)
	if len(inherited) != 1 || inherited[0] != "work" {
		t.Errorf("Child indented under parent should re-inherit 'work', got: %v", inherited)
	}

	// Step 5: Search by tag
	matches := tree.SearchByTag("work")
	if len(matches) != 2 { // Both parent and child have #work (directly & inherited)
		t.Errorf("SearchByTag('work') should match 2 items, got: %d", len(matches))
	}

	matchesBug := tree.SearchByTag("bug")
	if len(matchesBug) != 1 || matchesBug[0] != "c1" {
		t.Errorf("SearchByTag('bug') should match child c1, got: %v", matchesBug)
	}
}
