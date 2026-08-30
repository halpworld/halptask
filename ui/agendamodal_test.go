package ui

import (
	"testing"
	"time"

	"github.com/halpworld/halptask/model"
)

func TestAgendaModalCategorizationAndFilter(t *testing.T) {
	tree := model.NewTree()
	now := time.Now()

	// 1. Overdue task (2 days ago)
	t1 := tree.InsertBelow("", "Overdue urgent report")
	t1.IsTask = true
	t1.Status = model.StatusTodo
	t1.DueDate = now.AddDate(0, 0, -2).Unix()
	t1.DueText = "-2d"

	// 2. Due today task
	t2 := tree.InsertBelow("", "Finish today task")
	t2.IsTask = true
	t2.Status = model.StatusInProgress
	t2.DueDate = now.Unix()
	t2.DueText = "today"

	// 3. This week task (3 days from now)
	t3 := tree.InsertBelow("", "Midweek milestone")
	t3.IsTask = true
	t3.Status = model.StatusTodo
	t3.DueDate = now.AddDate(0, 0, 3).Unix()
	t3.DueText = "+3d"

	// 4. Upcoming task (14 days from now)
	t4 := tree.InsertBelow("", "Future release sprint")
	t4.IsTask = true
	t4.Status = model.StatusTodo
	t4.DueDate = now.AddDate(0, 0, 14).Unix()
	t4.DueText = "+2w"

	// 5. Completed task with due date
	t5 := tree.InsertBelow("", "Completed legacy review")
	t5.IsTask = true
	t5.Status = model.StatusDone
	t5.DueDate = now.AddDate(0, 0, -5).Unix()
	t5.DueText = "-5d"

	modal := NewAgendaModal()
	modal.Open(tree)

	if len(modal.Entries) != 5 {
		t.Fatalf("expected 5 agenda entries, got %d", len(modal.Entries))
	}

	// Verify sections
	if modal.Entries[0].Section != SectionOverdue || modal.Entries[0].Item.ID != t1.ID {
		t.Errorf("entry 0 should be overdue item t1")
	}
	if modal.Entries[1].Section != SectionToday || modal.Entries[1].Item.ID != t2.ID {
		t.Errorf("entry 1 should be today item t2")
	}
	if modal.Entries[2].Section != SectionThisWeek || modal.Entries[2].Item.ID != t3.ID {
		t.Errorf("entry 2 should be this week item t3")
	}
	if modal.Entries[3].Section != SectionUpcoming || modal.Entries[3].Item.ID != t4.ID {
		t.Errorf("entry 3 should be upcoming item t4")
	}
	if modal.Entries[4].Section != SectionCompleted || modal.Entries[4].Item.ID != t5.ID {
		t.Errorf("entry 4 should be completed item t5")
	}

	// Test navigation
	if modal.SelectedItem().ID != t1.ID {
		t.Errorf("expected selected item t1, got %v", modal.SelectedItem())
	}
	modal.Next()
	if modal.SelectedItem().ID != t2.ID {
		t.Errorf("expected selected item t2 after Next(), got %v", modal.SelectedItem())
	}
	modal.Prev()
	if modal.SelectedItem().ID != t1.ID {
		t.Errorf("expected selected item t1 after Prev(), got %v", modal.SelectedItem())
	}

	// Test filtering
	modal.SearchInput.SetValue("milestone")
	modal.ApplyFilter()
	if len(modal.Filtered) != 1 || modal.Filtered[0].Item.ID != t3.ID {
		t.Errorf("expected filtered to match t3 (milestone), got %d entries", len(modal.Filtered))
	}

	modal.SearchInput.SetValue("")
	modal.ApplyFilter()
	if len(modal.Filtered) != 5 {
		t.Errorf("expected reset filter to restore 5 entries, got %d", len(modal.Filtered))
	}

	// Render snapshot test
	rendered := modal.Render()
	if len(rendered) == 0 {
		t.Errorf("expected non-empty rendered modal")
	}
}
