package model

import (
	"crypto/rand"
	"fmt"
	"strings"
)

type Tree struct {
	Roots []*Item
}

func NewTree() *Tree {
	return &Tree{
		Roots: []*Item{},
	}
}

func GenerateID() string {
	b := make([]byte, 8)
	_, _ = rand.Read(b)
	return fmt.Sprintf("%x", b)
}

func (t *Tree) NextID() string {
	maxID := 0
	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			var num int
			if _, err := fmt.Sscanf(item.ID, "%d", &num); err == nil {
				if num > maxID {
					maxID = num
				}
			}
			if len(item.Children) > 0 {
				recurse(item.Children)
			}
		}
	}
	recurse(t.Roots)
	return fmt.Sprintf("%d", maxID+1)
}

func (t *Tree) EnsureIDs() {
	maxID := 0
	var findMax func(items []*Item)
	findMax = func(items []*Item) {
		for _, item := range items {
			var num int
			if _, err := fmt.Sscanf(item.ID, "%d", &num); err == nil {
				if num > maxID {
					maxID = num
				}
			}
			if len(item.Children) > 0 {
				findMax(item.Children)
			}
		}
	}
	findMax(t.Roots)

	var assign func(items []*Item)
	assign = func(items []*Item) {
		for _, item := range items {
			if item.ID == "" {
				maxID++
				item.ID = fmt.Sprintf("%d", maxID)
			}
			if len(item.Children) > 0 {
				assign(item.Children)
			}
		}
	}
	assign(t.Roots)
}

func (t *Tree) SetParents() {
	for _, root := range t.Roots {
		t.setParentsRec(root, nil)
	}
}

func (t *Tree) setParentsRec(item *Item, parent *Item) {
	item.Parent = parent
	for _, child := range item.Children {
		t.setParentsRec(child, item)
	}
}

func (t *Tree) FlattenVisible() []VisibleItem {
	return t.FlattenVisibleFiltered("", false)
}

func (t *Tree) FlattenVisibleFiltered(zoomedID string, hideCompleted bool) []VisibleItem {
	t.SetParents()
	var visible []VisibleItem

	var rootList []*Item
	baseDepth := 0
	if zoomedID != "" {
		zoomedItem := t.FindItem(zoomedID)
		if zoomedItem != nil {
			rootList = []*Item{zoomedItem}
		} else {
			rootList = t.Roots
		}
	} else {
		rootList = t.Roots
	}

	var recurse func(items []*Item, depth int)
	recurse = func(items []*Item, depth int) {
		for _, item := range items {
			if hideCompleted && item.IsTask && item.Status == StatusDone {
				continue
			}
			v := VisibleItem{
				Item:        item,
				Depth:       depth,
				Index:       len(visible),
				HasChildren: len(item.Children) > 0,
				Parent:      item.Parent,
			}
			visible = append(visible, v)
			if !item.Folded && len(item.Children) > 0 {
				recurse(item.Children, depth+1)
			}
		}
	}
	recurse(rootList, baseDepth)
	return visible
}

func (t *Tree) DeleteCompleted() int {
	t.SetParents()
	count := 0
	var filterItems func(items []*Item) []*Item
	filterItems = func(items []*Item) []*Item {
		var result []*Item
		for _, item := range items {
			if item.IsTask && item.Status == StatusDone {
				count++
				continue
			}
			if len(item.Children) > 0 {
				item.Children = filterItems(item.Children)
			}
			result = append(result, item)
		}
		return result
	}
	t.Roots = filterItems(t.Roots)
	t.SetParents()
	return count
}

func (t *Tree) FindItem(id string) *Item {
	var found *Item
	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			if item.ID == id {
				found = item
				return
			}
			if len(item.Children) > 0 {
				recurse(item.Children)
				if found != nil {
					return
				}
			}
		}
	}
	recurse(t.Roots)
	return found
}

func (t *Tree) FindSiblingsAndIndex(id string) ([]*Item, int) {
	item := t.FindItem(id)
	if item == nil {
		return nil, -1
	}
	if item.Parent == nil {
		for idx, root := range t.Roots {
			if root.ID == id {
				return t.Roots, idx
			}
		}
		return nil, -1
	}
	for idx, child := range item.Parent.Children {
		if child.ID == id {
			return item.Parent.Children, idx
		}
	}
	return nil, -1
}

