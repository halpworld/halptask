package model

import (
	"testing"
	"time"
)

func TestParseDueString(t *testing.T) {
	// Fixed reference time: Monday, Aug 25, 2026 10:00:00 UTC
	refTime := time.Date(2026, 8, 25, 10, 0, 0, 0, time.UTC)

	tests := []struct {
		input       string
		expectedDay int
		expectedMon time.Month
		expectedYr  int
		expectErr   bool
	}{
		{"today", 25, time.August, 2026, false},
		{"tdy", 25, time.August, 2026, false},
		{"tomorrow", 26, time.August, 2026, false},
		{"tmrw", 26, time.August, 2026, false},
		{"yesterday", 24, time.August, 2026, false},
		{"due:today", 25, time.August, 2026, false},
		{"@due(tomorrow)", 26, time.August, 2026, false},
		{"+1d", 26, time.August, 2026, false},
		{"+3d", 28, time.August, 2026, false},
		{"+1w", 1, time.September, 2026, false},
		{"2026-09-15", 15, time.September, 2026, false},
		{"09/15", 15, time.September, 2026, false},
		{"friday", 28, time.August, 2026, false},
		{"fri", 28, time.August, 2026, false},
		{"tuesday", 1, time.September, 2026, false}, // next Tuesday since today is Tuesday
		{"invalid-date-xyz", 0, 0, 0, true},
	}

	for _, tc := range tests {
		parsed, _, err := ParseDueString(tc.input, refTime)
		if tc.expectErr {
			if err == nil {
				t.Errorf("expected error for input %q, got nil", tc.input)
			}
			continue
		}
		if err != nil {
			t.Errorf("unexpected error for input %q: %v", tc.input, err)
			continue
		}
		if parsed.Day() != tc.expectedDay || parsed.Month() != tc.expectedMon || parsed.Year() != tc.expectedYr {
			t.Errorf("for input %q: expected %d-%v-%d, got %d-%v-%d",
				tc.input, tc.expectedYr, tc.expectedMon, tc.expectedDay, parsed.Year(), parsed.Month(), parsed.Day())
		}
		if parsed.Hour() != 23 || parsed.Minute() != 59 || parsed.Second() != 59 {
			t.Errorf("expected end of day 23:59:59, got %02d:%02d:%02d", parsed.Hour(), parsed.Minute(), parsed.Second())
		}
	}
}

func TestExtractDueToken(t *testing.T) {
	refTime := time.Date(2026, 8, 25, 10, 0, 0, 0, time.UTC)

	tests := []struct {
		input          string
		expectedClean  string
		expectedFound  bool
		expectedDay    int
		expectedRawDue string
	}{
		{"Fix login bug due:tomorrow #urgent", "Fix login bug #urgent", true, 26, "tomorrow"},
		{"Write unit tests @due(2026-08-30)", "Write unit tests", true, 30, "2026-08-30"},
		{"Normal bullet text without due", "Normal bullet text without due", false, 0, ""},
	}

	for _, tc := range tests {
		clean, dueDate, rawDue, found := ExtractDueToken(tc.input, refTime)
		if found != tc.expectedFound {
			t.Errorf("for input %q: expected found=%v, got %v", tc.input, tc.expectedFound, found)
		}
		if clean != tc.expectedClean {
			t.Errorf("for input %q: expected clean=%q, got %q", tc.input, tc.expectedClean, clean)
		}
		if tc.expectedFound {
			if dueDate == nil || dueDate.Day() != tc.expectedDay {
				t.Errorf("for input %q: expected day %d, got %v", tc.input, tc.expectedDay, dueDate)
			}
			if rawDue != tc.expectedRawDue {
				t.Errorf("for input %q: expected rawDue=%q, got %q", tc.input, tc.expectedRawDue, rawDue)
			}
		}
	}
}

func TestFormatDueBadge(t *testing.T) {
	refTime := time.Date(2026, 8, 25, 12, 0, 0, 0, time.UTC)

	// Past due (2 days ago: Aug 23)
	overdueTime := time.Date(2026, 8, 23, 23, 59, 59, 0, time.UTC).Unix()
	badge, cat := FormatDueBadge(overdueTime, false, refTime)
	if badge != "[Overdue: 2d]" || cat != "overdue" {
		t.Errorf("expected [Overdue: 2d] overdue, got %q %q", badge, cat)
	}

	// Overdue but Done
	doneBadge, doneCat := FormatDueBadge(overdueTime, true, refTime)
	if doneBadge != "[Due: Aug 23]" || doneCat != "done" {
		t.Errorf("expected [Due: Aug 23] done, got %q %q", doneBadge, doneCat)
	}

	// Due Today
	todayTime := time.Date(2026, 8, 25, 23, 59, 59, 0, time.UTC).Unix()
	todayBadge, todayCat := FormatDueBadge(todayTime, false, refTime)
	if todayBadge != "[Due Today]" || todayCat != "today" {
		t.Errorf("expected [Due Today] today, got %q %q", todayBadge, todayCat)
	}

	// Due Tomorrow
	tomorrowTime := time.Date(2026, 8, 26, 23, 59, 59, 0, time.UTC).Unix()
	tmrwBadge, tmrwCat := FormatDueBadge(tomorrowTime, false, refTime)
	if tmrwBadge != "[Due: Tomorrow]" || tmrwCat != "upcoming" {
		t.Errorf("expected [Due: Tomorrow] upcoming, got %q %q", tmrwBadge, tmrwCat)
	}
}
