package model

import (
	"bufio"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"golang.org/x/crypto/pbkdf2"
)

const EncryptedHeader = "# HALPTASK-ENCRYPTED-v1"
const PBKDF2Iterations = 100000

var (
	ErrIncorrectPassphrase = errors.New("incorrect passphrase or corrupted data")
	ErrNotEncrypted        = errors.New("file is not encrypted")
)

type Storage struct {
	FilePath  string
	Encrypted bool
}

func NewStorage(filePath string, encrypted bool) *Storage {
	return &Storage{
		FilePath:  filePath,
		Encrypted: encrypted,
	}
}

func IsEncryptedFile(filePath string) (bool, error) {
	file, err := os.Open(filePath)
	if err != nil {
		if os.IsNotExist(err) {
			return false, nil
		}
		return false, err
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	if scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		return strings.HasPrefix(line, EncryptedHeader), nil
	}
	return false, scanner.Err()
}

func GetMigratedFilePath(oldPath string) string {
	ext := strings.ToLower(filepath.Ext(oldPath))
	if ext == ".pb" || ext == ".halp" || ext == ".yaml" || ext == ".yml" || ext == ".json" {
		return oldPath
	}
	if ext == ".txt" || ext == ".md" {
		return oldPath[:len(oldPath)-len(ext)] + ".pb"
	}
	return oldPath + ".pb"
}

func isNonDataConfigFile(filePath string) bool {
	lower := strings.ToLower(filePath)
	return strings.HasSuffix(lower, ".yaml") ||
		strings.HasSuffix(lower, ".yml") ||
		strings.HasSuffix(lower, ".json") ||
		strings.Contains(lower, "config.yaml")
}

func IsProtobufData(rawBytes []byte, filePath string) bool {
	ext := strings.ToLower(filepath.Ext(filePath))
	if ext == ".pb" || ext == ".halp" {
		return true
	}
	if len(rawBytes) >= 2 && rawBytes[0] == 0x08 && rawBytes[1] == 0x01 {
		return true
	}
	if tree, err := ParseProtobuf(rawBytes); err == nil && tree != nil {
		return true
	}
	return false
}

func (s *Storage) MigrateIfNeeded(passphrase string) (bool, string, error) {
	if isNonDataConfigFile(s.FilePath) {
		return false, s.FilePath, nil
	}

	if _, err := os.Stat(s.FilePath); os.IsNotExist(err) {
		return false, s.FilePath, nil
	}

	rawBytes, err := os.ReadFile(s.FilePath)
	if err != nil {
		return false, s.FilePath, err
	}

	content := string(rawBytes)
	if strings.HasPrefix(strings.TrimSpace(content), EncryptedHeader) {
		if passphrase == "" {
			return false, s.FilePath, errors.New("passphrase required for encrypted file")
		}
		decrypted, err := decryptContent(content, passphrase)
		if err != nil {
			return false, s.FilePath, err
		}
		rawBytes = []byte(decrypted)
		content = decrypted
	}

	// Check if already valid Protobuf
	if IsProtobufData(rawBytes, s.FilePath) {
		if tree, err := ParseProtobuf(rawBytes); err == nil && tree != nil {
			return false, s.FilePath, nil
		}
	} else if tree, err := ParseProtobuf(rawBytes); err == nil && tree != nil && len(tree.Roots) > 0 {
		return false, s.FilePath, nil
	}

	// Legacy Markdown detected -> Migrate to Protobuf with updated .pb extension
	tree := ParseMarkdown(content)
	newPath := GetMigratedFilePath(s.FilePath)

	// Create backup of old file
	bakPath := s.FilePath + ".bak"
	if err := os.WriteFile(bakPath, []byte(content), 0644); err != nil {
		return false, s.FilePath, fmt.Errorf("failed to create migration backup: %w", err)
	}

	// Save migrated binary payload to new .pb file path
	targetStorage := NewStorage(newPath, s.Encrypted)
	if err := targetStorage.Save(tree, passphrase); err != nil {
		return false, s.FilePath, fmt.Errorf("failed to save migrated protobuf file: %w", err)
	}

	oldPath := s.FilePath
	s.FilePath = newPath

	// Remove old legacy file if different path
	if oldPath != newPath {
		_ = os.Remove(oldPath)
	}

	return true, newPath, nil
}

