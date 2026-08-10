package storagepb

import (
	"encoding/binary"
	"errors"
	"fmt"
	"io"
)

type TaskStatus int32

const (
	TaskStatus_TASK_STATUS_UNSPECIFIED TaskStatus = 0
	TaskStatus_TASK_STATUS_NONE        TaskStatus = 1
	TaskStatus_TASK_STATUS_TODO        TaskStatus = 2
	TaskStatus_TASK_STATUS_IN_PROGRESS TaskStatus = 3
	TaskStatus_TASK_STATUS_DONE        TaskStatus = 4
)

type ItemProto struct {
	Id        string
	Text      string
	IsTask    bool
	Status    TaskStatus
	Folded    bool
	Tags      []string
	Children  []*ItemProto
	CreatedAt int64
	UpdatedAt int64
	NodeId    string
	Deleted   bool
	Version   uint64
}

type TreeProto struct {
	SchemaVersion uint32
	Roots         []*ItemProto
	LastModified  int64
}

func MarshalTreeProto(t *TreeProto) ([]byte, error) {
	if t == nil {
		return nil, errors.New("cannot marshal nil TreeProto")
	}
	var buf []byte

	// Field 1: schema_version (uint32, varint)
	if t.SchemaVersion != 0 {
		buf = appendVarintField(buf, 1, uint64(t.SchemaVersion))
	}

	// Field 2: roots (repeated ItemProto, length-delimited)
	for _, root := range t.Roots {
		if root == nil {
			continue
		}
		itemBytes, err := MarshalItemProto(root)
		if err != nil {
			return nil, err
		}
		buf = appendBytesField(buf, 2, itemBytes)
	}

	// Field 3: last_modified (int64, varint)
	if t.LastModified != 0 {
		buf = appendVarintField(buf, 3, uint64(t.LastModified))
	}

	return buf, nil
}

func MarshalItemProto(item *ItemProto) ([]byte, error) {
	if item == nil {
		return nil, nil
	}
	var buf []byte

	// Field 1: id (string)
	if item.Id != "" {
		buf = appendStringField(buf, 1, item.Id)
	}
	// Field 2: text (string)
	if item.Text != "" {
		buf = appendStringField(buf, 2, item.Text)
	}
	// Field 3: is_task (bool)
	if item.IsTask {
		buf = appendVarintField(buf, 3, 1)
	}
	// Field 4: status (enum)
	if item.Status != TaskStatus_TASK_STATUS_UNSPECIFIED {
		buf = appendVarintField(buf, 4, uint64(item.Status))
	}
	// Field 5: folded (bool)
	if item.Folded {
		buf = appendVarintField(buf, 5, 1)
	}
	// Field 6: tags (repeated string)
	for _, tag := range item.Tags {
		buf = appendStringField(buf, 6, tag)
	}
	// Field 7: children (repeated ItemProto)
	for _, child := range item.Children {
		childBytes, err := MarshalItemProto(child)
		if err != nil {
			return nil, err
		}
		buf = appendBytesField(buf, 7, childBytes)
	}
	// Field 8: created_at (int64)
	if item.CreatedAt != 0 {
		buf = appendVarintField(buf, 8, uint64(item.CreatedAt))
	}
	// Field 9: updated_at (int64)
	if item.UpdatedAt != 0 {
		buf = appendVarintField(buf, 9, uint64(item.UpdatedAt))
	}
	// Field 10: node_id (string)
	if item.NodeId != "" {
		buf = appendStringField(buf, 10, item.NodeId)
	}
	// Field 11: deleted (bool)
	if item.Deleted {
		buf = appendVarintField(buf, 11, 1)
	}
	// Field 12: version (uint64)
	if item.Version != 0 {
		buf = appendVarintField(buf, 12, item.Version)
	}

	return buf, nil
}

