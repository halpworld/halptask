package ui_test

import (
	"path/filepath"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/kenth/halptask/config"
	"github.com/kenth/halptask/model"
	"github.com/kenth/halptask/ui"
)

func TestDashboardViewRenderingAndInProgressSubtasks(t *testing.T) {
	tree := model.NewTree()

	// Root task
	parentTask := model.NewTask("p1", "Project Refactoring", model.StatusTodo)

	// In progress subtask 1
	subTask1 := model.NewTask("s1", "Implement vertical split pane", model.StatusInProgress)
	parentTask.Children = append(parentTask.Children, subTask1)

	// Top level in progress task
	topTask := model.NewTask("t1", "Fix database connection leak", model.StatusInProgress)

	tree.Roots = append(tree.Roots, parentTask, topTask)
	tree.SetParents()

	// Verify GetInProgressTasks
	inProgress := tree.GetInProgressTasks()
	if len(inProgress) != 2 {
		t.Fatalf("expected 2 in progress tasks, got %d", len(inProgress))
	}

	foundSubtask := false
	for _, itemCtx := range inProgress {
		if itemCtx.Item.ID == "s1" {
			foundSubtask = true
			if itemCtx.ParentPath != "Project Refactoring" {
				t.Fatalf("expected parent path 'Project Refactoring', got %q", itemCtx.ParentPath)
			}
		}
	}
	if !foundSubtask {
		t.Fatalf("expected to find subTask1 in inProgress list")
	}

	// Render Dashboard
	dbView := ui.NewDashboardView()
	dbView.Width = 35
	dbView.Height = 20

	rendered := dbView.Render(tree, config.GetDefaultTagConfigs())

	if !strings.Contains(rendered, "DASHBOARD") {
		t.Fatalf("expected DASHBOARD in rendered output")
	}
	if !strings.Contains(rendered, "IN PROGRESS") {
		t.Fatalf("expected IN PROGRESS section in rendered output")
	}
	if !strings.Contains(rendered, "Implement vertical split pane") {
		t.Fatalf("expected subtask text in rendered output")
	}
	if !strings.Contains(rendered, "Project Refactoring") {
		t.Fatalf("expected parent hint in rendered output for subtask")
	}
}

func TestDashboardToggleLeaderCommand(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.ShowDashboard = true

	dummyPath := filepath.Join(t.TempDir(), "dummy_data.pb")
	cfg.DataFile = dummyPath
	storage := model.NewStorage(dummyPath, false)
	appModel, _ := ui.InitialModel(cfg, storage)

	if !appModel.Config.ShowDashboard {
		t.Fatalf("expected ShowDashboard to be true initially")
	}

	// Press leader key '<space>'
	m1, _ := appModel.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{' '}})
	am1 := m1.(ui.AppModel)
	if !am1.WhichKey.Active {
		t.Fatalf("expected WhichKey to be active after space")
	}

	// Press 'd' to toggle dashboard
	m2, _ := am1.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'d'}})
	am2 := m2.(ui.AppModel)

	if am2.Config.ShowDashboard {
		t.Fatalf("expected ShowDashboard to be toggled to false after <space> d")
	}
	if am2.WhichKey.Active {
		t.Fatalf("expected WhichKey to be deactivated after toggle")
	}
}

func TestCtrlCHardExit(t *testing.T) {
	cfg := config.DefaultConfig()
	dummyPath := filepath.Join(t.TempDir(), "dummy_data.pb")
	cfg.DataFile = dummyPath
	storage := model.NewStorage(dummyPath, false)
	appModel, _ := ui.InitialModel(cfg, storage)

	// Send ctrl+c
	_, cmd := appModel.Update(tea.KeyMsg{Type: tea.KeyCtrlC})
	if cmd == nil {
		t.Fatalf("expected tea.Quit command on ctrl+c")
	}
}