func (s *Storage) Load(passphrase string) (*Tree, error) {
	if _, err := os.Stat(s.FilePath); os.IsNotExist(err) {
		// Return empty default tree with a sample bullet
		tree := NewTree()
		root := NewItem("1", "Welcome to HalpTask! Press ? for help or <space> for leader menu.")
		child1 := NewTask("2", "Try toggling tasks with 't'", StatusTodo)
		child2 := NewTask("3", "Mark task in-progress with 'ts' or leader 't p'", StatusInProgress)
		child3 := NewTask("4", "Mark done with 'ts' or leader 't d'", StatusDone)
		root.Children = append(root.Children, child1, child2, child3)
		tree.Roots = append(tree.Roots, root)
		tree.EnsureIDs()
		tree.SetParents()
		return tree, nil
	}

	// Execute migration if legacy format detected
	migrated, newPath, err := s.MigrateIfNeeded(passphrase)
	if err != nil && !errors.Is(err, ErrIncorrectPassphrase) {
		// If migration fails non-fatally, fall back to direct file read
		_ = migrated
		_ = newPath
	}

	rawBytes, err := os.ReadFile(s.FilePath)
	if err != nil {
		return nil, err
	}

	content := string(rawBytes)
	if strings.HasPrefix(strings.TrimSpace(content), EncryptedHeader) {
		if passphrase == "" {
			return nil, errors.New("passphrase required for encrypted file")
		}
		decrypted, err := decryptContent(content, passphrase)
		if err != nil {
			return nil, err
		}
		rawBytes = []byte(decrypted)
		content = decrypted
	}

	// Try decoding as Protobuf first
	if IsProtobufData(rawBytes, s.FilePath) {
		tree, err := ParseProtobuf(rawBytes)
		if err != nil {
			return nil, fmt.Errorf("failed to parse protobuf storage file: %w", err)
		}
		return tree, nil
	}

	if tree, err := ParseProtobuf(rawBytes); err == nil && tree != nil {
		return tree, nil
	}

	// Fallback to legacy Markdown parsing for non-protobuf files
	return ParseMarkdown(content), nil
}

