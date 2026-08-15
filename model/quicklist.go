package model

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"
	"github.com/halpworld/halptask/config"
	"github.com/mattn/go-isatty"
	"golang.org/x/term"
)

// QuickListOptions configures headless task listing and status queries.
type QuickListOptions struct {
	FilePath   string
	Encrypt    bool
	Passphrase string
	All        bool
	Today      bool
	InProgress bool
	CountOnly  bool
	JSONOutput bool
	NoColor    bool
}

// TaskItemJSON represents a structured JSON representation of an item.
type TaskItemJSON struct {
	ID         string     `json:"id"`
	Text       string     `json:"text"`
	Status     TaskStatus `json:"status"`
	IsTask     bool       `json:"is_task"`
	Tags       []string   `json:"tags,omitempty"`
	Due        string     `json:"due,omitempty"`
	DueDate    string     `json:"due_date,omitempty"`
	DueStatus  string     `json:"due_status,omitempty"` // "overdue", "today", "upcoming", "none"
	ParentPath string     `json:"parent_path,omitempty"`
	IsFocused  bool       `json:"is_focused,omitempty"`
	CreatedAt  int64      `json:"created_at,omitempty"`
	UpdatedAt  int64      `json:"updated_at,omitempty"`
}

// ParseDueDate parses natural date strings into a normalized time.Time (midnight local time).
func ParseDueDate(dateStr string, now time.Time) (time.Time, bool) {
	val := strings.ToLower(strings.TrimSpace(dateStr))
	val = strings.TrimPrefix(val, "due:")
	val = strings.TrimPrefix(val, "due=")
	val = strings.Trim(val, "\"'")

	if val == "" {
		return time.Time{}, false
	}

	today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())

	switch val {
	case "today", "tdy":
		return today, true
	case "tomorrow", "tmrw":
		return today.AddDate(0, 0, 1), true
	case "yesterday", "ydy":
		return today.AddDate(0, 0, -1), true
	}

	// Weekdays
	weekdays := map[string]time.Weekday{
		"sunday":    time.Sunday,
		"sun":       time.Sunday,
		"monday":    time.Monday,
		"mon":       time.Monday,
		"tuesday":   time.Tuesday,
		"tue":       time.Tuesday,
		"wednesday": time.Wednesday,
		"wed":       time.Wednesday,
		"thursday":  time.Thursday,
		"thu":       time.Thursday,
		"friday":    time.Friday,
		"fri":       time.Friday,
		"saturday":  time.Saturday,
		"sat":       time.Saturday,
	}

	if targetWd, exists := weekdays[val]; exists {
		days := (int(targetWd) - int(now.Weekday()) + 7) % 7
		if days == 0 {
			days = 7 // next week's occurrence if requested today
		}
		return today.AddDate(0, 0, days), true
	}

	// Relative offsets: +1d, 1d, +2w, 1w
	if strings.HasSuffix(val, "d") {
		numStr := strings.TrimSuffix(strings.TrimPrefix(val, "+"), "d")
		if days, err := strconv.Atoi(numStr); err == nil {
			return today.AddDate(0, 0, days), true
		}
	}
	if strings.HasSuffix(val, "w") {
		numStr := strings.TrimSuffix(strings.TrimPrefix(val, "+"), "w")
		if weeks, err := strconv.Atoi(numStr); err == nil {
			return today.AddDate(0, 0, weeks*7), true
		}
	}

	// Standard date formats
	layouts := []string{
		"2006-01-02",
		"2006/01/02",
		"2006-1-2",
		"2006/1/2",
		"01-02",
		"01/02",
		"1-2",
		"1/2",
	}

	for _, layout := range layouts {
		if t, err := time.ParseInLocation(layout, val, now.Location()); err == nil {
			if strings.Count(layout, "2006") == 0 {
				// Year was omitted, apply current year
				t = time.Date(now.Year(), t.Month(), t.Day(), 0, 0, 0, 0, now.Location())
			} else {
				t = time.Date(t.Year(), t.Month(), t.Day(), 0, 0, 0, 0, now.Location())
			}
			return t, true
		}
	}

	return time.Time{}, false
}

// ExtractDueDate extracts due date keywords from text and returns clean title and parsed date.
func ExtractDueDate(text string) (cleanTitle string, rawDue string, dueDate time.Time, hasDue bool) {
	words := strings.Fields(text)
	var titleWords []string
	now := time.Now()

	for _, w := range words {
		lowerW := strings.ToLower(w)
		if (strings.HasPrefix(lowerW, "due:") || strings.HasPrefix(lowerW, "due=")) && len(w) > 4 {
			dueVal := strings.Trim(w[4:], "\"'")
			if t, ok := ParseDueDate(dueVal, now); ok {
				rawDue = dueVal
				dueDate = t
				hasDue = true
				continue
			}
		}
		titleWords = append(titleWords, w)
	}

	cleanTitle = strings.Join(titleWords, " ")
	return cleanTitle, rawDue, dueDate, hasDue
}

