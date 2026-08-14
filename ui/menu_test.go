package ui_test

import (
	"strings"
	"testing"

	"github.com/charmbracelet/lipgloss"
	"github.com/halpworld/halptask/config"
	"github.com/halpworld/halptask/model"
	"github.com/halpworld/halptask/ui"
)

// TestMenuLayoutConsistency verifies that all UI menus (WhichKey, TagModal in all states, HelpModal, QuickHelp, StatusBar)
// render within specified terminal width boundaries without visual line overflow or awkward linebreaks.
func TestMenuLayoutConsistency(t *testing.T) {
	testWidths := []int{60, 80, 100, 120}
	tagConfigs := config.GetDefaultTagConfigs()
	allBindings := ui.GetAllKeyBindings()

	sampleItem := model.NewTask("task-1", "Refactor core UI engine & verify tags", model.StatusTodo)
	sampleItem.AddTag("bug")
	sampleItem.AddTag("urgent")

	sampleTree := model.NewTree()
	sampleTree.Roots = append(sampleTree.Roots, sampleItem)
	sampleTree.SetParents()

	// 1. Test WhichKey Menu
	t.Run("WhichKey", func(t *testing.T) {
		wk := ui.NewWhichKeyModel()
		wk.Active = true
		wk.PrefixKeys = []string{" "}

		for _, w := range testWidths {
			rendered := wk.Render(allBindings, w)
			lines := strings.Split(rendered, "\n")
			for idx, line := range lines {
				visWidth := lipgloss.Width(line)
				if visWidth > w {
					t.Errorf("WhichKey line %d exceeds width %d (got %d): %q", idx, w, visWidth, line)
				}
			}
		}
	})

	// 2. Test TagModal across all states
	t.Run("TagModal", func(t *testing.T) {
		modal := ui.NewTagModal(tagConfigs)
		modal.SetItem(sampleItem, sampleTree)

		states := []ui.TagModalState{
			ui.TagModalSelect,
			ui.TagModalNewName,
			ui.TagModalNewEmoji,
			ui.TagModalNewColor,
		}

		for _, st := range states {
			modal.State = st
			for _, w := range testWidths {
				rendered := modal.Render(w, 24)
				lines := strings.Split(rendered, "\n")
				for idx, line := range lines {
					visWidth := lipgloss.Width(line)
					if visWidth > w {
						t.Errorf("TagModal (state %v) line %d exceeds width %d (got %d): %q", st, idx, w, visWidth, line)
					}
				}
			}
		}
	})

	// 3. Test ConfigModal
	t.Run("ConfigModal", func(t *testing.T) {
		cfg := config.DefaultConfig()
		cm := ui.NewConfigModal(cfg)
		for _, w := range testWidths {
			rendered := cm.Render(w, 24)
			lines := strings.Split(rendered, "\n")
			for idx, line := range lines {
				visWidth := lipgloss.Width(line)
				if visWidth > w {
					t.Errorf("ConfigModal line %d exceeds width %d (got %d): %q", idx, w, visWidth, line)
				}
			}
		}
	})

	// 4. Test HelpModal
	t.Run("HelpModal", func(t *testing.T) {
		help := ui.NewHelpModal()
		for _, w := range testWidths {
			rendered := help.Render(w, 24)
			lines := strings.Split(rendered, "\n")
			for idx, line := range lines {
				visWidth := lipgloss.Width(line)
				if visWidth > w {
					t.Errorf("HelpModal line %d exceeds width %d (got %d): %q", idx, w, visWidth, line)
				}
			}
		}
	})

	// 4. Test QuickHelp Bar
	t.Run("QuickHelp", func(t *testing.T) {
		qh := ui.NewQuickHelp()
		for _, w := range testWidths {
			rendered := qh.Render(w)
			visWidth := lipgloss.Width(rendered)
			if visWidth > w {
				t.Errorf("QuickHelp exceeds width %d (got %d)", w, visWidth)
			}
		}
	})

	// 5. Test StatusBar
	t.Run("StatusBar", func(t *testing.T) {
		sb := ui.NewStatusBar()
		stats := sampleTree.GetStats()
		modes := []ui.AppMode{ui.ModeNormal, ui.ModeInsert, ui.ModeSearch, ui.ModeTagPicker}

		for _, m := range modes {
			for _, w := range testWidths {
				sb.Width = w
				rendered := sb.Render(m, "/path/to/data.txt", true, stats, 1, 10, "Status OK", "")
				visWidth := lipgloss.Width(rendered)
				if visWidth > w {
					t.Errorf("StatusBar (mode %v) exceeds width %d (got %d)", m, w, visWidth)
				}
			}
		}
	})

	// 6. Test DashboardView
	t.Run("DashboardView", func(t *testing.T) {
		dbView := ui.NewDashboardView()
		for _, w := range testWidths {
			dbWidth := int(float64(w) * 0.35)
			if dbWidth > 42 {
				dbWidth = 42
			}
			if dbWidth < 20 {
				dbWidth = 20
			}
			dbView.Width = dbWidth
			dbView.Height = 20
			rendered := dbView.Render(sampleTree, tagConfigs)
			lines := strings.Split(rendered, "\n")
			for idx, line := range lines {
				visWidth := lipgloss.Width(line)
				if visWidth > dbWidth {
					t.Errorf("DashboardView line %d exceeds width %d (got %d): %q", idx, dbWidth, visWidth, line)
				}
			}
		}
	})
}

// PrintMenuSnapshots prints a visual snapshot of each menu layout for developer inspection.
func TestPrintMenuSnapshots(t *testing.T) {
	tagConfigs := config.GetDefaultTagConfigs()
	allBindings := ui.GetAllKeyBindings()

	sampleItem := model.NewTask("t1", "Fix database connection pooling #bug #urgent", model.StatusInProgress)
	sampleTree := model.NewTree()
	sampleTree.Roots = append(sampleTree.Roots, sampleItem)
	sampleTree.SetParents()

	wk := ui.NewWhichKeyModel()
	wk.Active = true
	wk.PrefixKeys = []string{" "}
	wkRendered := wk.Render(allBindings, 80)

	modal := ui.NewTagModal(tagConfigs)
	modal.SetItem(sampleItem, sampleTree)
	modalRendered := modal.Render(80, 24)

	help := ui.NewHelpModal()
	helpRendered := help.Render(80, 24)

	t.Log("\n--- WHICHKEY MENU SNAPSHOT (Width 80) ---\n" + wkRendered)
	t.Log("\n--- TAG MODAL SNAPSHOT (Width 80) ---\n" + modalRendered)
	t.Log("\n--- HELP MODAL SNAPSHOT (Width 80) ---\n" + helpRendered)

	if wkRendered == "" || modalRendered == "" || helpRendered == "" {
		t.Fatalf("Failed to generate menu snapshots")
	}
}
