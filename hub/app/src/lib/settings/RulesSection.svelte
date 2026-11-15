<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus';
  import Button from '../design/Button.svelte';
  import { hub } from '../stores/hub.svelte';
  import { settings } from '../stores/settings.svelte';
  import { clientStatus, ruleArgs, ruleMatchLines, rulableCommands } from './admin';

  const commands = $derived(rulableCommands(hub.commands));
  const active = $derived(settings.clients.filter((c) => clientStatus(c) === 'active'));

  let clientId = $state('');
  let command = $state('');
  let argsText = $state('{\n  \n}');
  let formError = $state<string | null>(null);

  const schema = $derived(commands.find((c) => c.name === command));
  const names = $derived(Object.fromEntries(settings.clients.map((c) => [c.id, c.name])));

  async function create(e: SubmitEvent) {
    e.preventDefault();
    const r = ruleArgs(clientId, command, argsText, schema);
    if (!r.ok) {
      formError = r.error;
      return;
    }
    formError = null;
    if (await settings.createRule(r.args)) argsText = '{\n  \n}';
  }
</script>

<section class="sp-section" aria-labelledby="rules-h">
  <h3 id="rules-h">Standing rules</h3>
  <p class="sp-lead">
    A standing rule lets one agent run one T2 command without asking, but only when every listed argument matches exactly. Rules never apply while the agent is
    tainted, and never to T3 commands: those always ask.
  </p>

  {#if !commands.length}
    <div class="sp-card" role="note" aria-labelledby="rule-empty-h" data-testid="rules-empty">
      <h4 id="rule-empty-h" class="sp-title">No commands to make rules for yet</h4>
      <p class="sp-meta">
        Rules apply only to T2 commands that agents can call. The Hub has none yet: agents read and write through T1 commands, which their grants allow, or ask
        you every time for T3 commands. Packs and extensions may add T2 commands; they will appear here when they do.
      </p>
    </div>
  {:else}
  <form class="sp-card" onsubmit={create} aria-labelledby="rule-new-h">
    <h4 id="rule-new-h" class="sp-title">New rule</h4>
    <div class="sp-row">
      <label class="sp-field"
        ><span>Agent</span>
        <select bind:value={clientId} required>
          <option value="" disabled>Choose an agent</option>
          {#each active as c (c.id)}<option value={c.id}>{c.name} ({c.id})</option>{/each}
        </select>
      </label>
      <label class="sp-field"
        ><span>Command (T2)</span>
        <select bind:value={command} required>
          <option value="" disabled>Choose a command</option>
          {#each commands as c (c.name)}<option value={c.name}>{c.name}</option>{/each}
        </select>
      </label>
    </div>
    {#if schema}<p class="sp-meta">{schema.description}</p>{/if}
    <label class="sp-field"
      ><span>Arguments that must match (JSON object, 1 to 16 keys)</span>
      <textarea bind:value={argsText} spellcheck="false" aria-describedby="rule-args-help"></textarea>
    </label>
    <p id="rule-args-help" class="sp-meta">For example {'{"parent_id": "…"}'} allows creating items only under that parent. Any argument not listed may be anything.</p>
    {#if formError}<p class="sp-error" role="alert">{formError}</p>{/if}
    <div class="sp-row"><Button type="submit" variant="primary" icon={Plus} disabled={settings.busy === 'rule' || !active.length}>Add rule</Button></div>
  </form>
  {/if}

  {#if settings.rulesError}<p class="sp-error" role="alert">{settings.rulesError}</p>{/if}
  {#if !settings.rules.length && commands.length}<p class="sp-meta">No standing rules. Every agent T2 command asks you first.</p>{/if}
  <ul class="sp-list" aria-label="Standing rules">
    {#each settings.rules as r (r.id)}
      <li class="sp-card">
        <div class="sp-row between">
          <span><span class="sp-title">{names[r.client_id] ?? r.client_id}</span> may run <span class="sp-mono">{r.command}</span> when</span>
          <Button size="sm" variant="danger" disabled={settings.busy === `rule:${r.id}`} onclick={() => settings.deleteRule(r)} aria-label="Delete rule {r.id}">Delete</Button>
        </div>
        <pre class="sp-mono">{ruleMatchLines(r.args).join('\n')}</pre>
        <p class="sp-meta">Added {new Date(r.created_at).toLocaleString()} by {r.created_by}</p>
      </li>
    {/each}
  </ul>
</section>
