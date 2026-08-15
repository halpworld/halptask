package config

import (
	"os"
	"path/filepath"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

const Version = "0.0.9"

type TagConfig struct {
	Name  string `yaml:"name"`
	Emoji string `yaml:"emoji"`
	Color string `yaml:"color"` // LipGloss hex color string or name
}

type Config struct {
	AutoSave        bool        `yaml:"auto_save"`
	CheckUpdates    bool        `yaml:"check_updates"`
	UpdateInterval  string      `yaml:"update_interval,omitempty"`
	LastUpdateCheck string      `yaml:"last_update_check,omitempty"`
	GithubRepo      string      `yaml:"github_repo"`
	DataFile        string      `yaml:"data_file"`
	ArchiveFile     string      `yaml:"archive_file,omitempty"`
	Encrypted       bool        `yaml:"encrypted"`
	IndentSpaces    int         `yaml:"indent_spaces"`
	LeaderKey       string      `yaml:"leader_key"`
	ShowWhichKey    bool        `yaml:"show_which_key"`
	ShowDashboard   bool        `yaml:"show_dashboard"`
	ShowItemIDs     bool        `yaml:"show_item_ids"`
	Theme           string      `yaml:"theme"`
	DefaultItemType string      `yaml:"default_item_type"` // "bullet" or "task"
	Tags            []TagConfig `yaml:"tags,omitempty"`
}

var AvailableThemes = []string{"default", "tokyonight", "catppuccin", "dracula", "nord"}

func CycleTheme(current string) string {
	for i, t := range AvailableThemes {
		if t == current {
			return AvailableThemes[(i+1)%len(AvailableThemes)]
		}
	}
	return AvailableThemes[0]
}

func GetDefaultTagConfigs() []TagConfig {
	return []TagConfig{
		{Name: "bug", Emoji: "🐛", Color: "#f7768e"},     // Red
		{Name: "urgent", Emoji: "🔥", Color: "#ff9e64"},  // Orange
		{Name: "feature", Emoji: "✨", Color: "#7aa2f7"}, // Blue
		{Name: "work", Emoji: "💼", Color: "#bb9af7"},    // Purple
		{Name: "home", Emoji: "🏠", Color: "#9ece6a"},    // Green
		{Name: "idea", Emoji: "💡", Color: "#e0af68"},    // Yellow
		{Name: "pin", Emoji: "📌", Color: "#7dcfff"},     // Cyan
		{Name: "review", Emoji: "👀", Color: "#f7768e"},  // Pink/Red
	}
}

func DefaultConfig() *Config {
	home, err := os.UserHomeDir()
	if err != nil {
		home = "."
	}
	defaultData := filepath.Join(home, ".config", "halptask", "data.pb")
	defaultArchive := filepath.Join(home, ".config", "halptask", "archive.dat")
	return &Config{
		AutoSave:        true,
		CheckUpdates:    true,
		UpdateInterval:  "daily",
		GithubRepo:      "halpworld/halptask",
		DataFile:        defaultData,
		ArchiveFile:     defaultArchive,
		Encrypted:       false,
		IndentSpaces:    2,
		LeaderKey:       " ",
		ShowWhichKey:    true,
		ShowDashboard:   true,
		ShowItemIDs:     true,
		Theme:           "default",
		DefaultItemType: "bullet",
		Tags:            GetDefaultTagConfigs(),
	}
}

func ConfigDir() string {
	home, err := os.UserHomeDir()
	if err != nil {
		home = "."
	}
	return filepath.Join(home, ".config", "halptask")
}

func ConfigFilePath() string {
	return filepath.Join(ConfigDir(), "config.yaml")
}

func LoadConfig() (*Config, error) {
	cfg := DefaultConfig()
	dir := ConfigDir()
	if err := os.MkdirAll(dir, 0755); err != nil {
		return cfg, err
	}

	path := ConfigFilePath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			// Save default config if not existing
			_ = SaveConfig(cfg)
			return cfg, nil
		}
		return cfg, err
	}

	if err := yaml.Unmarshal(data, cfg); err != nil {
		return cfg, err
	}
	if cfg.DataFile == "" {
		home, _ := os.UserHomeDir()
		cfg.DataFile = filepath.Join(home, ".config", "halptask", "data.txt")
	}
	if cfg.ArchiveFile == "" {
		dir := filepath.Dir(cfg.DataFile)
		cfg.ArchiveFile = filepath.Join(dir, "archive.dat")
	}
	if cfg.UpdateInterval == "" {
		cfg.UpdateInterval = "daily"
	}
	if cfg.IndentSpaces <= 0 {
		cfg.IndentSpaces = 2
	}
	if cfg.LeaderKey == "" {
		cfg.LeaderKey = " "
	}
	if cfg.GithubRepo == "" {
		cfg.GithubRepo = "halpworld/halptask"
	}
	if cfg.DefaultItemType == "" {
		cfg.DefaultItemType = "bullet"
	} else {
		if cfg.DefaultItemType == "bulletpoint" || cfg.DefaultItemType == "bullet_point" {
			cfg.DefaultItemType = "bullet"
		}
		if cfg.DefaultItemType != "bullet" && cfg.DefaultItemType != "task" {
			cfg.DefaultItemType = "bullet"
		}
	}
	if len(cfg.Tags) == 0 {
		cfg.Tags = GetDefaultTagConfigs()
	}

	// Sanitize data_file if it erroneously points to config.yaml
	lowerDataFile := strings.ToLower(cfg.DataFile)
	baseDataFile := filepath.Base(lowerDataFile)
	if baseDataFile == "config.yaml" || baseDataFile == "config.yml" {
		pbPath := filepath.Join(dir, "data.pb")
		txtPath := filepath.Join(dir, "data.txt")
		if _, err := os.Stat(pbPath); err == nil {
			cfg.DataFile = pbPath
		} else if _, err := os.Stat(txtPath); err == nil {
			cfg.DataFile = txtPath
		} else {
			cfg.DataFile = pbPath
		}
		_ = SaveConfig(cfg)
	}
	return cfg, nil
}

func SaveConfig(cfg *Config) error {
	dir := ConfigDir()
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}
	data, err := yaml.Marshal(cfg)
	if err != nil {
		return err
	}
	return os.WriteFile(ConfigFilePath(), data, 0644)
}

// ShouldCheckForUpdate determines if an update check should be performed based on config settings and last check time.
func ShouldCheckForUpdate(cfg *Config) bool {
	if cfg == nil || !cfg.CheckUpdates || strings.ToLower(cfg.UpdateInterval) == "never" || strings.ToLower(cfg.UpdateInterval) == "off" {
		return false
	}
	if strings.ToLower(cfg.UpdateInterval) == "always" || cfg.UpdateInterval == "0" {
		return true
	}
	if cfg.LastUpdateCheck == "" {
		return true
	}
	lastCheck, err := time.Parse(time.RFC3339, cfg.LastUpdateCheck)
	if err != nil {
		return true
	}
	var interval time.Duration
	switch strings.ToLower(cfg.UpdateInterval) {
	case "hourly", "1h":
		interval = time.Hour
	case "weekly", "7d", "week":
		interval = 7 * 24 * time.Hour
	case "daily", "1d", "24h", "day":
		fallthrough
	default:
		interval = 24 * time.Hour
	}
	return time.Since(lastCheck) >= interval
}
