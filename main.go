package main

import (
	"fmt"
	"os"
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/halpworld/halptask/config"
	"github.com/halpworld/halptask/model"
	"github.com/halpworld/halptask/ui"
	"github.com/halpworld/halptask/updater"
	flag "github.com/spf13/pflag"
)

const Version = config.Version

type CLIFlags struct {
	FilePath    string
	Encrypt     bool
	Version     bool
	Update      bool
	CheckUpdate bool
	Repo        string
	Help        bool
}

func printHelp() {
	helpText := `HalpTask - High-performance TUI task manager & outliner

Usage:
  halptask [flags]                      Launch interactive TUI
  halptask add [flags] <text>           Quick-capture a new task to Inbox
  halptask list [flags]                 List tasks or query status counts

Subcommands:
  add, capture                          Quick-capture task (e.g. halptask add "Fix bug #ops due:tomorrow")
  list, ls                              List tasks or query status (e.g. halptask list --count)

Add / Capture Flags:
  -f, --file string                     Path to halptask data file
  -e, --encrypt                         Force enable encryption
  -t, --tag strings                     Add tag(s) to task (can specify multiple times)
      --task                            Create as a task checkbox (default)
      --bullet                          Create as a non-task bullet point
      --top                             Prepend to top of Inbox instead of bottom
      --inbox string                    Target inbox section (default "Inbox")

List / Status Flags:
  -f, --file string                     Path to halptask data file
  -e, --encrypt                         Force enable encryption
  -a, --all                             List all tasks
  -t, --today                           List tasks due today, overdue, or in-progress
  -p, --in-progress                     List tasks currently in progress
  -c, --count                           Output compact single-line status tally
  -j, --json                            Output structured JSON array
      --no-color                        Disable ANSI color formatting

Root Flags:
  -f, --file string                     Path to halptask data file
  -e, --encrypt                         Force enable encryption
  -v, --version                         Print version
  -u, --update                          Check and perform auto-update
  -c, --check-update                    Check for available updates
  -r, --repo string                     Target GitHub repository override
  -h, --help                            Show help
`
	fmt.Print(helpText)
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
	fs.BoolVarP(&flags.Help, "help", "h", false, "Show help")

	err := fs.Parse(args)
	return flags, fs, err
}

func handleAddSubcommand(args []string, cfg *config.Config) {
	fs := flag.NewFlagSet("add", flag.ContinueOnError)
	var (
		filePath   string
		encrypt    bool
		tags       []string
		isTask     bool
		isBullet   bool
		prependTop bool
		inboxName  string
		help       bool
	)

	fs.StringVarP(&filePath, "file", "f", "", "Path to halptask data file")
	fs.BoolVarP(&encrypt, "encrypt", "e", false, "Force enable encryption")
	fs.StringSliceVarP(&tags, "tag", "t", nil, "Add tag(s) to task")
	fs.BoolVar(&isTask, "task", false, "Create as a task checkbox")
	fs.BoolVar(&isBullet, "bullet", false, "Create as a bullet point")
	fs.BoolVar(&prependTop, "top", false, "Prepend to top of Inbox")
	fs.StringVar(&inboxName, "inbox", "Inbox", "Target inbox section name")
	fs.BoolVarP(&help, "help", "h", false, "Show add command help")

	if err := fs.Parse(args); err != nil {
		os.Exit(2)
	}

	if help {
		fmt.Print(`Quick-Capture a new task to Inbox

Usage:
  halptask add [flags] "Task description #tags due:tomorrow"
  halptask capture [flags] "Task description #tags due:tomorrow"

Flags:
  -f, --file string    Path to halptask data file
  -e, --encrypt        Force enable encryption
  -t, --tag strings    Add tag(s) to task (can specify multiple times or comma-separated)
      --task           Create as a task checkbox (default)
      --bullet         Create as a non-task bullet point
      --top            Prepend to top of Inbox instead of bottom
      --inbox string   Target inbox section name (default "Inbox")
  -h, --help           Show help
`)
		os.Exit(0)
	}

	rawText := strings.Join(fs.Args(), " ")
	if strings.TrimSpace(rawText) == "" {
		fmt.Fprintln(os.Stderr, "Error: Task text is required.\nUsage: halptask add [flags] \"Task description #tag due:tomorrow\"")
		os.Exit(1)
	}

	opts := model.QuickCaptureOptions{
		FilePath:   filePath,
		Encrypt:    encrypt,
		RawText:    rawText,
		Tags:       tags,
		IsBullet:   isBullet,
		IsTask:     isTask,
		PrependTop: prependTop,
		InboxName:  inboxName,
	}

	_, confirmMsg, err := model.RunQuickCapture(cfg, opts)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	fmt.Println(confirmMsg)
	os.Exit(0)
}