func (t *Tree) InsertBelow(targetID, text string) *Item {
	t.SetParents()
	newItem := NewItem(t.NextID(), text)

	if len(t.Roots) == 0 || targetID == "" {
		t.Roots = append(t.Roots, newItem)
		t.SetParents()
		return newItem
	}

	target := t.FindItem(targetID)
	if target == nil {
		t.Roots = append(t.Roots, newItem)
		t.SetParents()
		return newItem
	}

	// Insert as next sibling after target
	if target.Parent == nil {
		idx := -1
		for i, r := range t.Roots {
			if r.ID == targetID {
				idx = i
				break
			}
		}
		if idx != -1 {
			t.Roots = append(t.Roots[:idx+1], append([]*Item{newItem}, t.Roots[idx+1:]...)...)
		} else {
			t.Roots = append(t.Roots, newItem)
		}
	} else {
		siblings := target.Parent.Children
		idx := -1
		for i, s := range siblings {
			if s.ID == targetID {
				idx = i
				break
			}
		}
		if idx != -1 {
			target.Parent.Children = append(siblings[:idx+1], append([]*Item{newItem}, siblings[idx+1:]...)...)
		} else {
			target.Parent.Children = append(siblings, newItem)
		}
	}
	t.SetParents()
	return newItem
}

func (t *Tree) InsertAbove(targetID, text string) *Item {
	t.SetParents()
	newItem := NewItem(t.NextID(), text)

	if len(t.Roots) == 0 || targetID == "" {
		t.Roots = append([]*Item{newItem}, t.Roots...)
		t.SetParents()
		return newItem
	}

	target := t.FindItem(targetID)
	if target == nil {
		t.Roots = append([]*Item{newItem}, t.Roots...)
		t.SetParents()
		return newItem
	}

	if target.Parent == nil {
		idx := -1
		for i, r := range t.Roots {
			if r.ID == targetID {
				idx = i
				break
			}
		}
		if idx != -1 {
			t.Roots = append(t.Roots[:idx], append([]*Item{newItem}, t.Roots[idx:]...)...)
		} else {
			t.Roots = append([]*Item{newItem}, t.Roots...)
		}
	} else {
		siblings := target.Parent.Children
		idx := -1
		for i, s := range siblings {
			if s.ID == targetID {
				idx = i
				break
			}
		}
		if idx != -1 {
			target.Parent.Children = append(siblings[:idx], append([]*Item{newItem}, siblings[idx:]...)...)
		} else {
			target.Parent.Children = append([]*Item{newItem}, siblings...)
		}
	}
	t.SetParents()
	return newItem
}

func (t *Tree) AddChild(parentID, text string) *Item {
	t.SetParents()
	newItem := NewItem(t.NextID(), text)
	parent := t.FindItem(parentID)
	if parent == nil {
		t.Roots = append(t.Roots, newItem)
		t.SetParents()
		return newItem
	}
	parent.Children = append(parent.Children, newItem)
	parent.Folded = false // unfold parent when adding child
	t.SetParents()
	return newItem
}

func (t *Tree) Delete(id string) string {
	t.SetParents()
	visible := t.FlattenVisible()
	var nextSelectID string

	for i, v := range visible {
		if v.Item.ID == id {
			if i+1 < len(visible) {
				nextSelectID = visible[i+1].Item.ID
			} else if i-1 >= 0 {
				nextSelectID = visible[i-1].Item.ID
			}
			break
		}
	}

	target := t.FindItem(id)
	if target == nil {
		return nextSelectID
	}

	if target.Parent == nil {
		for i, r := range t.Roots {
			if r.ID == id {
				t.Roots = append(t.Roots[:i], t.Roots[i+1:]...)
				break
			}
		}
	} else {
		siblings := target.Parent.Children
		for i, s := range siblings {
			if s.ID == id {
				target.Parent.Children = append(siblings[:i], siblings[i+1:]...)
				break
			}
		}
	}

	t.SetParents()
	return nextSelectID
}

func (t *Tree) Indent(id string) bool {
	t.SetParents()
	item := t.FindItem(id)
	if item == nil {
		return false
	}

	var siblings []*Item
	var idx int

	if item.Parent == nil {
		siblings = t.Roots
		for i, r := range t.Roots {
			if r.ID == id {
				idx = i
				break
			}
		}
		if idx <= 0 {
			return false // No preceding sibling to indent into
		}
		prevSibling := t.Roots[idx-1]
		// Remove from roots
		t.Roots = append(t.Roots[:idx], t.Roots[idx+1:]...)
		// Add to prevSibling's children
		prevSibling.Children = append(prevSibling.Children, item)
		prevSibling.Folded = false
	} else {
		siblings = item.Parent.Children
		for i, s := range siblings {
			if s.ID == id {
				idx = i
				break
			}
		}
		if idx <= 0 {
			return false // No preceding sibling
		}
		prevSibling := siblings[idx-1]
		// Remove from current parent
		item.Parent.Children = append(siblings[:idx], siblings[idx+1:]...)
		// Add to prevSibling
		prevSibling.Children = append(prevSibling.Children, item)
		prevSibling.Folded = false
	}

	t.SetParents()
	return true
}

