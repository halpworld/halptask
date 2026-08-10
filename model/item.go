package model

import (
	"strings"
	"time"

	storagepb "github.com/kenth/halptask/proto/v1"
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
		Status:    TaskStatusToProto(i.Status),
		Folded:    i.Folded,
		Tags:      append([]string{}, i.Tags...),
		CreatedAt: i.CreatedAt,
		UpdatedAt: i.UpdatedAt,
		NodeId:    i.NodeID,
		Deleted:   i.Deleted,
		Version:   i.Version,
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
		Text:      pb.Text,
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
