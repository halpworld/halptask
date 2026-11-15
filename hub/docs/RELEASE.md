# Releasing HalpTask Hub

## Status (2026-11-11)

| Item | State |
| --- | --- |
| Tests | hub 465, shell 39, frontend 418. fmt, clippy and svelte-check are clean. One timing test, `forged_and_stalled_callbacks_do_not_end_the_flow` (OAuth), failed once under a full parallel run and passes alone; treat a single failure there as flaky. |
| `cargo audit` (hub, shell) | No vulnerabilities. Warnings only: `zbus_names`, `zvariant` and `zvariant_derive` are unmaintained (pulled in through `keyring`); `proc-macro-error` is unmaintained (build-time, through Tauri); `glib` 0.18 is unsound in `VariantStrIter` (Linux only, through Tauri's GTK 3 stack; no upgrade until Tauri moves to GTK 4). CI runs the audit on every push (`.github/workflows/hub.yml`). |
| `pnpm audit` | 1 low: `deepmerge`, build-time only (through `@sveltejs/vite-plugin-svelte`); no patched version exists. |
| First-party signing key | **Not set. Owner action.** `FIRST_PARTY_KEYS` is empty, so no pack can be "HalpTask official". The built-in role packs are compiled in and don't need it. |
| Revocation lists | Empty, which is right for a first release (nothing has been revoked). |
| macOS signing and notarization | Not set up. Owner action (needs an Apple Developer ID). |
| Windows | Tier C widgets show a placeholder, and there is no MCP host yet (no named-pipe transport). |

## Owner steps before a public release

### 1. Make the HalpTask signing key

Do this on a machine you trust, and keep the private key offline (a
password manager or a hardware-backed store), never in the repo.

```sh
cd hub
cargo run -p halpd -- pack keygen --out ~/halptask-release.key
```

This prints the key id, fingerprint and public key (base64). Put the public
key into `hub/crates/halp-pack/src/verify.rs`:

```rust
#[cfg(not(test))]
pub const FIRST_PARTY_KEYS: &[(&str, [u8; 32])] = &[("HalpTask 2026", [/* 32 bytes */])];
```

To get the bytes:

```sh
echo '<public key base64>' | base64 -d | xxd -i
```

Publish the fingerprint (website, README), so users can check it.

### 2. Revoke something later

Add the key's 64-hex fingerprint to `REVOKED_KEYS`, or a pack's manifest
SHA-256 to `REVOKED_PACK_HASHES`, in the same file, then ship a release.

### 3. Sign the macOS app

Add `signingIdentity` and notarization credentials to
`app/src-tauri/tauri.conf.json` > `bundle.macOS` (see Tauri's macOS signing
guide). Then:

```sh
cd hub/app && pnpm tauri build
```

### 4. Final gate

```sh
cd hub && cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo audit
cd app && pnpm check && pnpm test && pnpm build && pnpm audit
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test && cargo audit
```
