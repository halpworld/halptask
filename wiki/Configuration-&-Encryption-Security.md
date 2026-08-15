# Configuration & Encryption Security 🔒

HalpTask prioritizes data integrity, user customization, and hardware-grade file encryption. This document details file configuration settings, visual theme palettes, and the underlying AES-256-GCM encryption architecture.

---

## ⚙️ Configuration File (`~/.config/halptask/config.yaml`)

Configuration settings are stored in YAML format at `~/.config/halptask/config.yaml`.

```yaml
# HalpTask Configuration File
auto_save: true
check_updates: true
data_file: ~/.config/halptask/data.txt
archive_file: ~/.config/halptask/archive.dat
default_item_type: bullet # Options: "bullet" or "task"
encrypted: false
indent_spaces: 2
leader_key: " "
show_which_key: true
show_item_ids: true # Options: true or false
theme: default # Options: "default", "tokyonight", "catppuccin", "dracula", "nord"
```

### Configuration Key Reference:

| Option | Type | Default | Description |
|---|---|---|---|
| `auto_save` | Boolean | `true` | Automatically saves tree modifications to disk in real-time. |
| `check_updates` | Boolean | `true` | Checks for new GitHub release versions on launch non-intrusively. |
| `data_file` | String | `~/.config/halptask/data.txt` | Path to default Markdown storage file. |
| `archive_file` | String | `~/.config/halptask/archive.dat` | Path to compressed archive storage file. |
| `default_item_type` | String | `bullet` | Default node type when creating new items (`bullet` or `task`). |
| `encrypted` | Boolean | `false` | Specifies whether default data file uses AES-256 encryption. |
| `indent_spaces` | Integer | `2` | Number of visual indentation spaces per tree depth level. |
| `leader_key` | String | `" "` | Primary key trigger for leader menu popup (default `<space>`). |
| `show_which_key` | Boolean | `true` | Renders WhichKey leader popup menu upon key press. |
| `show_item_ids` | Boolean | `true` | Renders permanent `#ID` numbers next to items in tree view. |
| `theme` | String | `default` | Visual TUI color theme palette. |

---

## 🎨 Visual Color Themes

HalpTask features 5 professionally styled color palettes:

1. **`default`**: Classic Lip Gloss purple/blue theme with crisp high-contrast text.
2. **`tokyonight`**: Popular TokyoNight storm palette featuring deep blues and neon purple accents.
3. **`catppuccin`**: Soft, warm mocha aesthetic with pastel pastel blue and teal tones.
4. **`dracula`**: High-contrast vampire dark mode with pink, purple, and green highlights.
5. **`nord`**: Cool arctic ice blue palette inspired by Nord colors.

### How to Switch Themes:
- **Interactive Modal**: Press `<space> c c` to open the Config Dashboard, highlight Theme, and press `Space` or `t`.
- **Quick Leader Toggle**: Press `<space> c t` in Normal mode to instantly cycle through available themes!

---

## 🔒 AES-256-GCM + PBKDF2 Encryption Architecture

HalpTask features robust file-level encryption to keep sensitive tasks, legal notes, and credentials safe.

<p align="center">
  <img src="images/halptask_config.jpg" alt="Encryption & Config View" width="450"/>
</p>

### Cryptographic Engine Overview:
- **Symmetric Cipher**: **AES-256-GCM** (Galois/Counter Mode), providing both confidentiality and authenticated data integrity verification.
- **Key Derivation Function (KDF)**: **PBKDF2** using **HMAC-SHA-256** with **100,000 iterations**.
- **Salt & Nonce**:
  - Cryptographically secure 16-byte random salt generated via `crypto/rand`.
  - Unique 12-byte random initialization nonce per save operation to prevent replay or pattern attacks.
- **File Header Header**: Encrypted files begin with the signature `# HALPTASK-ENCRYPTED-v1`.

### File Storage Specification:
```
┌─────────────────────────────────────────────────────────┐
│ Header: # HALPTASK-ENCRYPTED-v1\n                       │
├─────────────────────────────────────────────────────────┤
│ Salt:   16 bytes (raw salt)                             │
├─────────────────────────────────────────────────────────┤
│ Nonce:  12 bytes (GCM nonce)                            │
├─────────────────────────────────────────────────────────┤
│ Payload: AES-256-GCM ciphertext + 16-byte Auth Tag     │
│          (Encrypts Protobuf v1 binary payload)          │
└─────────────────────────────────────────────────────────┘
```

---

## 💾 Protobuf Binary Storage Protocol (`.pb`)

HalpTask uses **Protobuf v1 binary protocol** (`.pb`) for data persistence:

- **High Performance & Compact**: Zero-dependency binary serialization with varint and length-delimited wire encoding.
- **Schema Versioning**: Root `TreeProto` embeds `schema_version = 1` for future backward/forward compatibility.
- **Item Hierarchy & Metadata**: Preserves item IDs, title text, task statuses (`TASK_STATUS_TODO`, `TASK_STATUS_IN_PROGRESS`, `TASK_STATUS_DONE`), tags, fold states, timestamps, sequence numbers, and attached Markdown notes (`note`).
- **Legacy Markdown Auto-Migration**: Legacy `.txt` / `.md` files are parsed and upgraded to `.pb` Protobuf binary payloads while preserving a `.bak` backup file.

---

## 🛡️ Passphrase Prompt & Auto-Save Safety

To prevent data corruption or accidental lockouts, HalpTask implements safety mechanisms:

1. **Passphrase Prompt Modal (`ModePassphrasePrompt`)**: When opening an encrypted file, HalpTask prompts for your secret passphrase before attempting decryption or loading the UI.
2. **Auto-Save Protection**: If an encrypted file is opened but has not yet been unlocked with a valid passphrase, **auto-save is automatically suspended**. This prevents empty or unencrypted data from overwriting your encrypted file.
3. **Encryption Toggles**:
   - `<space> e e`: Toggle encryption state on/off for current file.
   - `<space> e p`: Change encryption passphrase.
4. **Headless CLI Passphrase Automation (`HALPTASK_PASSPHRASE`)**:
   - When using headless commands (`halptask add` or `halptask list`), you can supply your vault passphrase via the `HALPTASK_PASSPHRASE` environment variable to automate background scripts, tmux statusbars, and CI/CD pipelines without interactive prompts:
     ```bash
     HALPTASK_PASSPHRASE="my-secret-vault-key" halptask add "Urgent security patch #sec"
     HALPTASK_PASSPHRASE="my-secret-vault-key" halptask list --count
     ```
   - If `HALPTASK_PASSPHRASE` is not set and the command is executed in an interactive terminal, HalpTask prompts securely via `term.ReadPassword` on standard error.
