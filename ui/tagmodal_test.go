package ui_test

import (
	"testing"

	"github.com/halpworld/halptask/config"
	"github.com/halpworld/halptask/model"
	"github.com/halpworld/halptask/ui"
)

func TestTagModalSelectionAndCustomTag(t *testing.T) {
	tagConfigs := config.GetDefaultTagConfigs()
	modal := ui.NewTagModal(tagConfigs)
	item := model.NewTask("1", "Implement feature", model.StatusTodo)
	tree := model.NewTree()
	tree.Roots = append(tree.Roots, item)
	tree.SetParents()

	modal.SetItem(item, tree)

	rendered := modal.Render(80, 20)
	if rendered == "" {
		t.Fatalf("Expected rendered tag modal string")
	}

	// Toggle first tag ("bug")
	modal.Update(struct{ Key string }{"enter"}) // Toggle
	// Test rendering
	rendered = modal.Render(80, 20)
	if rendered == "" {
		t.Fatalf("Expected rendered tag modal string after toggle")
	}
}
