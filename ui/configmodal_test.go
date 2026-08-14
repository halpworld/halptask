package ui

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/halpworld/halptask/config"
)

func TestConfigModalNavigationAndToggles(t *testing.T) {
	cfg := config.DefaultConfig()
	cfg.AutoSave = true
	cfg.DefaultItemType = "bullet"

	cm := NewConfigModal(cfg)

	if len(cm.Items) == 0 {
		t.Fatalf("expected ConfigModal to load items, got 0")
	}

	// Test navigation down 'j'
	cm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'j'}})
	if cm.SelectedIndex != 1 {
		t.Fatalf("expected SelectedIndex 1 after 'j', got %d", cm.SelectedIndex)
	}

	// Test navigation up 'k'
	cm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'k'}})
	if cm.SelectedIndex != 0 {
		t.Fatalf("expected SelectedIndex 0 after 'k', got %d", cm.SelectedIndex)
	}

	// Test toggling AutoSave (item 0) via Space
	cm.Update(tea.KeyMsg{Type: tea.KeySpace})
	if cm.Config.AutoSave {
		t.Fatalf("expected AutoSave to be false after toggle, got true")
	}

	// Test cycling DefaultItemType (item 1) via Enter
	cm.SelectedIndex = 1
	cm.Update(tea.KeyMsg{Type: tea.KeyEnter})
	if cm.Config.DefaultItemType != "task" {
		t.Fatalf("expected DefaultItemType to be 'task', got %s", cm.Config.DefaultItemType)
	}

	// Test closing modal via Esc
	closeModal, _, _ := cm.Update(tea.KeyMsg{Type: tea.KeyEsc})
	if !closeModal {
		t.Fatalf("expected closeModal to be true on Esc")
	}
}

func TestConfigModalExternalEditorSignal(t *testing.T) {
	cfg := config.DefaultConfig()
	cm := NewConfigModal(cfg)

	// Pressing 'e' anywhere in modal should signal openExternalEditor
	_, _, openEditor := cm.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'e'}})
	if !openEditor {
		t.Fatalf("expected openEditor to be true when pressing 'e'")
	}
}

func TestConfigModalRender(t *testing.T) {
	cfg := config.DefaultConfig()
	cm := NewConfigModal(cfg)

	rendered := cm.Render(80, 24)
	if !strings.Contains(rendered, "HalpTask App Configuration") {
		t.Fatalf("expected title in rendered ConfigModal output")
	}
	if !strings.Contains(rendered, "Auto-Save Changes") {
		t.Fatalf("expected Auto-Save label in rendered ConfigModal output")
	}
}
