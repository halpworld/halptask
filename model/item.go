package model

import (
	"strings"
	"time"

	storagepb "github.com/halpworld/halptask/proto/v1"
)

type TaskStatus string

const (
	StatusNone       TaskStatus = ""
	StatusTodo       TaskStatus = "todo"
	StatusInProgress TaskStatus = "in_progress"
	StatusDone       TaskStatus = "done"
)

type Item struct {
	ID        string     `json:"id"`
	Text      string     `json:"text"`
	IsTask    bool       `json:"is_task"`
	Status    TaskStatus `json:"status"`
	Folded    bool       `json:"folded"`
	Tags      []string   `json:"tags,omitempty"`
	Children  []*Item    `json:"children,omitempty"`
	CreatedAt int64      `json:"created_at,omitempty"`
	UpdatedAt int64      `json:"updated_at,omitempty"`
	NodeID    string     `json:"node_id,omitempty"`
	Deleted   bool       `json:"deleted,omitempty"`
	Version   uint64     `json:"version,omitempty"`
	Note      string     `json:"note,omitempty"`
	IsFocused bool       `json:"is_focused,omitempty"`
	DueDate   int64      `json:"due_date,omitempty"`
	DueText   string     `json:"due_text,omitempty"`
	Parent    *Item      `json:"-"`
}

func NewItem(id, text string) *Item {
	now := time.Now().UnixNano()
	return &Item{
		ID:        id,
		Text:      text,
		IsTask:    false,
		Status:    StatusNone,
		Folded:    false,
		Tags:      []string{},
		Children:  []*Item{},
		CreatedAt: now,
		UpdatedAt: now,
		Version:   1,
	}
}

func NewTask(id, text string, status TaskStatus) *Item {
	if status == StatusNone {
		status = StatusTodo
	}
	now := time.Now().UnixNano()
	return &Item{
		ID:        id,
		Text:      text,
		IsTask:    true,
		Status:    status,
		Folded:    false,
		Tags:      []string{},
		Children:  []*Item{},
		CreatedAt: now,
		UpdatedAt: now,
		Version:   1,
	}
}

// HasSubtasks includes tasks nested beneath plain bullet points.
func (i *Item) HasSubtasks() bool {
	for _, child := range i.Children {
		if child.IsTask || child.HasSubtasks() {
			return true
		}
	}
	return false
}