func (t *Tree) Unindent(id string) bool {
	t.SetParents()
	item := t.FindItem(id)
	if item == nil || item.Parent == nil {
		return false // Already at root level
	}

	parent := item.Parent
	grandParent := parent.Parent

	// Remove item from parent.Children
	pSiblings := parent.Children
	pIdx := -1
	for i, s := range pSiblings {
		if s.ID == id {
			pIdx = i
			break
		}
	}
	if pIdx == -1 {
		return false
	}
	parent.Children = append(pSiblings[:pIdx], pSiblings[pIdx+1:]...)

	// Insert item after parent in grandParent or roots
	if grandParent == nil {
		parentIdx := -1
		for i, r := range t.Roots {
			if r.ID == parent.ID {
				parentIdx = i
				break
			}
		}
		if parentIdx != -1 {
			t.Roots = append(t.Roots[:parentIdx+1], append([]*Item{item}, t.Roots[parentIdx+1:]...)...)
		} else {
			t.Roots = append(t.Roots, item)
		}
	} else {
		gpSiblings := grandParent.Children
		parentIdx := -1
		for i, s := range gpSiblings {
			if s.ID == parent.ID {
				parentIdx = i
				break
			}
		}
		if parentIdx != -1 {
			grandParent.Children = append(gpSiblings[:parentIdx+1], append([]*Item{item}, gpSiblings[parentIdx+1:]...)...)
		} else {
			grandParent.Children = append(gpSiblings, item)
		}
	}

	t.SetParents()
	return true
}

func (t *Tree) MoveUp(id string) bool {
	t.SetParents()
	item := t.FindItem(id)
	if item == nil {
		return false
	}

	if item.Parent == nil {
		for i, r := range t.Roots {
			if r.ID == id {
				if i == 0 {
					return false
				}
				t.Roots[i], t.Roots[i-1] = t.Roots[i-1], t.Roots[i]
				return true
			}
		}
	} else {
		siblings := item.Parent.Children
		for i, s := range siblings {
			if s.ID == id {
				if i == 0 {
					return false
				}
				item.Parent.Children[i], item.Parent.Children[i-1] = item.Parent.Children[i-1], item.Parent.Children[i]
				return true
			}
		}
	}
	return false
}

func (t *Tree) MoveDown(id string) bool {
	t.SetParents()
	item := t.FindItem(id)
	if item == nil {
		return false
	}

	if item.Parent == nil {
		for i, r := range t.Roots {
			if r.ID == id {
				if i == len(t.Roots)-1 {
					return false
				}
				t.Roots[i], t.Roots[i+1] = t.Roots[i+1], t.Roots[i]
				return true
			}
		}
	} else {
		siblings := item.Parent.Children
		for i, s := range siblings {
			if s.ID == id {
				if i == len(siblings)-1 {
					return false
				}
				item.Parent.Children[i], item.Parent.Children[i+1] = item.Parent.Children[i+1], item.Parent.Children[i]
				return true
			}
		}
	}
	return false
}

func (t *Tree) ToggleTask(id string) {
	item := t.FindItem(id)
	if item == nil {
		return
	}
	if !item.IsTask {
		item.IsTask = true
		if item.Status == StatusNone {
			item.Status = StatusTodo
		}
	} else {
		item.IsTask = false
		item.Status = StatusNone
	}
}

func (t *Tree) CycleStatus(id string) {
	item := t.FindItem(id)
	if item == nil {
		return
	}
	if !item.IsTask {
		item.IsTask = true
		item.Status = StatusTodo
		return
	}
	switch item.Status {
	case StatusTodo:
		item.Status = StatusInProgress
	case StatusInProgress:
		item.Status = StatusDone
	case StatusDone:
		item.Status = StatusTodo
	default:
		item.Status = StatusTodo
	}
}

func (t *Tree) SetStatus(id string, status TaskStatus) {
	item := t.FindItem(id)
	if item == nil {
		return
	}
	item.IsTask = true
	item.Status = status
}

func (t *Tree) ToggleFold(id string) {
	item := t.FindItem(id)
	if item != nil && len(item.Children) > 0 {
		item.Folded = !item.Folded
	}
}

func (t *Tree) Fold(id string) {
	item := t.FindItem(id)
	if item != nil && len(item.Children) > 0 {
		item.Folded = true
	}
}

