package model

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/halpworld/halptask/config"
	"golang.org/x/term"
)

// QuickCaptureOptions configures headless task capture.
type QuickCaptureOptions struct {
	FilePath   string
	Encrypt    bool
	Passphrase string
	RawText    string
	Tags       []string
	IsBullet   bool
	IsTask     bool
	PrependTop bool
	InboxName  string
}

// ParseCaptureText extracts status prefixes, inline #tags, inline due:<date>, and clean title.
func ParseCaptureText(rawText string) (cleanTitle string, tags []string, rawDue string, isTask bool, status TaskStatus) {
	text := strings.TrimSpace(rawText)
	isTask = true
	status = StatusTodo

	// Check task status or bullet prefixes
	if strings.HasPrefix(text, "- [ ] ") || strings.HasPrefix(text, "* [ ] ") {
		text = strings.TrimSpace(text[6:])
		isTask = true
		status = StatusTodo
	} else if strings.HasPrefix(text, "- [~] ") || strings.HasPrefix(text, "* [~] ") {
		text = strings.TrimSpace(text[6:])
		isTask = true
		status = StatusInProgress
	} else if strings.HasPrefix(text, "- [x] ") || strings.HasPrefix(text, "- [X] ") || strings.HasPrefix(text, "* [x] ") || strings.HasPrefix(text, "* [X] ") {
		text = strings.TrimSpace(text[6:])
		isTask = true
		status = StatusDone
	} else if strings.HasPrefix(text, "- ") || strings.HasPrefix(text, "* ") {
		text = strings.TrimSpace(text[2:])
	} else if strings.HasPrefix(text, "• ") {
		text = strings.TrimSpace(text[len("• "):])
	}

	words := strings.Fields(text)
	var titleWords []string
	tagSet := make(map[string]bool)

	for _, w := range words {
		lowerW := strings.ToLower(w)
		if strings.HasPrefix(lowerW, "@due(") && strings.HasSuffix(w, ")") {
			rawDue = w[5 : len(w)-1]
			continue
		}
		// Check for due: prefix
		if (strings.HasPrefix(lowerW, "due:") || strings.HasPrefix(lowerW, "due=")) && len(w) > 4 {
			dueVal := strings.Trim(w[4:], "\"'")
			if dueVal != "" {
				rawDue = dueVal
				continue
			}
		}

		// Check for #tag
		if strings.HasPrefix(w, "#") && len(w) > 1 {
			rawTag := w[1:]
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

		titleWords = append(titleWords, w)
	}

	cleanTitle = strings.Join(titleWords, " ")
	return cleanTitle, tags, rawDue, isTask, status
}

// FindOrCreateInbox locates an existing root item representing Inbox, or creates one if none exists.
func FindOrCreateInbox(tree *Tree, inboxName string) *Item {
	if inboxName == "" {
		inboxName = "Inbox"
	}
	cleanInboxName := strings.ToLower(strings.TrimPrefix(inboxName, "#"))

	for _, root := range tree.Roots {
		rootClean := strings.TrimPrefix(root.Text, "#")
		if strings.EqualFold(rootClean, cleanInboxName) || strings.EqualFold(root.Text, inboxName) || root.HasDirectTag(cleanInboxName) {
			return root
		}
	}

	// Not found, create new root item for Inbox
	inboxItem := NewItem(tree.NextID(), inboxName)
	tree.Roots = append(tree.Roots, inboxItem)
	tree.SetParents()
	return inboxItem
}

// ResolvePassphrase retrieves encryption passphrase from options, environment, or terminal prompt.
func ResolvePassphrase(filePath string, forceEncrypt bool, in io.Reader, out io.Writer) (string, error) {
	isEnc, err := IsEncryptedFile(filePath)
	if err != nil && !os.IsNotExist(err) {
		return "", err
	}
	if !isEnc && !forceEncrypt {
		return "", nil
	}

	// 1. Environment variable
	if envPass := os.Getenv("HALPTASK_PASSPHRASE"); envPass != "" {
		return envPass, nil
	}

	// 2. Interactive terminal prompt
	if f, ok := in.(*os.File); ok && term.IsTerminal(int(f.Fd())) {
		if out != nil {
			fmt.Fprintf(out, "🔑 Enter passphrase for %s: ", filepath.Base(filePath))
		}
		passBytes, err := term.ReadPassword(int(f.Fd()))
		if out != nil {
			fmt.Fprintln(out)
		}
		if err != nil {
			return "", fmt.Errorf("failed to read passphrase: %w", err)
		}
		return string(passBytes), nil
	}

	return "", errors.New("passphrase required for encrypted file (set HALPTASK_PASSPHRASE or run in interactive terminal)")
}

// RunQuickCapture parses user input, loads the storage tree, inserts the new node, and saves.
func RunQuickCapture(cfg *config.Config, opts QuickCaptureOptions) (*Item, string, error) {
	if opts.IsTask && opts.IsBullet {
		return nil, "", errors.New("--task and --bullet cannot be combined")
	}
	if strings.TrimSpace(opts.RawText) == "" {
		return nil, "", errors.New("task text cannot be empty")
	}

	storage, tree, passphrase, err := loadHeadlessStorage(cfg, opts.FilePath, opts.Encrypt, opts.Passphrase)
	if err != nil {
		return nil, "", err
	}

	cleanTitle, parsedTags, rawDue, parsedIsTask, parsedStatus := ParseCaptureText(opts.RawText)
	if strings.TrimSpace(cleanTitle) == "" {
		return nil, "", errors.New("task title cannot be empty")
	}

	// Combine tags from text and explicit flags
	tagSet := make(map[string]bool)
	var finalTags []string
	for _, t := range parsedTags {
		tLower := strings.ToLower(t)
		if !tagSet[tLower] {
			tagSet[tLower] = true
			finalTags = append(finalTags, tLower)
		}
	}
	for _, t := range opts.Tags {
		t = strings.TrimPrefix(strings.TrimSpace(t), "#")
		tLower := strings.ToLower(t)
		if tLower != "" && !tagSet[tLower] {
			tagSet[tLower] = true
			finalTags = append(finalTags, tLower)
		}
	}

	// Determine task vs bullet
	isTask := parsedIsTask
	if opts.IsBullet {
		isTask = false
	} else if opts.IsTask {
		isTask = true
	} else if cfg != nil && cfg.DefaultItemType == "task" {
		isTask = true
	}

	status := parsedStatus
	if !isTask {
		status = StatusNone
	}

	// Prepare stored item text and due date
	storedText := cleanTitle
	var itemDueDate int64
	var itemDueText string

	if rawDue != "" {
		if parsedDue, normRaw, err := ParseDueString(rawDue, time.Now()); err == nil {
			itemDueDate = parsedDue.Unix()
			itemDueText = normRaw
		} else {
			storedText = fmt.Sprintf("%s due:%s", cleanTitle, rawDue)
		}
	}

	inbox := FindOrCreateInbox(tree, opts.InboxName)

	now := time.Now().UnixNano()
	newItem := &Item{
		ID:        tree.NextID(),
		Text:      storedText,
		IsTask:    isTask,
		Status:    status,
		Folded:    false,
		Tags:      finalTags,
		Children:  []*Item{},
		CreatedAt: now,
		UpdatedAt: now,
		Version:   1,
		DueDate:   itemDueDate,
		DueText:   itemDueText,
	}

	if opts.PrependTop {
		inbox.Children = append([]*Item{newItem}, inbox.Children...)
	} else {
		inbox.Children = append(inbox.Children, newItem)
	}
	inbox.Folded = false

	tree.EnsureIDs()
	tree.SetParents()

	if err := storage.Save(tree, passphrase); err != nil {
		return nil, "", fmt.Errorf("failed to save data file %s: %w", storage.FilePath, err)
	}

	// Build confirmation message
	var msgParts []string
	itemTypeStr := "Added to Inbox"
	if !isTask {
		itemTypeStr = "Added bullet to Inbox"
	}
	if opts.PrependTop {
		itemTypeStr = "Added to top of Inbox"
		if !isTask {
			itemTypeStr = "Added bullet to top of Inbox"
		}
	}

	itemTypeStr = strings.ReplaceAll(itemTypeStr, "Inbox", inbox.Text)
	confirmMsg := fmt.Sprintf("✔ %s: %q", itemTypeStr, storedText)
	for _, t := range finalTags {
		msgParts = append(msgParts, fmt.Sprintf("[#%s]", t))
	}
	if itemDueDate != 0 {
		msgParts = append(msgParts, fmt.Sprintf("[Due: %s]", FormatDueDisplay(rawDue)))
	}
	if len(msgParts) > 0 {
		confirmMsg += " " + strings.Join(msgParts, " ")
	}

	return newItem, confirmMsg, nil
}

// FormatDueDisplay capitalizes standard due keywords for clean user display.
func FormatDueDisplay(rawDue string) string {
	lower := strings.ToLower(rawDue)
	switch lower {
	case "today", "tdy":
		return "Today"
	case "tomorrow", "tmrw":
		return "Tomorrow"
	case "yesterday", "ydy":
		return "Yesterday"
	case "monday", "mon":
		return "Monday"
	case "tuesday", "tue":
		return "Tuesday"
	case "wednesday", "wed":
		return "Wednesday"
	case "thursday", "thu":
		return "Thursday"
	case "friday", "fri":
		return "Friday"
	case "saturday", "sat":
		return "Saturday"
	case "sunday", "sun":
		return "Sunday"
	default:
		return rawDue
	}
}