// EvaluatedTask holds an item with parsed metadata and context.
type EvaluatedTask struct {
	Item       *Item
	CleanTitle string
	RawDue     string
	DueDate    time.Time
	HasDue     bool
	DueStatus  string // "overdue", "today", "upcoming", "none"
	ParentPath string
	AllTags    []string
}

// EvaluateTreeTasks extracts all task items from a tree and enriches them with context and due status.
func EvaluateTreeTasks(tree *Tree, now time.Time) []EvaluatedTask {
	tree.SetParents()
	todayMidnight := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())

	var evaluated []EvaluatedTask
	var recurse func(items []*Item)
	recurse = func(items []*Item) {
		for _, item := range items {
			if item.IsTask {
				cleanTitle, rawDue, dueDate, hasDue := ExtractDueDate(item.Text)
				dueStatus := "none"
				if hasDue {
					if dueDate.Before(todayMidnight) {
						if item.Status != StatusDone {
							dueStatus = "overdue"
						} else {
							dueStatus = "upcoming"
						}
					} else if dueDate.Equal(todayMidnight) {
						dueStatus = "today"
					} else {
						dueStatus = "upcoming"
					}
				}

				allTags := tree.GetAllTags(item)
				parentPath := tree.GetParentPath(item)

				evaluated = append(evaluated, EvaluatedTask{
					Item:       item,
					CleanTitle: cleanTitle,
					RawDue:     rawDue,
					DueDate:    dueDate,
					HasDue:     hasDue,
					DueStatus:  dueStatus,
					ParentPath: parentPath,
					AllTags:    allTags,
				})
			}
			if len(item.Children) > 0 {
				recurse(item.Children)
			}
		}
	}
	recurse(tree.Roots)
	return evaluated
}

// RunQuickList outputs formatted task listings or status tallies to the provided writer.
func RunQuickList(cfg *config.Config, opts QuickListOptions, out io.Writer) error {
	targetFilePath := opts.FilePath
	if targetFilePath == "" {
		if cfg != nil && cfg.DataFile != "" {
			targetFilePath = cfg.DataFile
		} else {
			targetFilePath = config.DefaultConfig().DataFile
		}
	}

	isEncrypted := opts.Encrypt || (cfg != nil && cfg.Encrypted)
	passphrase := opts.Passphrase
	if passphrase == "" {
		var err error
		passphrase, err = ResolvePassphrase(targetFilePath, isEncrypted, os.Stdin, os.Stderr)
		if err != nil {
			return err
		}
	}

	storage := NewStorage(targetFilePath, isEncrypted)
	tree, err := storage.Load(passphrase)
	if err != nil {
		return fmt.Errorf("failed to load data file %s: %w", targetFilePath, err)
	}

	now := time.Now()
	allTasks := EvaluateTreeTasks(tree, now)

	// Filter tasks based on flags
	var filtered []EvaluatedTask
	var todoCount, inProgressCount, overdueCount, todayCount, doneCount int

	for _, task := range allTasks {
		switch task.Item.Status {
		case StatusTodo:
			todoCount++
		case StatusInProgress:
			inProgressCount++
		case StatusDone:
			doneCount++
		}

		if task.DueStatus == "overdue" && task.Item.Status != StatusDone {
			overdueCount++
		} else if task.DueStatus == "today" && task.Item.Status != StatusDone {
			todayCount++
		}

		// Apply filtering predicate
		if opts.InProgress {
			if task.Item.Status == StatusInProgress {
				filtered = append(filtered, task)
			}
		} else if opts.Today {
			if task.Item.Status == StatusInProgress || task.DueStatus == "today" || task.DueStatus == "overdue" {
				filtered = append(filtered, task)
			}
		} else {
			// Default / --all: include all active tasks (or all tasks)
			if opts.All || task.Item.Status != StatusDone {
				filtered = append(filtered, task)
			}
		}
	}

	// 1. Single-line Count Output
	if opts.CountOnly {
		var countStr string
		if overdueCount > 0 {
			countStr = fmt.Sprintf("📋 %d todo, %d in-progress, %d overdue", todoCount, inProgressCount, overdueCount)
		} else if inProgressCount > 0 {
			countStr = fmt.Sprintf("📋 %d todo, %d in-progress", todoCount, inProgressCount)
		} else {
			countStr = fmt.Sprintf("📋 %d todo", todoCount)
		}
		fmt.Fprintln(out, countStr)
		return nil
	}

	// 2. JSON Output
	if opts.JSONOutput {
		jsonList := make([]TaskItemJSON, 0, len(filtered))
		for _, task := range filtered {
			var dueDateStr string
			if task.HasDue {
				dueDateStr = task.DueDate.Format("2006-01-02")
			}
			jsonList = append(jsonList, TaskItemJSON{
				ID:         task.Item.ID,
				Text:       task.CleanTitle,
				Status:     task.Item.Status,
				IsTask:     task.Item.IsTask,
				Tags:       task.AllTags,
				Due:        task.RawDue,
				DueDate:    dueDateStr,
				DueStatus:  task.DueStatus,
				ParentPath: task.ParentPath,
				IsFocused:  task.Item.IsFocused,
				CreatedAt:  task.Item.CreatedAt,
				UpdatedAt:  task.Item.UpdatedAt,
			})
		}
		data, err := json.MarshalIndent(jsonList, "", "  ")
		if err != nil {
			return fmt.Errorf("failed to encode JSON: %w", err)
		}
		fmt.Fprintln(out, string(data))
		return nil
	}

	// 3. Formatted Terminal Listing
	isTTY := false
	if f, ok := out.(*os.File); ok {
		isTTY = isatty.IsTerminal(f.Fd()) || term.IsTerminal(int(f.Fd()))
	}
	useColor := isTTY && !opts.NoColor

	renderListing(out, filtered, targetFilePath, opts, useColor, todoCount, inProgressCount, overdueCount)
	return nil
}

