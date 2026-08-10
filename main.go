package main

import (
	"fmt"
	"os"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/kenth/halptask/config"
	"github.com/kenth/halptask/model"
	"github.com/kenth/halptask/ui"
	"github.com/kenth/halptask/updater"
	flag "github.com/spf13/pflag"
)

const Version = "0.0.5"

type CLIFlags struct {
	FilePath    string
	Encrypt     bool
	Version     bool
	Update      bool
	CheckUpdate bool
	Repo        string
}

func parseFlags(args []string) (*CLIFlags, *flag.FlagSet, error) {
	flags := &CLIFlags{}
	fs := flag.NewFlagSet("halptask", flag.ContinueOnError)
	fs.StringVarP(&flags.FilePath, "file", "f", "", "Path to halptask data file")
	fs.BoolVarP(&flags.Encrypt, "encrypt", "e", false, "Force enable encryption")
	fs.BoolVarP(&flags.Version, "version", "v", false, "Print version")
	fs.BoolVarP(&flags.Update, "update", "u", false, "Check for updates and perform auto-update if a new version is available")
	fs.BoolVarP(&flags.CheckUpdate, "check-update", "c", false, "Check if a new version is available")
	fs.StringVarP(&flags.Repo, "repo", "r", "", "Override target GitHub repository (e.g. owner/repo)")

	err := fs.Parse(args)
	return flags, fs, err
}

func main() {
	cliFlags, _, err := parseFlags(os.Args[1:])
	if err != nil {
		os.Exit(2)
	}

	if cliFlags.Version {
		fmt.Printf("halptask v%s\n", Version)
		os.Exit(0)
	}

	cfg, err := config.LoadConfig()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error loading config: %v\n", err)
	}

	targetRepo := cfg.GithubRepo
	if cliFlags.Repo != "" {
		targetRepo = cliFlags.Repo
	}

	if cliFlags.CheckUpdate || cliFlags.Update {
		fmt.Printf("Checking for updates from %s...\n", targetRepo)
		rel, err := updater.CheckForUpdate(Version, targetRepo)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error checking for updates: %v\n", err)
			os.Exit(1)
		}

		if rel.NewRepo != "" && rel.NewRepo != cfg.GithubRepo {
			fmt.Printf("Repository has moved to %s. Updating config...\n", rel.NewRepo)
			cfg.GithubRepo = rel.NewRepo
			_ = config.SaveConfig(cfg)
		}

		if !rel.HasUpdate {
			fmt.Printf("halptask v%s is up to date! (Latest: v%s)\n", Version, rel.Version)
			os.Exit(0)
		}

		fmt.Printf("A new version of halptask is available: v%s (current: v%s)\n", rel.Version, Version)

		if cliFlags.Update {
			canUpdate, realPath, reason := updater.CanUpdate()
			if !canUpdate {
				fmt.Fprintf(os.Stderr, "Cannot update executable at %s: %s\n", realPath, reason)
				os.Exit(1)
			}

			fmt.Printf("Updating halptask binary at %s...\n", realPath)
			if err := updater.DoUpdate(rel); err != nil {
				fmt.Fprintf(os.Stderr, "Update failed: %v\n", err)
				os.Exit(1)
			}
			fmt.Printf("Successfully updated halptask to v%s!\n", rel.Version)
			os.Exit(0)
		}
		os.Exit(0)
	}

	targetFilePath := cfg.DataFile
	if cliFlags.FilePath != "" {
		targetFilePath = cliFlags.FilePath
	}

	isEncrypted := cfg.Encrypted || cliFlags.Encrypt

	storage := model.NewStorage(targetFilePath, isEncrypted)

	appModel, initCmd := ui.InitialModel(cfg, storage)

	p := tea.NewProgram(
		appModel,
		tea.WithAltScreen(),
		tea.WithMouseCellMotion(),
	)

	if initCmd != nil {
		go func() {
			// Exec initial command if any
		}()
	}

	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Error running halptask: %v\n", err)
		os.Exit(1)
	}
}
