package model

import "testing"

func TestParentEffectiveStatus(t *testing.T) {
	for _, tc := range []struct{ a, b, want TaskStatus }{
		{StatusTodo, StatusTodo, StatusTodo},
		{StatusDone, StatusDone, StatusDone},
		{StatusTodo, StatusDone, StatusInProgress},
		{StatusInProgress, StatusTodo, StatusInProgress},
	} {
		parent := NewTask("1", "parent", StatusDone)
		nested := NewTask("2", "nested parent", StatusDone)
		bullet := NewItem("3", "bullet")
		a, b := NewTask("4", "a", tc.a), NewTask("5", "b", tc.b)
		nested.Children = []*Item{a}
		bullet.Children = []*Item{b}
		parent.Children = []*Item{nested, bullet}
		parent.Folded = true
		tree := &Tree{Roots: []*Item{parent}}
		tree.SetParents()
		if got := parent.EffectiveStatus(); got != tc.want {
			t.Fatalf("%s/%s: got %s want %s", tc.a, tc.b, got, tc.want)
		}
		tree.SetStatus(parent.ID, StatusTodo)
		tree.CycleStatus(parent.ID)
		if a.Status != tc.a || b.Status != tc.b || parent.EffectiveStatus() != tc.want {
			t.Fatal("direct parent mutation changed group")
		}
		restored := ItemFromProto(parent.ToProto())
		if restored.EffectiveStatus() != tc.want {
			t.Fatal("round trip lost derived status")
		}
		tree.SetSubtreeStatus(parent.ID, StatusDone)
		if parent.EffectiveStatus() != StatusDone || a.Status != StatusDone || b.Status != StatusDone || bullet.IsTask {
			t.Fatal("bulk status did not preserve bullet/task distinctions")
		}
		parent.Children = nil
		tree.SetStatus(parent.ID, StatusTodo)
		if parent.EffectiveStatus() != StatusTodo {
			t.Fatal("childless task should be editable")
		}
	}
}

func TestIncompleteParentIsNotHiddenOrArchived(t *testing.T) {
	parent := NewTask("1", "parent", StatusDone)
	parent.Children = []*Item{NewTask("2", "unfinished", StatusTodo)}
	tree := &Tree{Roots: []*Item{parent}}
	if len(tree.ArchiveCompleted()) != 0 || tree.DeleteCompleted() != 0 {
		t.Fatal("incomplete subtree removed using stale parent status")
	}
}
