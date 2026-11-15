<script lang="ts">
  // Shows a new client's token once, with ready-to-paste MCP config. The
  // token lives only in settings.pairing; closing the dialog drops it.
  import Copy from '@lucide/svelte/icons/copy';
  import Dialog from '../design/Dialog.svelte';
  import Button from '../design/Button.svelte';
  import { settings } from '../stores/settings.svelte';
  import { toasts } from '../design/toast.svelte';
  import { mcpSnippets, tokenPath } from './admin';

  const pairing = $derived(settings.pairing);
  const snippets = $derived(pairing ? mcpSnippets(pairing.client.id) : null);

  async function copy(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.show(`Copied ${what}`, { tone: 'success', timeout: 2500 });
    } catch {
      toasts.error('Could not copy; select the text and copy it yourself.');
    }
  }
</script>

<Dialog open={!!pairing} title={pairing ? `Paired ${pairing.client.name}` : 'Paired'} description="Copy the token now. You will not see it again: only its hash is stored." size="lg" onclose={() => settings.closePairing()}>
  {#if pairing && snippets}
    <div class="settings-panel pair">
      <section class="sp-card" aria-labelledby="pair-token-h">
        <div class="sp-row between">
          <h3 id="pair-token-h" class="sp-title">Token for {pairing.client.id}</h3>
          <Button size="sm" icon={Copy} onclick={() => copy(pairing.token, 'the token')}>Copy token</Button>
        </div>
        <pre class="sp-mono token" aria-label="Pairing token">{pairing.token}</pre>
        <p class="sp-meta">Save it to an owner-only file ({tokenPath(pairing.client.id)}, mode 0600). Run this, paste the token, press Enter, then Ctrl-D:</p>
        <pre class="sp-mono">{snippets.saveToken}</pre>
      </section>

      <section class="sp-card" aria-labelledby="pair-cc-h">
        <div class="sp-row between">
          <h3 id="pair-cc-h" class="sp-title">Claude Code</h3>
          <Button size="sm" variant="ghost" icon={Copy} onclick={() => copy(snippets.claudeCode, 'the Claude Code command')}>Copy</Button>
        </div>
        <pre class="sp-mono">{snippets.claudeCode}</pre>
      </section>

      <section class="sp-card" aria-labelledby="pair-cd-h">
        <div class="sp-row between">
          <h3 id="pair-cd-h" class="sp-title">Claude Desktop (claude_desktop_config.json)</h3>
          <Button size="sm" variant="ghost" icon={Copy} onclick={() => copy(snippets.claudeDesktop, 'the Claude Desktop config')}>Copy</Button>
        </div>
        <pre class="sp-mono">{snippets.claudeDesktop}</pre>
      </section>

      <section class="sp-card" aria-labelledby="pair-gen-h">
        <div class="sp-row between">
          <h3 id="pair-gen-h" class="sp-title">Any MCP client (stdio)</h3>
          <Button size="sm" variant="ghost" icon={Copy} onclick={() => copy(snippets.generic, 'the generic config')}>Copy</Button>
        </div>
        <pre class="sp-mono">{snippets.generic}</pre>
        <p class="sp-meta">The config only names the token file, so the token stays out of shell history, process lists and client configs.</p>
      </section>
    </div>
  {/if}
  {#snippet footer()}
    <Button variant="primary" onclick={() => settings.closePairing()}>I saved the token</Button>
  {/snippet}
</Dialog>

<style>
  .pair {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .token {
    font-size: var(--text-sm);
    user-select: all;
    word-break: break-all;
    white-space: pre-wrap;
  }
</style>
