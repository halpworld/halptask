package ui

type KeyBinding struct {
	Keys        []string // e.g. [" ", "b", "n"] or ["o"]
	KeyString   string   // display string e.g. "<space> b n" or "o"
	Label       string   // e.g. "new below"
	Category    string   // e.g. "Bullets", "Tasks", "Folds", "File"
	Description string
}

func GetAllKeyBindings() []KeyBinding {
	return []KeyBinding{
		// Leader Bullet operations
		{Keys: []string{" ", "b", "n"}, KeyString: "n", Label: "new bullet below", Category: "Bullets"},
		{Keys: []string{" ", "b", "N"}, KeyString: "N", Label: "new bullet above", Category: "Bullets"},
		{Keys: []string{" ", "b", "c"}, KeyString: "c", Label: "add child bullet", Category: "Bullets"},
		{Keys: []string{" ", "b", "e"}, KeyString: "e", Label: "edit bullet text", Category: "Bullets"},
		{Keys: []string{" ", "b", "d"}, KeyString: "d", Label: "delete bullet", Category: "Bullets"},
		{Keys: []string{" ", "b", "i"}, KeyString: "i", Label: "indent bullet (Tab)", Category: "Bullets"},
		{Keys: []string{" ", "b", "o"}, KeyString: "o", Label: "unindent bullet (S-Tab)", Category: "Bullets"},
		{Keys: []string{" ", "b", "j"}, KeyString: "j", Label: "move bullet down", Category: "Bullets"},
		{Keys: []string{" ", "b", "k"}, KeyString: "k", Label: "move bullet up", Category: "Bullets"},
		{Keys: []string{" ", "b", "t"}, KeyString: "t", Label: "toggle task status", Category: "Bullets"},
		{Keys: []string{" ", "b", "D"}, KeyString: "D", Label: "toggle default type", Category: "Bullets"},

		// Leader Task operations
		{Keys: []string{" ", "t", "t"}, KeyString: "t", Label: "toggle bullet <-> task", Category: "Tasks"},
		{Keys: []string{" ", "t", "c"}, KeyString: "c", Label: "cycle task status", Category: "Tasks"},
		{Keys: []string{" ", "t", "d"}, KeyString: "d", Label: "mark done [x]", Category: "Tasks"},
		{Keys: []string{" ", "t", "p"}, KeyString: "p", Label: "mark in-progress [~]", Category: "Tasks"},
		{Keys: []string{" ", "t", "s"}, KeyString: "s", Label: "mark todo [ ]", Category: "Tasks"},
		{Keys: []string{" ", "t", "a"}, KeyString: "a", Label: "manage tags / labels", Category: "Tasks"},
		{Keys: []string{" ", "t", "n"}, KeyString: "n", Label: "open/edit task note", Category: "Tasks"},
		{Keys: []string{" ", "n"}, KeyString: "n", Label: "open/edit task note", Category: "Tasks"},
		{Keys: []string{" ", "t", "f"}, KeyString: "f", Label: "toggle/exit focus mode", Category: "Tasks"},
		{Keys: []string{" ", "t", "h"}, KeyString: "h", Label: "toggle hide completed", Category: "Tasks"},
		{Keys: []string{" ", "t", "D"}, KeyString: "D", Label: "toggle default type", Category: "Tasks"},

		// Leader Focus Mode operations
		{Keys: []string{" ", "f", "o"}, KeyString: "o", Label: "toggle/exit focus mode", Category: "Focus"},
		{Keys: []string{" ", "f", "f"}, KeyString: "f", Label: "toggle/exit focus mode", Category: "Focus"},
		{Keys: []string{" ", "f", "c"}, KeyString: "c", Label: "clear current focus", Category: "Focus"},

		// Leader Archive operations
		{Keys: []string{" ", "a", "a"}, KeyString: "a", Label: "archive selected item", Category: "Archive"},
		{Keys: []string{" ", "a", "c"}, KeyString: "c", Label: "archive completed tasks", Category: "Archive"},
		{Keys: []string{" ", "a", "v"}, KeyString: "v", Label: "view / restore archive", Category: "Archive"},
		{Keys: []string{" ", "a", "r"}, KeyString: "r", Label: "view / restore archive", Category: "Archive"},

		// Leader Fold & Zoom operations
		{Keys: []string{" ", "z", "c"}, KeyString: "c", Label: "close fold", Category: "Folds"},
		{Keys: []string{" ", "z", "o"}, KeyString: "o", Label: "open fold", Category: "Folds"},
		{Keys: []string{" ", "z", "a"}, KeyString: "a", Label: "toggle fold", Category: "Folds"},
		{Keys: []string{" ", "z", "z"}, KeyString: "z", Label: "zoom / hoist subtree", Category: "Folds"},
		{Keys: []string{" ", "z", "M"}, KeyString: "M", Label: "close all folds", Category: "Folds"},
		{Keys: []string{" ", "z", "R"}, KeyString: "R", Label: "open all folds", Category: "Folds"},

		// Leader Configuration & Options
		{Keys: []string{" ", "c", "c"}, KeyString: "c", Label: "open config dashboard", Category: "Config"},
		{Keys: []string{" ", "c", "a"}, KeyString: "a", Label: "toggle auto-save", Category: "Config"},
		{Keys: []string{" ", "c", "d"}, KeyString: "d", Label: "toggle default item type", Category: "Config"},
		{Keys: []string{" ", "c", "D"}, KeyString: "D", Label: "toggle dashboard pane", Category: "Config"},
		{Keys: []string{" ", "c", "t"}, KeyString: "t", Label: "cycle visual theme", Category: "Config"},
		{Keys: []string{" ", "c", "w"}, KeyString: "w", Label: "toggle whichkey popup", Category: "Config"},
		{Keys: []string{" ", "c", "e"}, KeyString: "e", Label: "open config.yaml ($EDITOR)", Category: "Config"},

		// Leader File & System
		{Keys: []string{" ", "d"}, KeyString: "d", Label: "toggle overview dashboard", Category: "Leader"},
		{Keys: []string{" ", "w"}, KeyString: "w", Label: "save file", Category: "Leader"},
		{Keys: []string{" ", "s"}, KeyString: "s", Label: "save file", Category: "Leader"},
		{Keys: []string{" ", "e", "e"}, KeyString: "e", Label: "toggle encryption", Category: "Encrypt"},
		{Keys: []string{" ", "e", "p"}, KeyString: "p", Label: "set passphrase", Category: "Encrypt"},
		{Keys: []string{" ", "g", "i"}, KeyString: "gi", Label: "jump to item by ID", Category: "Nav"},
		{Keys: []string{" ", "/"}, KeyString: "/", Label: "search items", Category: "Leader"},
		{Keys: []string{" ", "?"}, KeyString: "?", Label: "show help", Category: "Leader"},
		{Keys: []string{" ", "U"}, KeyString: "U", Label: "check/install update", Category: "Leader"},
		{Keys: []string{" ", "q"}, KeyString: "q", Label: "quit halptask", Category: "Leader"},

		// Direct Vim Normal Keymaps
		{Keys: []string{"j"}, KeyString: "j", Label: "down", Category: "Nav"},
		{Keys: []string{"k"}, KeyString: "k", Label: "up", Category: "Nav"},
		{Keys: []string{"h"}, KeyString: "h", Label: "close fold / parent", Category: "Nav"},
		{Keys: []string{"l"}, KeyString: "l", Label: "open fold / child", Category: "Nav"},
		{Keys: []string{"g", "g"}, KeyString: "gg", Label: "go to top", Category: "Nav"},
		{Keys: []string{"g", "i"}, KeyString: "gi", Label: "jump to item by ID", Category: "Nav"},
		{Keys: []string{"G"}, KeyString: "G", Label: "go to bottom", Category: "Nav"},

		{Keys: []string{"o", "o"}, KeyString: "oo", Label: "new bullet below", Category: "Edit"},
		{Keys: []string{"o", "c"}, KeyString: "oc", Label: "add child bullet", Category: "Edit"},
		{Keys: []string{"O"}, KeyString: "O", Label: "new bullet above", Category: "Edit"},
		{Keys: []string{"i"}, KeyString: "i", Label: "edit text", Category: "Edit"},
		{Keys: []string{"a"}, KeyString: "a", Label: "edit text", Category: "Edit"},
		{Keys: []string{"d", "d"}, KeyString: "dd", Label: "delete bullet", Category: "Edit"},
		{Keys: []string{"x"}, KeyString: "x", Label: "delete bullet", Category: "Edit"},

		{Keys: []string{"tab"}, KeyString: "tab", Label: "indent", Category: "Edit"},
		{Keys: []string{"shift+tab"}, KeyString: "shift+tab", Label: "unindent", Category: "Edit"},

		{Keys: []string{"J"}, KeyString: "J", Label: "move down", Category: "Edit"},
		{Keys: []string{"K"}, KeyString: "K", Label: "move up", Category: "Edit"},

		{Keys: []string{"enter"}, KeyString: "enter", Label: "toggle fold", Category: "Folds"},
		{Keys: []string{"c"}, KeyString: "c", Label: "clear & edit text", Category: "Edit"},
		{Keys: []string{"w", "w"}, KeyString: "ww", Label: "quick save file", Category: "Leader"},
		{Keys: []string{"f", "c"}, KeyString: "fc", Label: "toggle hide completed", Category: "Tasks"},
		{Keys: []string{"d", "a"}, KeyString: "da", Label: "delete all completed tasks", Category: "Tasks"},
		{Keys: []string{"f", "f"}, KeyString: "ff", Label: "zoom / hoist subtree", Category: "Nav"},
		{Keys: []string{"t"}, KeyString: "t", Label: "toggle/cycle task status", Category: "Tasks"},
		{Keys: []string{"f", "o"}, KeyString: "fo", Label: "toggle current focus task", Category: "Tasks"},
		{Keys: []string{"T"}, KeyString: "T", Label: "manage tags / labels", Category: "Tasks"},
		{Keys: []string{"N"}, KeyString: "N", Label: "open/edit task note", Category: "Tasks"},
		{Keys: []string{"u"}, KeyString: "u", Label: "undo", Category: "Edit"},
		{Keys: []string{"ctrl+r"}, KeyString: "ctrl+r", Label: "redo", Category: "Edit"},
		{Keys: []string{"/"}, KeyString: "/", Label: "search", Category: "Nav"},
	}
}
