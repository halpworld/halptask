package ui

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/halpworld/halptask/config"
	"github.com/halpworld/halptask/model"
	"strings"
	"testing"
)

func TestParentStatusConfirmation(t *testing.T) {
	parent := model.NewTask("1", "parent", model.StatusDone)
	child := model.NewTask("2", "child", model.StatusTodo)
	parent.Children = []*model.Item{child}
	tree := &model.Tree{Roots: []*model.Item{parent}}
	tree.SetParents()
	app := AppModel{Tree: tree, Config: config.DefaultConfig(), SelectedID: "1", Mode: ModeNormal}
	app.requestStatus("1", model.StatusDone)
	if !strings.Contains(app.View(), "ALL subtasks") || child.Status != model.StatusTodo || len(app.UndoStack) != 0 {
		t.Fatal("prompt must precede mutation and undo")
	}
	updated, _ := app.Update(tea.KeyMsg{Type: tea.KeyEnter})
	app = updated.(AppModel)
	if app.PendingStatusID == "" || child.Status != model.StatusTodo {
		t.Fatal("Enter must not confirm")
	}
	updated, _ = app.Update(tea.KeyMsg{Type: tea.KeyEsc})
	app = updated.(AppModel)
	if app.PendingStatusID != "" || len(app.UndoStack) != 0 {
		t.Fatal("cancellation changed history")
	}
	app.requestStatus("1", model.StatusDone)
	updated, _ = app.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	app = updated.(AppModel)
	if child.Status != model.StatusDone || len(app.UndoStack) != 1 {
		t.Fatal("confirmation must apply one undoable group change")
	}
	app.undo()
	if app.Tree.FindItem("1").EffectiveStatus() != model.StatusTodo {
		t.Fatal("undo did not restore child-derived state")
	}
	app.redo()
	if app.Tree.FindItem("1").EffectiveStatus() != model.StatusDone {
		t.Fatal("redo did not restore group change")
	}
}