func (s *Storage) Save(tree *Tree, passphrase string) error {
	payloadBytes, err := SerializeProtobuf(tree)
	if err != nil {
		return fmt.Errorf("failed to serialize protobuf: %w", err)
	}

	var content string
	if s.Encrypted {
		if passphrase == "" {
			return errors.New("passphrase required to encrypt file")
		}
		encrypted, err := encryptContent(string(payloadBytes), passphrase)
		if err != nil {
			return fmt.Errorf("encryption error: %w", err)
		}
		content = encrypted
	} else {
		content = string(payloadBytes)
	}

	dir := filepath.Dir(s.FilePath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	return os.WriteFile(s.FilePath, []byte(content), 0644)
}

func ParseMarkdown(content string) *Tree {
	tree := NewTree()
	scanner := bufio.NewScanner(strings.NewReader(content))

	type stackItem struct {
		item  *Item
		depth int
	}

	var stack []stackItem

	for scanner.Scan() {
		rawLine := scanner.Text()
		trimmedLine := strings.TrimRight(rawLine, "\r\n")
		if strings.TrimSpace(trimmedLine) == "" || strings.HasPrefix(strings.TrimSpace(trimmedLine), "#") {
			continue // Skip blank lines and comments/headers
		}

		// Calculate depth based on leading spaces (2 spaces = 1 depth) or leading tabs
		indent := 0
		for _, r := range trimmedLine {
			if r == ' ' {
				indent++
			} else if r == '\t' {
				indent += 2
			} else {
				break
			}
		}
		depth := indent / 2

		lineText := strings.TrimSpace(trimmedLine)

		// Check id tag <!-- id: XYZ -->
		var explicitID string
		if idx := strings.Index(lineText, "<!-- id:"); idx != -1 {
			endIdx := strings.Index(lineText[idx:], "-->")
			if endIdx != -1 {
				idPart := lineText[idx+8 : idx+endIdx]
				explicitID = strings.TrimSpace(idPart)
				lineText = strings.TrimSpace(lineText[:idx] + lineText[idx+endIdx+3:])
			}
		}

		// Check fold tag
		folded := false
		if strings.Contains(lineText, "<!-- fold -->") || strings.Contains(lineText, "<!-- folded -->") {
			folded = true
			lineText = strings.ReplaceAll(lineText, "<!-- fold -->", "")
			lineText = strings.ReplaceAll(lineText, "<!-- folded -->", "")
			lineText = strings.TrimSpace(lineText)
		}

		// Check focus tag
		isFocused := false
		if strings.Contains(lineText, "<!-- focus -->") || strings.Contains(lineText, "<!-- focused -->") {
			isFocused = true
			lineText = strings.ReplaceAll(lineText, "<!-- focus -->", "")
			lineText = strings.ReplaceAll(lineText, "<!-- focused -->", "")
			lineText = strings.TrimSpace(lineText)
		}

		// Strip leading bullet markers ('-', '*', '•')
		var item *Item
		id := explicitID
		var text string

		if strings.HasPrefix(lineText, "- [ ] ") || strings.HasPrefix(lineText, "* [ ] ") {
			text = strings.TrimPrefix(strings.TrimPrefix(lineText, "- [ ] "), "* [ ] ")
			item = NewTask(id, text, StatusTodo)
		} else if strings.HasPrefix(lineText, "- [~] ") || strings.HasPrefix(lineText, "* [~] ") {
			text = strings.TrimPrefix(strings.TrimPrefix(lineText, "- [~] "), "* [~] ")
			item = NewTask(id, text, StatusInProgress)
		} else if strings.HasPrefix(lineText, "- [x] ") || strings.HasPrefix(lineText, "- [X] ") || strings.HasPrefix(lineText, "* [x] ") || strings.HasPrefix(lineText, "* [X] ") {
			text = lineText[6:]
			item = NewTask(id, text, StatusDone)
		} else if strings.HasPrefix(lineText, "- ") || strings.HasPrefix(lineText, "* ") {
			text = lineText[2:]
			item = NewItem(id, text)
		} else if strings.HasPrefix(lineText, "• ") {
			text = lineText[len("• "):]
			item = NewItem(id, text)
		} else {
			text = lineText
			item = NewItem(id, text)
		}

		// Extract #tags from text
		cleanText, tags := parseTagsFromText(text)
		item.Text = cleanText
		item.Tags = tags
		item.Folded = folded
		item.IsFocused = isFocused

		if depth == 0 {
			tree.Roots = append(tree.Roots, item)
			stack = []stackItem{{item: item, depth: 0}}
		} else {
			// Find parent with depth < current depth
			for len(stack) > 0 && stack[len(stack)-1].depth >= depth {
				stack = stack[:len(stack)-1]
			}
			if len(stack) > 0 {
				parent := stack[len(stack)-1].item
				parent.Children = append(parent.Children, item)
				stack = append(stack, stackItem{item: item, depth: depth})
			} else {
				// Fallback to root if indentation doesn't match parent
				tree.Roots = append(tree.Roots, item)
				stack = []stackItem{{item: item, depth: 0}}
			}
		}
	}

	tree.EnsureIDs()
	tree.SetParents()
	return tree
}

func parseTagsFromText(text string) (string, []string) {
	words := strings.Fields(text)
	var cleanWords []string
	var tags []string
	tagSet := make(map[string]bool)

	for _, word := range words {
		if strings.HasPrefix(word, "#") && len(word) > 1 {
			rawTag := word[1:]
			// Tag must start with a letter to avoid parsing #123 issue numbers as tags
			firstChar := rune(rawTag[0])
			if (firstChar >= 'a' && firstChar <= 'z') || (firstChar >= 'A' && firstChar <= 'Z') {
				tagLower := strings.ToLower(rawTag)
				if !tagSet[tagLower] {
					tagSet[tagLower] = true
					tags = append(tags, tagLower)
				}
				continue
			}
		}
		cleanWords = append(cleanWords, word)
	}

	return strings.Join(cleanWords, " "), tags
}

func SerializeMarkdown(tree *Tree) string {
	var builder strings.Builder

	var recurse func(items []*Item, depth int)
	recurse = func(items []*Item, depth int) {
		indent := strings.Repeat("  ", depth)
		for _, item := range items {
			builder.WriteString(indent)
			if item.IsTask {
				switch item.Status {
				case StatusDone:
					builder.WriteString("- [x] ")
				case StatusInProgress:
					builder.WriteString("- [~] ")
				default:
					builder.WriteString("- [ ] ")
				}
			} else {
				builder.WriteString("- ")
			}
			builder.WriteString(item.Text)

			for _, tag := range item.Tags {
				builder.WriteString(" #" + tag)
			}

			if item.Folded {
				builder.WriteString(" <!-- fold -->")
			}
			if item.IsFocused {
				builder.WriteString(" <!-- focus -->")
			}
			if item.ID != "" {
				builder.WriteString(fmt.Sprintf(" <!-- id: %s -->", item.ID))
			}
			builder.WriteString("\n")

			if len(item.Children) > 0 {
				recurse(item.Children, depth+1)
			}
		}
	}

	recurse(tree.Roots, 0)
	return builder.String()
}

func encryptContent(plainText, passphrase string) (string, error) {
	salt := make([]byte, 16)
	if _, err := io.ReadFull(rand.Reader, salt); err != nil {
		return "", err
	}

	key := pbkdf2.Key([]byte(passphrase), salt, PBKDF2Iterations, 32, sha256.New)

	block, err := aes.NewCipher(key)
	if err != nil {
		return "", err
	}

	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return "", err
	}

	nonce := make([]byte, gcm.NonceSize())
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		return "", err
	}

	ciphertext := gcm.Seal(nil, nonce, []byte(plainText), nil)

	// Combine salt + nonce + ciphertext
	payload := append(salt, nonce...)
	payload = append(payload, ciphertext...)

	encoded := base64.StdEncoding.EncodeToString(payload)
	return fmt.Sprintf("%s\n%s", EncryptedHeader, encoded), nil
}

