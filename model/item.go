package model

import "strings"

type TaskStatus string

const (
	StatusNone       TaskStatus = ""
	StatusTodo       TaskStatus = "todo"
	StatusInProgress TaskStatus = "in_progress"
	StatusDone       TaskStatus = "done"
)

type Item struct {
	ID       string     `json:"id"`
	Text     string     `json:"text"`
	IsTask   bool       `json:"is_task"`
	Status   TaskStatus `json:"status"`
	Folded   bool       `json:"folded"`
	Tags     []string   `json:"tags,omitempty"`
	Children []*Item    `json:"children,omitempty"`
	Parent   *Item      `json:"-"`
}

func NewItem(id, text string) *Item {
	return &Item{
		ID:       id,
		Text:     text,
		IsTask:   false,
		Status:   StatusNone,
		Folded:   false,
		Tags:     []string{},
		Children: []*Item{},
	}
}

func NewTask(id, text string, status TaskStatus) *Item {
	if status == StatusNone {
		status = StatusTodo
	}
	return &Item{
		ID:       id,
		Text:     text,
		IsTask:   true,
		Status:   status,
		Folded:   false,
		Tags:     []string{},
		Children: []*Item{},
	}
}

func (i *Item) HasDirectTag(tag string) bool {
	if i == nil {
		return false
	}
	for _, t := range i.Tags {
		if strings.EqualFold(t, tag) {
			return true
		}
	}
	return false
}

func (i *Item) AddTag(tag string) {
	if i == nil || tag == "" {
		return
	}
	if !i.HasDirectTag(tag) {
		i.Tags = append(i.Tags, strings.ToLower(tag))
	}
}

func (i *Item) RemoveTag(tag string) {
	if i == nil {
		return
	}
	var newTags []string
	for _, t := range i.Tags {
		if !strings.EqualFold(t, tag) {
			newTags = append(newTags, t)
		}
	}
	i.Tags = newTags
}

func (i *Item) ToggleTag(tag string) {
	if i.HasDirectTag(tag) {
		i.RemoveTag(tag)
	} else {
		i.AddTag(tag)
	}
}

func (i *Item) Clone() *Item {
	if i == nil {
		return nil
	}
	tagsClone := make([]string, len(i.Tags))
	copy(tagsClone, i.Tags)
	newItem := &Item{
		ID:     i.ID,
		Text:   i.Text,
		IsTask: i.IsTask,
		Status: i.Status,
		Folded: i.Folded,
		Tags:   tagsClone,
	}
	for _, child := range i.Children {
		childClone := child.Clone()
		childClone.Parent = newItem
		newItem.Children = append(newItem.Children, childClone)
	}
	return newItem
}

type VisibleItem struct {
	Item        *Item
	Depth       int
	Index       int // index in flat visible list
	HasChildren bool
	Parent      *Item
}