// EffectiveStatus derives a parent task's state from its descendant tasks.
// Mixed todo/done work counts as in progress; bullets do not count as work.
func (i *Item) EffectiveStatus() TaskStatus {
	if !i.IsTask {
		return i.Status
	}
	todo, done, active := false, false, false
	var visit func(*Item)
	visit = func(node *Item) {
		if node.IsTask && !node.HasSubtasks() {
			switch node.Status {
			case StatusDone:
				done = true
			case StatusInProgress:
				active = true
			default:
				todo = true
			}
		}
		for _, child := range node.Children {
			visit(child)
		}
	}
	for _, child := range i.Children {
		visit(child)
	}
	if active || (todo && done) {
		return StatusInProgress
	}
	if done {
		return StatusDone
	}
	if todo {
		return StatusTodo
	}
	return i.Status
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

func (i *Item) HasDueDate() bool {
	return i != nil && i.DueDate > 0
}

func (i *Item) IsOverdue(now time.Time) bool {
	if !i.HasDueDate() || i.EffectiveStatus() == StatusDone {
		return false
	}
	dueDate := time.Unix(i.DueDate, 0).In(now.Location())
	todayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	dueDayStart := time.Date(dueDate.Year(), dueDate.Month(), dueDate.Day(), 0, 0, 0, 0, dueDate.Location())
	return dueDayStart.Before(todayStart)
}

func (i *Item) IsDueToday(now time.Time) bool {
	if !i.HasDueDate() {
		return false
	}
	dueDate := time.Unix(i.DueDate, 0).In(now.Location())
	todayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	dueDayStart := time.Date(dueDate.Year(), dueDate.Month(), dueDate.Day(), 0, 0, 0, 0, dueDate.Location())
	return dueDayStart.Equal(todayStart)
}

func (i *Item) SetDueDate(t time.Time, raw string) {
	if i == nil {
		return
	}
	i.DueDate = t.Unix()
	i.DueText = raw
	i.UpdatedAt = time.Now().UnixNano()
}

func (i *Item) ClearDueDate() {
	if i == nil {
		return
	}
	i.DueDate = 0
	i.DueText = ""
	i.UpdatedAt = time.Now().UnixNano()
}

func (i *Item) Clone() *Item {
	if i == nil {
		return nil
	}
	tagsClone := make([]string, len(i.Tags))
	copy(tagsClone, i.Tags)
	newItem := &Item{
		ID:        i.ID,
		Text:      i.Text,
		IsTask:    i.IsTask,
		Status:    i.Status,
		Folded:    i.Folded,
		Tags:      tagsClone,
		CreatedAt: i.CreatedAt,
		UpdatedAt: i.UpdatedAt,
		NodeID:    i.NodeID,
		Deleted:   i.Deleted,
		Version:   i.Version,
		Note:      i.Note,
		IsFocused: i.IsFocused,
		DueDate:   i.DueDate,
		DueText:   i.DueText,
	}
	for _, child := range i.Children {
		childClone := child.Clone()
		childClone.Parent = newItem
		newItem.Children = append(newItem.Children, childClone)
	}
	return newItem
}

func TaskStatusToProto(s TaskStatus) storagepb.TaskStatus {
	switch s {
	case StatusNone:
		return storagepb.TaskStatus_TASK_STATUS_NONE
	case StatusTodo:
		return storagepb.TaskStatus_TASK_STATUS_TODO
	case StatusInProgress:
		return storagepb.TaskStatus_TASK_STATUS_IN_PROGRESS
	case StatusDone:
		return storagepb.TaskStatus_TASK_STATUS_DONE
	default:
		return storagepb.TaskStatus_TASK_STATUS_NONE
	}
}

func TaskStatusFromProto(s storagepb.TaskStatus) TaskStatus {
	switch s {
	case storagepb.TaskStatus_TASK_STATUS_NONE:
		return StatusNone
	case storagepb.TaskStatus_TASK_STATUS_TODO:
		return StatusTodo
	case storagepb.TaskStatus_TASK_STATUS_IN_PROGRESS:
		return StatusInProgress
	case storagepb.TaskStatus_TASK_STATUS_DONE:
		return StatusDone
	default:
		return StatusNone
	}
}

func (i *Item) ToProto() *storagepb.ItemProto {
	if i == nil {
		return nil
	}
	pb := &storagepb.ItemProto{
		Id:        i.ID,
		Text:      i.Text,
		IsTask:    i.IsTask,
		Status:    TaskStatusToProto(i.EffectiveStatus()),
		Folded:    i.Folded,
		Tags:      append([]string{}, i.Tags...),
		CreatedAt: i.CreatedAt,
		UpdatedAt: i.UpdatedAt,
		NodeId:    i.NodeID,
		Deleted:   i.Deleted,
		Version:   i.Version,
		Note:      i.Note,
		IsFocused: i.IsFocused,
		DueDate:   i.DueDate,
		DueText:   i.DueText,
	}
	for _, child := range i.Children {
		if childProto := child.ToProto(); childProto != nil {
			pb.Children = append(pb.Children, childProto)
		}
	}
	return pb
}

func ItemFromProto(pb *storagepb.ItemProto) *Item {
	if pb == nil {
		return nil
	}
	tags := append([]string{}, pb.Tags...)
	item := &Item{
		ID:        pb.Id,
		Text:      SanitizeTerminalEscapeArtifacts(pb.Text),
		IsTask:    pb.IsTask,
		Status:    TaskStatusFromProto(pb.Status),
		Folded:    pb.Folded,
		Tags:      tags,
		Children:  []*Item{},
		CreatedAt: pb.CreatedAt,
		UpdatedAt: pb.UpdatedAt,
		NodeID:    pb.NodeId,
		Deleted:   pb.Deleted,
		Version:   pb.Version,
		Note:      SanitizeTerminalEscapeArtifacts(pb.Note),
		IsFocused: pb.IsFocused,
		DueDate:   pb.DueDate,
		DueText:   pb.DueText,
	}
	for _, childProto := range pb.Children {
		if child := ItemFromProto(childProto); child != nil {
			child.Parent = item
			item.Children = append(item.Children, child)
		}
	}
	return item
}

type VisibleItem struct {
	Item        *Item
	Depth       int
	Index       int // index in flat visible list
	HasChildren bool
	Parent      *Item
}
