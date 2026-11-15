<script lang="ts">
  // First screen in the desktop app: waits for the keychain, or asks for the
  // passphrase when the keychain is not used. The passphrase is passed to
  // Rust once; this component clears its fields before the call returns.
  import { onMount, type Snippet } from 'svelte';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import { keyStatus, onKeyStatus, passphraseProblem, unlock, MIN_PASSPHRASE_CHARS, type KeyStatus } from '../api/shell';

  let { children, onready }: { children: Snippet; onready: () => void } = $props();

  let status = $state<KeyStatus>({ state: 'starting' });
  let passphrase = $state('');
  let confirm = $state('');
  let error = $state<string | null>(null);
  let busy = $state(false);
  let readyFired = false;

  const create = $derived(status.state === 'setup');
  const problem = $derived(passphraseProblem(passphrase, confirm, create));

  function apply(s: KeyStatus) {
    // `ready` is sent again when the MCP host starts (with mcp or
    // mcp_error). Once unlocked, later statuses never re-lock, remount the
    // app or ask for the passphrase again.
    if (readyFired && s.state !== 'ready') return;
    status = s;
    if (s.state === 'ready' && !readyFired) {
      readyFired = true;
      onready();
    }
  }

  onMount(() => {
    let off: (() => void) | null = null;
    let gone = false;
    void onKeyStatus(apply).then((fn) => (gone ? fn() : (off = fn)));
    void keyStatus().then(apply);
    return () => {
      gone = true;
      off?.();
    };
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (problem || busy) return;
    const secret = passphrase;
    passphrase = '';
    confirm = '';
    busy = true;
    error = null;
    try {
      apply(await unlock(secret, create));
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }
</script>

{#if status.state === 'ready'}
  {@render children()}
{:else}
  <main class="gate">
    <div class="card">
      <KeyRound size={28} aria-hidden="true" />
      {#if status.state === 'starting'}
        <h1>Opening your Hub…</h1>
        <p>If macOS asks, allow HalpTask Hub to use its keychain item.</p>
      {:else if status.state === 'error'}
        <h1>The Hub could not open</h1>
        <p class="err" role="alert">{status.reason}</p>
      {:else}
        <h1>{create ? 'Choose a passphrase' : 'Unlock your Hub'}</h1>
        <p>
          {#if create}
            The system keychain is not available, so your data will be encrypted with a passphrase. There is no way to recover it if you forget it.
          {:else}
            Your data is encrypted with a passphrase.
          {/if}
        </p>
        <form onsubmit={submit}>
          <label>
            Passphrase
            <!-- svelte-ignore a11y_autofocus -->
            <input type="password" bind:value={passphrase} autocomplete={create ? 'new-password' : 'current-password'} autofocus disabled={busy} />
          </label>
          {#if create}
            <label>
              Repeat it
              <input type="password" bind:value={confirm} autocomplete="new-password" disabled={busy} />
            </label>
            <small>At least {MIN_PASSPHRASE_CHARS} characters.</small>
          {/if}
          {#if error}<p class="err" role="alert">{error}</p>{/if}
          <button type="submit" disabled={!!problem || busy}>{busy ? 'Working…' : create ? 'Create' : 'Unlock'}</button>
        </form>
      {/if}
    </div>
  </main>
{/if}

<style>
  .gate {
    display: grid;
    place-items: center;
    min-height: 100vh;
    background: var(--bg);
    color: var(--text);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: min(420px, calc(100vw - 32px));
    padding: var(--space-6);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    color: var(--text-2);
  }
  h1 {
    margin: 0;
    font-size: var(--text-lg);
    color: var(--text);
  }
  p {
    margin: 0;
    font-size: var(--text-sm);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
  }
  input {
    height: 36px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    color: var(--text);
  }
  input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  small {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .err {
    color: var(--danger);
  }
  button {
    height: 36px;
    border: 0;
    border-radius: var(--radius-md);
    background: var(--accent);
    color: var(--text-on-accent);
    font-weight: var(--weight-semibold);
  }
  button:disabled {
    opacity: 0.5;
  }
</style>