func handleListSubcommand(args []string, cfg *config.Config) {
	fs := flag.NewFlagSet("list", flag.ContinueOnError)
	var (
		filePath   string
		encrypt    bool
		all        bool
		today      bool
		inProgress bool
		countOnly  bool
		jsonOutput bool
		noColor    bool
		help       bool
	)

	fs.StringVarP(&filePath, "file", "f", "", "Path to halptask data file")
	fs.BoolVarP(&encrypt, "encrypt", "e", false, "Force enable encryption")
	fs.BoolVarP(&all, "all", "a", false, "List all tasks")
	fs.BoolVarP(&today, "today", "t", false, "List tasks due today, overdue, or in-progress")
	fs.BoolVarP(&inProgress, "in-progress", "p", false, "List tasks currently in progress")
	fs.BoolVarP(&countOnly, "count", "c", false, "Output compact single-line status tally")
	fs.BoolVarP(&jsonOutput, "json", "j", false, "Output structured JSON array")
	fs.BoolVar(&noColor, "no-color", false, "Disable ANSI color formatting")
	fs.BoolVarP(&help, "help", "h", false, "Show list command help")

	if err := fs.Parse(args); err != nil {
		os.Exit(2)
	}

	if help {
		fmt.Print(`List tasks or query status counts

Usage:
  halptask list [flags]
  halptask ls [flags]

Flags:
  -f, --file string    Path to halptask data file
  -e, --encrypt        Force enable encryption
  -a, --all            List all tasks
  -t, --today          List tasks due today, overdue, or in-progress
  -p, --in-progress    List tasks currently in progress
  -c, --count          Output compact single-line status tally (for tmux, Waybar, etc.)
  -j, --json           Output structured JSON array
      --no-color       Disable ANSI color formatting
  -h, --help           Show help
`)
		os.Exit(0)
	}

	opts := model.QuickListOptions{
		FilePath:   filePath,
		Encrypt:    encrypt,
		All:        all,
		Today:      today,
		InProgress: inProgress,
		CountOnly:  countOnly,
		JSONOutput: jsonOutput,
		NoColor:    noColor,
	}

	if err := model.RunQuickList(cfg, opts, os.Stdout); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	os.Exit(0)
}

func main() {
	args := os.Args[1:]

	cfg, err := config.LoadConfig()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error loading config: %v\n", err)
	}

	// Check for subcommands as first argument
	if len(args) > 0 {
		subcmd := strings.ToLower(args[0])
		switch subcmd {
		case "add", "capture":
			handleAddSubcommand(args[1:], cfg)
			return
		case "list", "ls":
			handleListSubcommand(args[1:], cfg)
			return
		case "help":
			printHelp()
			os.Exit(0)
		}
	}

	cliFlags, _, err := parseFlags(args)
	if err != nil {
		os.Exit(2)
	}

	if cliFlags.Help {
		printHelp()
		os.Exit(0)
	}

	if cliFlags.Version {
		fmt.Printf("halptask v%s\n", Version)
		os.Exit(0)
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

	appModel, _ := ui.InitialModel(cfg, storage)

	p := tea.NewProgram(
		appModel,
		tea.WithAltScreen(),
		tea.WithMouseCellMotion(),
	)

	if _, err := p.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Error running halptask: %v\n", err)
		os.Exit(1)
	}
}
