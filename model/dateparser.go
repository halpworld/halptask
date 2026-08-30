package model

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
	"time"
)

var ErrInvalidDateFormat = errors.New("invalid date format")

// NormalizeDueInput cleans raw due tokens such as 'due:tomorrow', '@due(2026-08-25)', etc.
func NormalizeDueInput(raw string) string {
	s := strings.TrimSpace(raw)
	s = strings.TrimPrefix(s, "@due(")
	if strings.HasSuffix(s, ")") && strings.HasPrefix(raw, "@due(") {
		s = strings.TrimSuffix(s, ")")
	}
	s = strings.TrimPrefix(s, "due:")
	s = strings.TrimPrefix(s, "due=")
	s = strings.Trim(s, "\"'`")
	return strings.TrimSpace(s)
}

// ParseDueString parses natural language date strings into a normalized time.Time (end of day 23:59:59 local time).
func ParseDueString(input string, now time.Time) (time.Time, string, error) {
	val := NormalizeDueInput(input)
	if val == "" {
		return time.Time{}, "", ErrInvalidDateFormat
	}

	loc := now.Location()
	todayEnd := time.Date(now.Year(), now.Month(), now.Day(), 23, 59, 59, 0, loc)
	lower := strings.ToLower(val)

	switch lower {
	case "today", "tdy":
		return todayEnd, val, nil
	case "tomorrow", "tmrw":
		return todayEnd.AddDate(0, 0, 1), val, nil
	case "yesterday", "ydy":
		return todayEnd.AddDate(0, 0, -1), val, nil
	}

	// Weekday names
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

	if targetWd, exists := weekdays[lower]; exists {
		days := (int(targetWd) - int(now.Weekday()) + 7) % 7
		if days == 0 {
			days = 7 // Target next week's occurrence
		}
		return todayEnd.AddDate(0, 0, days), val, nil
	}

	// Relative offsets: e.g. "+1d", "3d", "+2w", "1w", "+1m", "2m"
	if strings.HasSuffix(lower, "d") {
		numStr := strings.TrimSuffix(strings.TrimPrefix(lower, "+"), "d")
		if days, err := strconv.Atoi(numStr); err == nil && days != 0 {
			return todayEnd.AddDate(0, 0, days), val, nil
		}
	}
	if strings.HasSuffix(lower, "w") {
		numStr := strings.TrimSuffix(strings.TrimPrefix(lower, "+"), "w")
		if weeks, err := strconv.Atoi(numStr); err == nil && weeks != 0 {
			return todayEnd.AddDate(0, 0, weeks*7), val, nil
		}
	}
	if strings.HasSuffix(lower, "m") {
		numStr := strings.TrimSuffix(strings.TrimPrefix(lower, "+"), "m")
		if months, err := strconv.Atoi(numStr); err == nil && months != 0 {
			return todayEnd.AddDate(0, months, 0), val, nil
		}
	}

	// Standard date layouts
	layouts := []string{
		"2006-01-02",
		"2006/01/02",
		"2006-1-2",
		"2006/1/2",
		"01-02",
		"01/02",
		"1-2",
		"1/2",
		"Jan 02",
		"Jan 2",
		"January 02",
		"January 2",
		"02 Jan",
		"2 Jan",
		"02 January",
		"2 January",
	}

	for _, layout := range layouts {
		if t, err := time.ParseInLocation(layout, val, loc); err == nil {
			year := now.Year()
			if strings.Contains(layout, "2006") {
				year = t.Year()
			}
			target := time.Date(year, t.Month(), t.Day(), 23, 59, 59, 0, loc)
			return target, val, nil
		}
	}

	return time.Time{}, "", fmt.Errorf("%w: %q", ErrInvalidDateFormat, input)
}

// ExtractDueToken scans text for inline 'due:xxx' or '@due(xxx)' and strips it out cleanly.
func ExtractDueToken(text string, now time.Time) (cleanText string, dueDate *time.Time, rawDue string, found bool) {
	// 1. Check for @due(...) format
	if idx := strings.Index(text, "@due("); idx != -1 {
		endIdx := strings.Index(text[idx:], ")")
		if endIdx != -1 {
			token := text[idx : idx+endIdx+1]
			inside := token[5 : len(token)-1]
			if parsed, raw, err := ParseDueString(inside, now); err == nil {
				cleaned := strings.TrimSpace(text[:idx] + text[idx+endIdx+1:])
				return cleaned, &parsed, raw, true
			}
		}
	}

	// 2. Check for due:xxx or due=xxx in words
	words := strings.Fields(text)
	var remainingWords []string

	for _, w := range words {
		lowerW := strings.ToLower(w)
		if (strings.HasPrefix(lowerW, "due:") || strings.HasPrefix(lowerW, "due=")) && len(w) > 4 {
			dueVal := strings.Trim(w[4:], "\"'`")
			if parsed, raw, err := ParseDueString(dueVal, now); err == nil && !found {
				dueDate = &parsed
				rawDue = raw
				found = true
				continue
			}
		}
		remainingWords = append(remainingWords, w)
	}

	if found {
		cleanText = strings.Join(remainingWords, " ")
		return cleanText, dueDate, rawDue, true
	}

	return text, nil, "", false
}

// FormatDueBadge generates user-friendly badge text and category ("overdue", "today", "upcoming", "done").
func FormatDueBadge(dueDateUnix int64, isDone bool, now time.Time) (badgeText string, badgeCategory string) {
	if dueDateUnix <= 0 {
		return "", ""
	}

	dueDate := time.Unix(dueDateUnix, 0).In(now.Location())
	todayStart := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	dueDayStart := time.Date(dueDate.Year(), dueDate.Month(), dueDate.Day(), 0, 0, 0, 0, dueDate.Location())

	daysDiff := int(dueDayStart.Sub(todayStart).Hours() / 24)

	if isDone {
		if daysDiff == 0 {
			return "[Due: Today]", "done"
		}
		return fmt.Sprintf("[Due: %s]", dueDate.Format("Jan 02")), "done"
	}

	if daysDiff < 0 {
		daysOverdue := -daysDiff
		if daysOverdue == 1 {
			return "[Overdue: 1d]", "overdue"
		}
		return fmt.Sprintf("[Overdue: %dd]", daysOverdue), "overdue"
	}

	if daysDiff == 0 {
		return "[Due Today]", "today"
	}

	if daysDiff == 1 {
		return "[Due: Tomorrow]", "upcoming"
	}

	if daysDiff < 7 {
		return fmt.Sprintf("[Due: %s]", dueDate.Format("Mon")), "upcoming"
	}

	return fmt.Sprintf("[Due: %s]", dueDate.Format("Jan 02")), "upcoming"
}
