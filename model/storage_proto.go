package model

import (
	"fmt"
	"time"

	storagepb "github.com/kenth/halptask/proto/v1"
)

const CurrentSchemaVersion uint32 = 1

func SerializeProtobuf(tree *Tree) ([]byte, error) {
	if tree == nil {
		tree = NewTree()
	}

	treeProto := &storagepb.TreeProto{
		SchemaVersion: CurrentSchemaVersion,
		LastModified:  time.Now().UnixNano(),
	}

	for _, root := range tree.Roots {
		if rootProto := root.ToProto(); rootProto != nil {
			treeProto.Roots = append(treeProto.Roots, rootProto)
		}
	}

	data, err := storagepb.MarshalTreeProto(treeProto)
	if err != nil {
		return nil, fmt.Errorf("protobuf marshal error: %w", err)
	}

	return data, nil
}

func ParseProtobuf(data []byte) (*Tree, error) {
	treeProto, err := storagepb.UnmarshalTreeProto(data)
	if err != nil {
		return nil, fmt.Errorf("protobuf unmarshal error: %w", err)
	}

	tree := NewTree()
	for _, rootProto := range treeProto.Roots {
		if root := ItemFromProto(rootProto); root != nil {
			tree.Roots = append(tree.Roots, root)
		}
	}

	tree.EnsureIDs()
	tree.SetParents()
	return tree, nil
}
