# Run HalpTask Hub on your Mac

## 1. One-time setup

You need Homebrew, Node 24 with pnpm 10, and Rust 1.99.

```sh
brew install rustup
rustup-init -y --default-toolchain 1.99.1
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # add this line to ~/.zshrc
corepack enable                                     # gives you pnpm
xcode-select --install                              # if you have no Xcode command line tools
```

## 2. Try it on a throwaway hub first

This keeps your test data in its own folder and out of your keychain:

```sh
cd hub/app
pnpm install
HALP_HUB_DATA_DIR=/tmp/halp-try HALP_HUB_NO_KEYCHAIN=1 pnpm tauri dev
```

The first build takes a few minutes. Then the window opens on a first-run
screen: choose any passphrase for this test hub. To start over, quit the app
and delete `/tmp/halp-try`.

- Pick a role (Developer, Engineering Manager or Chief Sustainability
  Officer). Each role brings its own views.
- **Cmd+Shift+Space** opens quick capture from anywhere.
  Try `Fix login #bug due:friday`.
- Closing the window keeps the Hub running in the menu bar, where the
  Quit item is.

## 3. Fill the dashboard

A new hub starts empty. Every card says how it gets its data: an empty card
shows a short guide, and the **?** in any card's header opens the full one.
There are three ways in:

- **Connect** (Settings > Connections): GitHub fills the review queue, your
  pull requests and CI radar. A calendar feed (the secret iCal address from
  Google, Outlook, Apple or Fastmail) fills the day plan, deep work, meeting
  load and one-on-ones. Connect it for the Engineering Manager role too if
  you use that role.
- **By hand:** tasks, due dates and focus (press **N**, or **Cmd+Shift+Space**).
- **Ask your agent:** everything else (people, commitments, asks, disclosure
  deadlines, targets, regulatory changes and so on) has no connector yet.
  Pair an agent (step 5), then use **Copy prompt for your agent** on the card
  and paste it to the agent. The prompt tells it which item type to create,
  to read the type's fields first, and not to invent values.

## 4. Try an extension pack (Tier B and Tier C)

1. Open Settings > Packs and choose
   `hub/examples/packs/acme-sre.halppack`.
2. You get a review first. The pack is signed with a key the Hub doesn't
   know yet. Tick both **I have read this review…** and **Trust this
   publisher key…**, then choose **Install**. This key is a test-only key,
   so only trust it in this throwaway hub.
3. Switch to the **Platform SRE** role and create a few tasks of type
   **Incident**.
4. Add a **Computed** widget with the config
   `{"transform": "acme.platform-sre/burn", "view": "metric"}`. It should
   show the number of open incidents. This is Tier B, a sandboxed
   WebAssembly transform.
5. Add the **On-call** widget. It should list the incidents. This is Tier C,
   a sandboxed HTML widget.

Tier B and Tier C have not yet been tried by hand in the running app, so
note anything that looks off.

## 5. Connect an AI agent (MCP)

While the app is running it listens on `<data dir>/halp.sock`. From `hub/`:

```sh
cargo install --path crates/halpd
cargo install --path crates/halp-mcp
halpd --data-dir /tmp/halp-try clients pair --name "Claude Code"
```

`pair` prints a token once, along with ready-made client config. Save the
token to a file only you can read, then:

```sh
claude mcp add halptask --env HALP_MCP_TOKEN_FILE=$HOME/.halptask/claude-code.token --env HALP_SOCKET=/tmp/halp-try/halp.sock -- halp-mcp
```

Agents start with `general` data only. When an agent does something that
needs your approval, the app shows a consent dialog. Grants are managed in
Settings. See `docs/MCP.md` for the details.

## 6. Use it for real

Run the same command without the two variables:

```sh
pnpm tauri dev
```

This uses `~/Library/Application Support/halptask-hub` and keeps the
database key in your login keychain. For an app you can put in
/Applications:

```sh
pnpm tauri build
open "src-tauri/target/release/bundle/macos/HalpTask Hub.app"
```

The build is not signed with an Apple Developer ID. The first time, open it
with right-click > Open, or allow it in System Settings > Privacy & Security.