func (t *Tree) Unfold(id string) {
	item := t.FindItem(id)
	if item != nil {
		item.Folded = false
	}
}

func (t *Tree) FoldAll() {
	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			if len(item.Children) > 0 {
				item.Folded = true
				recurse(item.Children)
			}
		}
	}
	recurse(t.Roots)
}

func (t *Tree) UnfoldAll() {
	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			item.Folded = false
			if len(item.Children) > 0 {
				recurse(item.Children)
			}
		}
	}
	recurse(t.Roots)
}

func (t *Tree) Clone() *Tree {
	newTree := NewTree()
	for _, r := range t.Roots {
		newTree.Roots = append(newTree.Roots, r.Clone())
	}
	newTree.SetParents()
	return newTree
}

type TaskStats struct {
	Total      int
	Todo       int
	InProgress int
	Done       int
}

func (t *Tree) GetStats() TaskStats {
	var stats TaskStats
	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			if item.IsTask {
				stats.Total++
				switch item.Status {
				case StatusTodo:
					stats.Todo++
				case StatusInProgress:
					stats.InProgress++
				case StatusDone:
					stats.Done++
				}
			}
			if len(item.Children) > 0 {
				recurse(item.Children)
			}
		}
	}
	recurse(t.Roots)
	return stats
}

func (t *Tree) Search(query string) []string {
	query = strings.ToLower(query)
	cleanQuery := strings.TrimPrefix(query, "#")
	var matchedIDs []string
	if query == "" {
		return matchedIDs
	}
	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			if strings.Contains(strings.ToLower(item.Text), query) || strings.EqualFold(item.ID, cleanQuery) {
				matchedIDs = append(matchedIDs, item.ID)
			}
			if len(item.Children) > 0 {
				recurse(item.Children)
			}
		}
	}
	recurse(t.Roots)
	return matchedIDs
}

// GetEffectiveTags returns direct tags on the item and tags dynamically inherited from parent hierarchy.
func (t *Tree) GetEffectiveTags(item *Item) (direct []string, inherited []string) {
	if item == nil {
		return nil, nil
	}
	direct = append([]string{}, item.Tags...)

	directSet := make(map[string]bool)
	for _, dt := range direct {
		directSet[strings.ToLower(dt)] = true
	}

	inheritedMap := make(map[string]bool)
	curr := item.Parent
	for curr != nil {
		for _, pt := range curr.Tags {
			tLower := strings.ToLower(pt)
			if !directSet[tLower] && !inheritedMap[tLower] {
				inheritedMap[tLower] = true
				inherited = append(inherited, tLower)
			}
		}
		curr = curr.Parent
	}
	return direct, inherited
}

// GetAllTags returns combined slice of direct and inherited tags without duplicates.
func (t *Tree) GetAllTags(item *Item) []string {
	direct, inherited := t.GetEffectiveTags(item)
	all := append([]string{}, direct...)
	all = append(all, inherited...)
	return all
}

// SearchByTag returns matching item IDs that have the given tag (direct or inherited).
func (t *Tree) SearchByTag(tag string) []string {
	tag = strings.ToLower(strings.TrimPrefix(tag, "#"))
	if tag == "" {
		return nil
	}
	t.SetParents()
	var matchedIDs []string

	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			allTags := t.GetAllTags(item)
			hasTag := false
			for _, itemTag := range allTags {
				if strings.EqualFold(itemTag, tag) {
					hasTag = true
					break
				}
			}
			if hasTag {
				matchedIDs = append(matchedIDs, item.ID)
			}
			if len(item.Children) > 0 {
				recurse(item.Children)
			}
		}
	}
	recurse(t.Roots)
	return matchedIDs
}

type TaskWithContext struct {
	Item       *Item
	ParentItem *Item
	ParentPath string
}

// GetInProgressTasks returns all items in the tree that have Status == StatusInProgress, along with parent context.
func (t *Tree) GetInProgressTasks() []TaskWithContext {
	t.SetParents()
	var result []TaskWithContext

	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			if item.IsTask && item.Status == StatusInProgress {
				parentPath := ""
				if item.Parent != nil {
					var ancestorTexts []string
					curr := item.Parent
					for curr != nil {
						if curr.Text != "" {
							ancestorTexts = append([]string{curr.Text}, ancestorTexts...)
						}
						curr = curr.Parent
					}
					parentPath = strings.Join(ancestorTexts, " > ")
				}
				result = append(result, TaskWithContext{
					Item:       item,
					ParentItem: item.Parent,
					ParentPath: parentPath,
				})
			}
			if len(item.Children) > 0 {
				recurse(item.Children)
			}
		}
	}
	recurse(t.Roots)
	return result
}
