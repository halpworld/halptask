package storagepb

import (
	"errors"
	"io"
	"testing"
)

func TestReadBytesIntegerOverflow(t *testing.T) {
	// 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01 is varint for 0x8000000000000000
	// (a uint64 with bit 63 set, which becomes negative when converted to signed int64)
	malformedLen := []byte{0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 0x01, 0x02, 0x03}

	// Make sure readBytes returns io.ErrUnexpectedEOF instead of panicking with slice bounds out of range
	_, _, err := readBytes(malformedLen)
	if err == nil {
		t.Fatalf("expected error for overflow length, got nil")
	}
	if !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatalf("expected io.ErrUnexpectedEOF, got %v", err)
	}
}

func TestUnmarshalCorruptedTreeProto(t *testing.T) {
	corruptedPayloads := [][]byte{
		{0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}, // Malformed varint
		{0x12, 0xFF, 0xFF, 0xFF, 0xFF, 0x01},                       // Length delimited out of bounds
		{0x1A, 0x10},                                                // Truncated length
	}

	for i, payload := range corruptedPayloads {
		_, err := UnmarshalTreeProto(payload)
		if err == nil {
			t.Errorf("test case %d: expected error for corrupted payload, got nil", i)
		}
	}
}
