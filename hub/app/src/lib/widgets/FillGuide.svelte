<script lang="ts">
  import Plug from '@lucide/svelte/icons/plug';
  import Bot from '@lucide/svelte/icons/bot';
  import Copy from '@lucide/svelte/icons/copy';
  import Button from '../design/Button.svelte';
  import { hub } from '../stores/hub.svelte';
  import { settings, type SettingsSection } from '../stores/settings.svelte';
  import { toasts } from '../design/toast.svelte';
  import { CONNECT_LABEL, fillGuide } from './fill';

  interface Props {
    kind: string;
    /** The full guide (card header help); otherwise the short one in an empty card. */
    full?: boolean;
  }
  let { kind, full = false }: Props = $props();

  const g = $derived(fillGuide(kind));

  function openSettings(section: SettingsSection) {
    settings.section = section;
    hub.settingsOpen = true;
  }

  async function copyPrompt() {
    if (!g) return;
    try {
      await navigator.clipboard.writeText(g.agent);
      toasts.show('Prompt copied. Paste it to an AI agent paired with the Hub (Settings > Agents).', { tone: 'success', timeout: 5000 });
    } catch {
      toasts.show('Could not copy the prompt. Open “How to fill” on the card to select it.', { timeout: 7000 });
    }
  }
</script>

{#snippet buttons()}
  <div class="row">
    {#each g?.connect ?? [] as c (c)}
      <Button size="sm" variant="primary" icon={Plug} onclick={() => openSettings('connections')}>{CONNECT_LABEL[c]}</Button>
    {/each}
    <Button size="sm" variant={g?.connect?.length ? 'secondary' : 'primary'} icon={Copy} onclick={copyPrompt}>Copy prompt for your agent</Button>
    {#if full}
      <Button size="sm" variant="ghost" icon={Bot} onclick={() => openSettings('agents')}>Pair an agent</Button>
    {/if}
  </div>
{/snippet}

{#if g}
  <div class="fill" class:full>
    {#if full}
      <p class="shows">{g.shows}</p>
      {#if g.notYet}<p class="note">{g.notYet}</p>{/if}
      {@render buttons()}
      <h3>How to fill it</h3>
      <ol>
        {#if g.connect?.length}
          <li><span class="lead">Connect.</span> In Settings &gt; Connections: {g.connect.map((c) => CONNECT_LABEL[c].replace('Connect ', '')).join(' or ')}.</li>
        {/if}
        {#if g.manual}<li><span class="lead">By hand.</span> {g.manual}</li>{/if}
        <li>
          <span class="lead">Ask your agent.</span> Pair an AI agent in Settings &gt; Agents, then give it this:
          <pre class="prompt">{g.agent}</pre>
        </li>
      </ol>
    {:else}
      {#if g.manual}<p class="manual">{g.manual}</p>{/if}
      {#if g.notYet}<p class="note">{g.notYet}</p>{/if}
      {@render buttons()}
    {/if}
  </div>
{/if}

<style>
  .fill {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-3);
    max-width: 44ch;
    margin: 0 auto;
  }
  .fill.full {
    align-items: stretch;
    max-width: none;
    padding: var(--space-1) var(--space-1) var(--space-2);
    color: var(--text-2);
  }
  .shows {
    color: var(--text);
    font-weight: var(--weight-medium);
  }
  h3 {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-2);
    margin-top: var(--space-1);
  }
  ol {
    margin: 0;
    padding-left: 1.2em;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .lead {
    color: var(--text);
    font-weight: var(--weight-medium);
  }
  .prompt {
    margin-top: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
    white-space: pre-wrap;
    word-break: break-word;
    font-size: var(--text-xs);
    user-select: text;
  }
  .note {
    font-style: italic;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    justify-content: center;
    margin-top: var(--space-1);
  }
  .full .row {
    justify-content: flex-start;
  }
</style>
