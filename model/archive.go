package model

import (
	"bytes"
	"compress/gzip"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type ArchivedEntry struct {
	ID         string    `json:"id"`
	ArchivedAt time.Time `json:"archived_at"`
	Context    string    `json:"context,omitempty"`
	Item       *Item     `json:"item"`
}

type ArchiveStore struct {
	FilePath  string
	Encrypted bool
}

func NewArchiveStore(filePath string, encrypted bool) *ArchiveStore {
	return &ArchiveStore{
		FilePath:  filePath,
		Encrypted: encrypted,
	}
}

func (s *ArchiveStore) Load(passphrase string) ([]*ArchivedEntry, error) {
	if _, err := os.Stat(s.FilePath); os.IsNotExist(err) {
		return []*ArchivedEntry{}, nil
	}

	data, err := os.ReadFile(s.FilePath)
	if err != nil {
		return nil, err
	}

	content := string(data)
	var compressedData []byte

	if strings.HasPrefix(strings.TrimSpace(content), EncryptedHeader) {
		if passphrase == "" {
			return nil, ErrIncorrectPassphrase
		}
		decryptedBase64, err := decryptContent(content, passphrase)
		if err != nil {
			return nil, err
		}
		rawCompressed, err := base64.StdEncoding.DecodeString(decryptedBase64)
		if err != nil {
			return nil, ErrIncorrectPassphrase
		}
		compressedData = rawCompressed
	} else {
		compressedData = data
	}

	var jsonData []byte
	// Check if data is gzip compressed (magic numbers 0x1f, 0x8b)
	if len(compressedData) >= 2 && compressedData[0] == 0x1f && compressedData[1] == 0x8b {
		gzReader, err := gzip.NewReader(bytes.NewReader(compressedData))
		if err != nil {
			return nil, fmt.Errorf("gzip reader error: %w", err)
		}
		defer gzReader.Close()
		decompressed, err := io.ReadAll(gzReader)
		if err != nil {
			return nil, fmt.Errorf("gzip decompress error: %w", err)
		}
		jsonData = decompressed
	} else {
		jsonData = compressedData
	}

	var entries []*ArchivedEntry
	if err := json.Unmarshal(jsonData, &entries); err != nil {
		return nil, fmt.Errorf("archive json unmarshal error: %w", err)
	}

	for _, entry := range entries {
		if entry.Item != nil {
			t := NewTree()
			t.Roots = []*Item{entry.Item}
			t.SetParents()
		}
	}

	return entries, nil
}

func (s *ArchiveStore) Save(entries []*ArchivedEntry, passphrase string) error {
	jsonData, err := json.Marshal(entries)
	if err != nil {
		return fmt.Errorf("archive json marshal error: %w", err)
	}

	var gzBuf bytes.Buffer
	gzWriter := gzip.NewWriter(&gzBuf)
	if _, err := gzWriter.Write(jsonData); err != nil {
		return fmt.Errorf("gzip write error: %w", err)
	}
	if err := gzWriter.Close(); err != nil {
		return fmt.Errorf("gzip close error: %w", err)
	}

	compressedData := gzBuf.Bytes()
	var fileContent []byte

	if s.Encrypted {
		if passphrase == "" {
			return fmt.Errorf("passphrase required for encrypted archive")
		}
		encodedCompressed := base64.StdEncoding.EncodeToString(compressedData)
		encryptedStr, err := encryptContent(encodedCompressed, passphrase)
		if err != nil {
			return fmt.Errorf("archive encryption error: %w", err)
		}
		fileContent = []byte(encryptedStr)
	} else {
		fileContent = compressedData
	}

	dir := filepath.Dir(s.FilePath)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}

	perm := os.FileMode(0600)
	if stat, err := os.Stat(s.FilePath); err == nil {
		perm = stat.Mode().Perm()
	}

	return os.WriteFile(s.FilePath, fileContent, perm)
}

func (s *ArchiveStore) Reencrypt(oldPassphrase, newPassphrase string) error {
	entries, err := s.Load(oldPassphrase)
	if err != nil {
		return err
	}
	s.Encrypted = (newPassphrase != "")
	return s.Save(entries, newPassphrase)
}
