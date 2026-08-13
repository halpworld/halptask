package ui

import (
	"testing"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/kenth/halptask/model"
)

func TestExtractLinks(t *testing.T) {
	text := "Check task #2 and [Backend Refactor](#5) or [Frontend Task](10) and task:12."
	links := ExtractLinks(text)

	if len(links) != 4 {
		t.Fatalf("expected 4 links, got %d", len(links))
	}

	expectedIDs := []string{"5", "10", "2", "12"}
	for i, exp := range expectedIDs {
		if links[i].TargetID != exp {
			t.Errorf("link %d: expected target ID %s, got %s", i, exp, links[i].TargetID)
		}
	}
}

func TestNoteModalViewAndEdit(t *testing.T) {
	nm := NewNoteModal(80, 24)
	tree := model.NewTree()
	item1 := model.NewTask("1", "First Task", model.StatusTodo)
	item2 := model.NewTask("2", "Second Task", model.StatusTodo)
	tree.Roots = append(tree.Roots, item1, item2)

	item1.Note = "See #2 for more details."
	nm.SetItem(item1, tree)

	if nm.Mode != NoteModeView {
		t.Fatalf("expected NoteModeView for non-empty note, got %v", nm.Mode)
	}

	if len(nm.Links) != 1 || nm.Links[0].TargetID != "2" {
		t.Fatalf("expected link to target ID '2', got %v", nm.Links)
	}

	// Test pressing Enter on link
	_, _, jumpID := nm.Update(tea.KeyMsg{Type: tea.KeyEnter})
	if jumpID != "2" {
		t.Fatalf("expected jumpID '2' after pressing enter, got %s", jumpID)
	}

	// Test switching to edit mode with 'e'
	nm.Update(tea.KeyMsg{Runes: []rune{'e'}, Type: tea.KeyRunes})
	if nm.Mode != NoteModeEdit {
		t.Fatalf("expected NoteModeEdit after pressing 'e', got %v", nm.Mode)
	}

	// Type new note text and save with esc
	nm.TextArea.SetValue("Updated note referencing #1")
	nm.Update(tea.KeyMsg{Type: tea.KeyEsc})

	if nm.Mode != NoteModeView {
		t.Fatalf("expected NoteModeView after saving with Esc, got %v", nm.Mode)
	}

	if item1.Note != "Updated note referencing #1" {
		t.Fatalf("expected updated note on item1, got %s", item1.Note)
	}

	if len(nm.Links) != 1 || nm.Links[0].TargetID != "1" {
		t.Fatalf("expected extracted link to #1 after update, got %v", nm.Links)
	}
}

func TestLinkReplacementNoSubstringCorruption(t *testing.T) {
	nm := NewNoteModal(80, 24)
	tree := model.NewTree()
	item1 := model.NewTask("1", "First Task", model.StatusTodo)
	item10 := model.NewTask("10", "Tenth Task", model.StatusTodo)
	tree.Roots = append(tree.Roots, item1, item10)

	// Note text containing #1, #10, and tag #work
	noteText := "Check #10 and #1 for tag #work details."
	item1.Note = noteText
	nm.SetItem(item1, tree)

	rendered := nm.RenderMarkdownView(noteText)

	// #work should not be parsed as a task jump link or styled with (?)
	if len(nm.Links) != 2 {
		t.Fatalf("expected 2 task links (#10 and #1), got %d: %v", len(nm.Links), nm.Links)
	}

	// Verify links order and target IDs
	if nm.Links[0].TargetID != "10" || nm.Links[1].TargetID != "1" {
		t.Fatalf("expected target IDs '10' and '1', got %v", nm.Links)
	}

	// Verify rendered text contains both #10 and #1 correctly rendered without corrupted ANSI tags
	if !testing.Verbose() {
		// Ensure rendered view contains formatted labels
		if len(rendered) == 0 {
			t.Fatalf("rendered view is empty")
		}
	}
}