func renderListing(out io.Writer, tasks []EvaluatedTask, filePath string, opts QuickListOptions, useColor bool, todoCount, inProgressCount, overdueCount int) {
	headerStyle := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("86"))
	dimStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	todoStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("252"))
	inProgStyle := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("214"))
	doneStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("244"))
	overdueStyle := lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("203"))
	tagStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("141"))
	dueStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("220"))

	title := "HalpTask"
	if opts.Today {
		title = "HalpTask • Today's Focus"
	} else if opts.InProgress {
		title = "HalpTask • In-Progress Tasks"
	}

	if useColor {
		fmt.Fprintf(out, "%s %s (%d tasks)\n\n", headerStyle.Render(title), dimStyle.Render("• "+filePath), len(tasks))
	} else {
		fmt.Fprintf(out, "%s • %s (%d tasks)\n\n", title, filePath, len(tasks))
	}

	if len(tasks) == 0 {
		if useColor {
			fmt.Fprintf(out, "  %s\n\n", dimStyle.Render("No tasks matching filter."))
		} else {
			fmt.Fprintln(out, "  No tasks matching filter.")
		}
		return
	}

	for _, task := range tasks {
		var statusMarker string
		switch task.Item.Status {
		case StatusDone:
			if useColor {
				statusMarker = doneStyle.Render("[x]")
			} else {
				statusMarker = "[x]"
			}
		case StatusInProgress:
			if useColor {
				statusMarker = inProgStyle.Render("[~]")
			} else {
				statusMarker = "[~]"
			}
		default:
			if task.DueStatus == "overdue" {
				if useColor {
					statusMarker = overdueStyle.Render("[!]")
				} else {
					statusMarker = "[!]"
				}
			} else {
				if useColor {
					statusMarker = todoStyle.Render("[ ]")
				} else {
					statusMarker = "[ ]"
				}
			}
		}

		idStr := fmt.Sprintf("#%s", task.Item.ID)
		if useColor {
			idStr = dimStyle.Render(idStr)
		}

		textStr := task.CleanTitle
		if task.Item.Status == StatusDone && useColor {
			textStr = doneStyle.Render(textStr)
		}

		var tagParts []string
		for _, t := range task.AllTags {
			if useColor {
				tagParts = append(tagParts, tagStyle.Render("#"+t))
			} else {
				tagParts = append(tagParts, "#"+t)
			}
		}

		contextStr := ""
		if task.ParentPath != "" {
			if useColor {
				contextStr = dimStyle.Render("(" + task.ParentPath + ")")
			} else {
				contextStr = "(" + task.ParentPath + ")"
			}
		}

		dueBadge := ""
		if task.HasDue {
			if task.DueStatus == "overdue" {
				badgeText := fmt.Sprintf("[Overdue: %s]", task.DueDate.Format("2006-01-02"))
				if useColor {
					dueBadge = overdueStyle.Render(badgeText)
				} else {
					dueBadge = badgeText
				}
			} else {
				badgeText := fmt.Sprintf("[Due: %s]", FormatDueDisplay(task.RawDue))
				if useColor {
					dueBadge = dueStyle.Render(badgeText)
				} else {
					dueBadge = badgeText
				}
			}
		}

		line := fmt.Sprintf("  %s %-4s %s", statusMarker, idStr, textStr)
		if len(tagParts) > 0 {
			line += " " + strings.Join(tagParts, " ")
		}
		if contextStr != "" {
			line += " " + contextStr
		}
		if dueBadge != "" {
			line += " " + dueBadge
		}

		fmt.Fprintln(out, line)
	}

	// Footer summary
	fmt.Fprintln(out)
	var summaryParts []string
	if todoCount > 0 {
		summaryParts = append(summaryParts, fmt.Sprintf("%d todo", todoCount))
	}
	if inProgressCount > 0 {
		summaryParts = append(summaryParts, fmt.Sprintf("%d in-progress", inProgressCount))
	}
	if overdueCount > 0 {
		summaryParts = append(summaryParts, fmt.Sprintf("%d overdue", overdueCount))
	}
	if len(summaryParts) == 0 {
		summaryParts = append(summaryParts, "All tasks completed! 🎉")
	}

	summaryLine := strings.Join(summaryParts, ", ")
	if useColor {
		fmt.Fprintln(out, dimStyle.Render(summaryLine))
	} else {
		fmt.Fprintln(out, summaryLine)
	}
}