func UnmarshalTreeProto(data []byte) (*TreeProto, error) {
	t := &TreeProto{}
	offset := 0

	for offset < len(data) {
		fieldNum, wireType, n, err := readKey(data[offset:])
		if err != nil {
			return nil, err
		}
		offset += n

		switch fieldNum {
		case 1: // schema_version
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			t.SchemaVersion = uint32(val)
		case 2: // roots
			itemData, n, err := readBytes(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item, err := UnmarshalItemProto(itemData)
			if err != nil {
				return nil, err
			}
			t.Roots = append(t.Roots, item)
		case 3: // last_modified
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			t.LastModified = int64(val)
		default:
			n, err := skipField(data[offset:], wireType)
			if err != nil {
				return nil, err
			}
			offset += n
		}
	}

	return t, nil
}

func UnmarshalItemProto(data []byte) (*ItemProto, error) {
	item := &ItemProto{}
	offset := 0

	for offset < len(data) {
		fieldNum, wireType, n, err := readKey(data[offset:])
		if err != nil {
			return nil, err
		}
		offset += n

		switch fieldNum {
		case 1: // id
			strBytes, n, err := readBytes(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.Id = string(strBytes)
		case 2: // text
			strBytes, n, err := readBytes(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.Text = string(strBytes)
		case 3: // is_task
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.IsTask = (val != 0)
		case 4: // status
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.Status = TaskStatus(val)
		case 5: // folded
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.Folded = (val != 0)
		case 6: // tags
			tagBytes, n, err := readBytes(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.Tags = append(item.Tags, string(tagBytes))
		case 7: // children
			childBytes, n, err := readBytes(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			child, err := UnmarshalItemProto(childBytes)
			if err != nil {
				return nil, err
			}
			item.Children = append(item.Children, child)
		case 8: // created_at
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.CreatedAt = int64(val)
		case 9: // updated_at
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.UpdatedAt = int64(val)
		case 10: // node_id
			strBytes, n, err := readBytes(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.NodeId = string(strBytes)
		case 11: // deleted
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.Deleted = (val != 0)
		case 12: // version
			val, n, err := readVarint(data[offset:])
			if err != nil {
				return nil, err
			}
			offset += n
			item.Version = val
		default:
			n, err := skipField(data[offset:], wireType)
			if err != nil {
				return nil, err
			}
			offset += n
		}
	}

	return item, nil
}

// Protobuf wire encoding helper functions
func appendVarintField(buf []byte, fieldNum int, val uint64) []byte {
	key := (uint64(fieldNum) << 3) | 0
	buf = binary.AppendUvarint(buf, key)
	buf = binary.AppendUvarint(buf, val)
	return buf
}

func appendStringField(buf []byte, fieldNum int, val string) []byte {
	return appendBytesField(buf, fieldNum, []byte(val))
}

func appendBytesField(buf []byte, fieldNum int, val []byte) []byte {
	key := (uint64(fieldNum) << 3) | 2
	buf = binary.AppendUvarint(buf, key)
	buf = binary.AppendUvarint(buf, uint64(len(val)))
	buf = append(buf, val...)
	return buf
}

func readKey(buf []byte) (fieldNum int, wireType int, n int, err error) {
	key, n, err := readVarint(buf)
	if err != nil {
		return 0, 0, 0, err
	}
	return int(key >> 3), int(key & 0x07), n, nil
}

func readVarint(buf []byte) (uint64, int, error) {
	val, n := binary.Uvarint(buf)
	if n <= 0 {
		return 0, 0, io.ErrUnexpectedEOF
	}
	return val, n, nil
}

func readBytes(buf []byte) ([]byte, int, error) {
	length, n, err := readVarint(buf)
	if err != nil {
		return nil, 0, err
	}
	total := n + int(length)
	if len(buf) < total {
		return nil, 0, io.ErrUnexpectedEOF
	}
	return buf[n:total], total, nil
}

func skipField(buf []byte, wireType int) (int, error) {
	switch wireType {
	case 0: // Varint
		_, n, err := readVarint(buf)
		return n, err
	case 2: // Length-delimited
		_, n, err := readBytes(buf)
		return n, err
	case 1: // 64-bit
		if len(buf) < 8 {
			return 0, io.ErrUnexpectedEOF
		}
		return 8, nil
	case 5: // 32-bit
		if len(buf) < 4 {
			return 0, io.ErrUnexpectedEOF
		}
		return 4, nil
	default:
		return 0, fmt.Errorf("unsupported wire type: %d", wireType)
	}
}
