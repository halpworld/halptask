package model

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/halpworld/halptask/config"
)

// loadHeadlessStorage never migrates or seeds files: queries must be read-only,
// and captures must keep using the configured path on subsequent invocations.
func loadHeadlessStorage(cfg *config.Config, path string, encrypt bool, passphrase string) (*Storage, *Tree, string, error) {
	if path == "" {
		path = config.DefaultConfig().DataFile
		if cfg != nil && cfg.DataFile != "" {
			path = cfg.DataFile
		}
	}
	if strings.HasPrefix(path, "~/") {
		home, err := os.UserHomeDir()
		if err != nil {
			return nil, nil, "", err
		}
		path = filepath.Join(home, path[2:])
	}
	raw, err := os.ReadFile(path)
	if err != nil && !os.IsNotExist(err) {
		return nil, nil, "", err
	}
	isEncrypted := strings.HasPrefix(strings.TrimSpace(string(raw)), EncryptedHeader)
	storage := NewStorage(path, encrypt || isEncrypted || (cfg != nil && cfg.Encrypted))
	if passphrase == "" && (isEncrypted || storage.Encrypted) {
		passphrase, err = ResolvePassphrase(path, storage.Encrypted, os.Stdin, os.Stderr)
		if err != nil {
			return nil, nil, "", err
		}
	}
	if len(raw) == 0 {
		return storage, NewTree(), passphrase, nil
	}
	if isEncrypted {
		plain, err := decryptContent(string(raw), passphrase)
		if err != nil {
			return nil, nil, "", err
		}
		raw = []byte(plain)
	}
	var tree *Tree
	if IsProtobufData(raw, path) {
		tree, err = ParseProtobuf(raw)
	} else {
		tree = ParseMarkdown(string(raw))
	}
	if err != nil {
		return nil, nil, "", fmt.Errorf("failed to load data file %s: %w", path, err)
	}
	tree.SetParents()
	return storage, tree, passphrase, nil
}
