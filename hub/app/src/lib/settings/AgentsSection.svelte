<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus';
  import Button from '../design/Button.svelte';
  import Badge from '../design/Badge.svelte';
  import PairDialog from './PairDialog.svelte';
  import { settings } from '../stores/settings.svelte';
  import { checkClientId, clientStatus, DATA_CLASS_LABELS, toggleClass, type GrantsDraft } from './admin';
  import type { ClientView } from '../api/types';

  let name = $state('');
  let id = $state('');
  let formError = $state<string | null>(null);
  /** Grants being edited, by client id. */
  let drafts = $state<Record<string, GrantsDraft>>({});

  const fmt = (ts: string | null | undefined) => (ts ? new Date(ts).toLocaleString() : 'never');

  async function pair(e: SubmitEvent) {
    e.preventDefault();
    const n = name.trim();
    if (!n) {
      formError = 'Give the agent a name.';
      return;
    }
    const i = id.trim();
    const bad = i ? checkClientId(i) : null;
    if (bad) {
      formError = bad;
      return;
    }
    formError = null;
    if (await settings.pair(n, i || undefined)) {
      name = '';
      id = '';
    }
  }

  function edit(c: ClientView) {
    drafts = { ...drafts, [c.id]: { t1: c.grants.t1, data_classes: [...c.grants.data_classes] } };
  }
  function cancel(c: ClientView) {
    const { [c.id]: _, ...rest } = drafts;
    drafts = rest;
  }
  async function save(c: ClientView) {
    const d = drafts[c.id];
    if (d && (await settings.saveGrants(c, d))) cancel(c);
  }
  function rulesFor(c: ClientView) {
    return settings.rules.filter((r) => r.client_id === c.id).length;
  }
</script>

<section class="sp-section" aria-labelledby="agents-h">
  <h3 id="agents-h">Agents</h3>
  <p class="sp-lead">
    Agents reach HalpTask through <span class="sp-mono">halp-mcp</span> with a token you pair here. Reads run directly; writes ask you first unless a grant or a
    standing rule allows them. An agent that has read untrusted content (email, web, synced items) is tainted for 30 minutes: then every write asks again.
  </p>

  <form class="sp-card" onsubmit={pair} aria-labelledby="pair-h">
    <h4 id="pair-h" class="sp-title">Pair an agent</h4>
    <div class="sp-row">
      <label class="sp-field"><span>Name</span><input bind:value={name} placeholder="Claude Code" autocomplete="off" /></label>
      <label class="sp-field"><span>Id (optional)</span><input bind:value={id} placeholder="from the name" autocomplete="off" spellcheck="false" /></label>
      <Button type="submit" variant="primary" icon={Plus} disabled={settings.busy === 'pair'}>Pair</Button>
    </div>
    {#if formError}<p class="sp-error" role="alert">{formError}</p>{/if}
  </form>

  {#if settings.agentsError}<p class="sp-error" role="alert">{settings.agentsError}</p>{/if}

  {#if !settings.clients.length && !settings.agentsLoading}
    <p class="sp-meta">No agents paired yet.</p>
  {/if}
  <ul class="sp-list" aria-label="Paired agents">
    {#each settings.clients as c (c.id)}
      {@const act = settings.activity[c.id]}
      {@const status = clientStatus(c)}
      {@const draft = drafts[c.id]}
      <li class="sp-card">
        <div class="sp-row between">
          <div class="sp-row">
            <span class="sp-title">{c.name}</span>
            <span class="sp-mono sp-meta">{c.actor}</span>
            {#if status === 'revoked'}<Badge tone="neutral">Revoked</Badge>{:else}<Badge tone="success">Active</Badge>{/if}
            {#if act?.tainted}<Badge tone="warning" title="It read untrusted content recently">Tainted</Badge>{/if}
          </div>
          {#if status === 'active'}
            <div class="sp-row">
              {#if act?.tainted}<Button size="sm" disabled={settings.busy === `taint:${c.id}`} onclick={() => settings.clearTaint(c)}>Trust again</Button>{/if}
              <Button size="sm" variant="danger" disabled={settings.busy === `revoke:${c.id}`} onclick={() => settings.revoke(c)}>Revoke</Button>
            </div>
          {/if}
        </div>
        <p class="sp-meta">
          Paired {fmt(c.created_at)} · last seen {act?.lastSeen ? fmt(act.lastSeen) : 'not since the Hub started'}{#if c.revoked_at} · revoked {fmt(c.revoked_at)}{/if} · {rulesFor(c)} standing rule{rulesFor(c) === 1 ? '' : 's'}
        </p>
        {#if act?.tainted}
          <p class="sp-meta">Tainted: it read untrusted content. Standing rules do not apply until you trust it again or until {fmt(act.taintedUntil)}.</p>
        {/if}

        {#if draft}
          <fieldset>
            <legend>What {c.name} may do</legend>
            <label class="sp-check"><input type="checkbox" checked={draft.t1} onchange={(e) => (drafts[c.id] = { ...draft, t1: e.currentTarget.checked })} /> Make small changes (T1) without asking</label>
          </fieldset>
          <fieldset>
            <legend>What {c.name} may read</legend>
            {#each settings.dataClasses as cls (cls.id)}
              <label class="sp-check" title={cls.source}>
                <input type="checkbox" checked={draft.data_classes.includes(cls.id)} onchange={(e) => (drafts[c.id] = toggleClass(draft, cls.id, e.currentTarget.checked))} />
                <span>{cls.label} <span class="sp-meta">({cls.id}{cls.builtin ? '' : ` · ${cls.source}`})</span></span>
              </label>
            {/each}
          </fieldset>
          <div class="sp-row">
            <Button size="sm" variant="primary" disabled={settings.busy === `grants:${c.id}`} onclick={() => save(c)}>Save grants</Button>
            <Button size="sm" variant="ghost" onclick={() => cancel(c)}>Cancel</Button>
          </div>
        {:else}
          <div class="sp-row">
            <span class="sp-meta">T1 without asking: {c.grants.t1 ? 'yes' : 'no'} · reads: {c.grants.data_classes.map((x) => settings.dataClasses.find((d) => d.id === x)?.label ?? DATA_CLASS_LABELS[x] ?? x).join(', ') || 'nothing'}</span>
            {#if status === 'active'}<Button size="sm" variant="ghost" onclick={() => edit(c)}>Edit grants</Button>{/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
</section>

<PairDialog />