func decryptContent(encryptedText, passphrase string) (string, error) {
	lines := strings.Split(strings.TrimSpace(encryptedText), "\n")
	if len(lines) < 2 {
		return "", ErrIncorrectPassphrase
	}

	encodedPayload := strings.TrimSpace(lines[1])
	payload, err := base64.StdEncoding.DecodeString(encodedPayload)
	if err != nil {
		return "", ErrIncorrectPassphrase
	}

	if len(payload) < 16+12 { // salt(16) + nonce(12) minimum
		return "", ErrIncorrectPassphrase
	}

	salt := payload[:16]
	key := pbkdf2.Key([]byte(passphrase), salt, PBKDF2Iterations, 32, sha256.New)

	block, err := aes.NewCipher(key)
	if err != nil {
		return "", ErrIncorrectPassphrase
	}

	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return "", ErrIncorrectPassphrase
	}

	nonceSize := gcm.NonceSize()
	if len(payload) < 16+nonceSize {
		return "", ErrIncorrectPassphrase
	}

	nonce := payload[16 : 16+nonceSize]
	ciphertext := payload[16+nonceSize:]

	plainTextBytes, err := gcm.Open(nil, nonce, ciphertext, nil)
	if err != nil {
		return "", ErrIncorrectPassphrase
	}

	return string(plainTextBytes), nil
}
