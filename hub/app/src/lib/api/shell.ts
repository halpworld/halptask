// The desktop shell's own commands (src-tauri/src/lib.rs), next to the one
// `dispatch` door into the core. In a browser (`pnpm dev`, the mock) there is
// no key to unlock and no capture window, so these are no-ops there.

import { isTauri } from './transport';

export type KeyStatus =
  | { state: 'starting' }
  | {
      state: 'ready';
      kind: string;
      /**
       * The MCP host (the socket agents reach through `halp-mcp`). Both
       * absent while it starts; then `ready` is re-sent on `halp://key`
       * with either `mcp` or `mcp_error` (CONTRACT round 2 §7).
       */
      mcp?: { socket: string } | null;
      mcp_error?: string | null;
    }
  | { state: 'locked' }
  | { state: 'setup'; reason: string }
  | { state: 'error'; reason: string };

const KEY_CHANNEL = 'halp://key';
/** Shortest passphrase for a new database (keys.rs MIN_PASSPHRASE_CHARS). */
export const MIN_PASSPHRASE_CHARS = 8;

export async function keyStatus(): Promise<KeyStatus> {
  if (!isTauri()) return { state: 'ready', kind: 'mock' };
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<KeyStatus>('key_status');
}

export async function onKeyStatus(cb: (s: KeyStatus) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import('@tauri-apps/api/event');
  return listen<KeyStatus>(KEY_CHANNEL, (e) => cb(e.payload));
}

/**
 * Sends the passphrase to Rust exactly once. Callers clear their own copy
 * (the input) before awaiting this.
 */
export async function unlock(passphrase: string, create: boolean): Promise<KeyStatus> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<KeyStatus>('unlock', { passphrase, create });
}

export async function hideCapture(): Promise<void> {
  if (!isTauri()) return;
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('hide_capture');
}

export async function onCaptureShown(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import('@tauri-apps/api/event');
  return listen('halp://capture-shown', () => cb());
}

/** `capture` for the quick-capture window (or `#capture` in a browser). */
export async function windowLabel(): Promise<string> {
  if (isTauri()) {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    return getCurrentWindow().label;
  }
  return typeof location !== 'undefined' && location.hash === '#capture' ? 'capture' : 'main';
}

/** Why the passphrase form cannot be submitted yet, or null. */
export function passphraseProblem(passphrase: string, confirm: string, create: boolean): string | null {
  if (!passphrase) return 'Enter your passphrase.';
  if (!create) return null;
  if (Array.from(passphrase).length < MIN_PASSPHRASE_CHARS) return `Use at least ${MIN_PASSPHRASE_CHARS} characters.`;
  if (passphrase !== confirm) return 'The passphrases do not match.';
  return null;
}

// ---- Integration tokens (CONTRACT v1.5 §3, docs/INTEGRATIONS.md "Tokens")

/** Why `integrations_set_token` refused a token. */
export const TOKEN_ERRORS = ['not_running', 'unknown_integration', 'empty', 'too_long', 'not_printable_ascii', 'invalid_url', 'keychain_error', 'internal'] as const;
export type TokenError = (typeof TOKEN_ERRORS)[number];

export const TOKEN_ERROR_TEXT: Record<TokenError, string> = {
  not_running: 'The integration runtime is not running in this window.',
  unknown_integration: 'That integration is not configured (any more).',
  empty: 'Paste a token first.',
  too_long: 'That token is longer than 4 KiB.',
  not_printable_ascii: 'Tokens are plain printable ASCII; check for spaces or stray characters.',
  invalid_url: 'That is not a calendar feed address: paste the full https:// (or webcal://) link.',
  keychain_error: 'The system keychain refused to store it.',
  internal: 'Something went wrong storing the token.',
};

/** The error code of a rejected token call (anything unexpected is `internal`). */
export function tokenErrorCode(err: unknown): TokenError {
  const code =
    typeof err === 'string'
      ? err
      : err instanceof Error
        ? err.message
        : typeof err === 'object' && err !== null && typeof (err as { code?: unknown }).code === 'string'
          ? (err as { code: string }).code
          : '';
  return (TOKEN_ERRORS as readonly string[]).includes(code) ? (code as TokenError) : 'internal';
}

type MockTokenSink = {
  setIntegrationToken?: (id: string, token: string) => Promise<void>;
  secretHint?: (id: string) => Promise<string | null>;
};

/**
 * Hand a token straight to the shell, which keeps it in the OS keychain.
 * Never through `dispatch`, never stored or logged here; callers clear
 * their own copy (the input) right after calling. Rejects with a
 * `TokenError` code string. In a browser the mock stands in.
 */
export async function setIntegrationToken(id: string, token: string): Promise<void> {
  if (!isTauri()) {
    const mock = (globalThis as { __halpMock?: MockTokenSink }).__halpMock;
    if (!mock?.setIntegrationToken) throw 'not_running';
    return mock.setIntegrationToken(id, token);
  }
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('integrations_set_token', { id, token });
}

/**
 * What may be shown of the stored secret of integration `id` (for a
 * calendar feed: its host and last four characters), or null. The shell
 * never returns the secret itself; any failure is just "no hint".
 */
export async function getSecretHint(id: string): Promise<string | null> {
  try {
    if (!isTauri()) {
      const mock = (globalThis as { __halpMock?: MockTokenSink }).__halpMock;
      return (await mock?.secretHint?.(id)) ?? null;
    }
    const { invoke } = await import('@tauri-apps/api/core');
    const hint = await invoke<string | null>('integrations_secret_hint', { id });
    return typeof hint === 'string' ? hint : null;
  } catch {
    return null;
  }
}
