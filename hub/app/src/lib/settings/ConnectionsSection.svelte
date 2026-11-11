<script lang="ts">
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Plus from '@lucide/svelte/icons/plus';
  import Button from '../design/Button.svelte';
  import Badge from '../design/Badge.svelte';
  import { settings } from '../stores/settings.svelte';
  import { hub } from '../stores/hub.svelte';
  import { calendarConnectArgs, defaultCalendarRoles, githubConnectArgs, INTEGRATION_STATE_LABEL, redactError } from './admin';
  import type { IntegrationEntry } from '../api/types';

  // GitHub connect form.
  let ghId = $state('github');
  let ghTitle = $state('');
  let ghRepos = $state('');
  let ghInterval = $state('');
  let ghError = $state<string | null>(null);

  // Calendar connect form. Roles default to the active role (plus developer
  // for eng-manager); `calRoles` null means "not touched yet".
  let calId = $state('calendar');
  let calTitle = $state('');
  let calPicked = $state<string[] | null>(null);
  let calError = $state<string | null>(null);
  const calRoles = $derived(calPicked ?? defaultCalendarRoles(hub.roleId));

  // Token entry. The value is only ever in the input: read once on submit,
  // cleared before the call, never kept in a store and never logged.
  let tokenFor = $state<string | null>(null);
  let tokenError = $state<string | null>(null);
  let tokenInput: HTMLInputElement | undefined = $state();

  let removeItems = $state<Record<string, boolean>>({});

  const fmt = (ts: string | null | undefined) => (ts ? new Date(ts).toLocaleString() : 'not yet');

  /** A calendar that reads an ICS feed: its "token" is the feed URL. */
  function isFeed(e: IntegrationEntry): boolean {
    const s = e.config.settings as { source?: unknown } | null;
    return e.config.kind === 'calendar' && s?.source !== 'mcp';
  }

  type Secret = { what: string; form: string; field: string; save: string };

  /** The secret an integration takes through token entry, or null. */
  function secretOf(e: IntegrationEntry): Secret | null {
    if (e.config.kind === 'github')
      return { what: 'token', form: 'Personal access token', field: 'GitHub personal access token (read-only scopes for the listed repos)', save: 'Save token' };
    if (isFeed(e))
      return { what: 'feed URL', form: 'Calendar feed URL', field: 'Secret calendar feed address (https:// or webcal://); it works like a password', save: 'Save feed URL' };
    if (e.config.kind === 'energy') {
      const s = e.config.settings as { source?: unknown; mqtt?: { username?: unknown } | null } | null;
      if (s?.source === 'mqtt')
        return typeof s.mqtt?.username === 'string'
          ? { what: 'password', form: 'MQTT broker password', field: `Password of the broker user ${s.mqtt.username}`, save: 'Save password' }
          : null;
      return { what: 'API token', form: 'Electricity Maps API token', field: 'Electricity Maps API token (sent only to the Electricity Maps API)', save: 'Save token' };
    }
    return null;
  }

  function repos(e: IntegrationEntry): string[] {
    const s = e.config.settings as { repos?: unknown } | null;
    return Array.isArray(s?.repos) ? s.repos.filter((x): x is string => typeof x === 'string') : [];
  }

  async function connect(e: SubmitEvent) {
    e.preventDefault();
    const r = githubConnectArgs(ghId, ghTitle, ghRepos, ghInterval);
    if (!r.ok) {
      ghError = r.error;
      return;
    }
    ghError = null;
    if (await settings.connectGithub(r.args)) {
      ghRepos = '';
      ghTitle = '';
      ghInterval = '';
      tokenFor = r.args.id;
      tokenError = null;
    }
  }

  async function connectCalendar(e: SubmitEvent) {
    e.preventDefault();
    const r = calendarConnectArgs(calId, calTitle, calRoles);
    if (!r.ok) {
      calError = r.error;
      return;
    }
    calError = null;
    if (await settings.connectGithub(r.args)) {
      calTitle = '';
      calPicked = null;
      tokenFor = r.args.id;
      tokenError = null;
    }
  }

  function toggleCalRole(id: string, on: boolean) {
    calPicked = on ? [...new Set([...calRoles, id])] : calRoles.filter((r) => r !== id);
  }

  async function submitToken(e: SubmitEvent, id: string) {
    e.preventDefault();
    if (!tokenInput) return;
    const token = tokenInput.value;
    tokenInput.value = '';
    tokenError = await settings.setToken(id, token);
    if (!tokenError) tokenFor = null;
  }
</script>

<section class="sp-section" aria-labelledby="conn-h">
  <h3 id="conn-h">Connections</h3>
  <p class="sp-lead">
    Integrations sync items from other tools into HalpTask as <span class="sp-mono">integration:&lt;id&gt;</span>. Synced content is untrusted: an agent that reads
    it is tainted. Items removed on the other side get the tag <span class="sp-mono">archived</span> and are hidden in widgets. Tokens go to the system keychain, never
    into HalpTask's data.
  </p>

  {#if !settings.integrationsRunning && !settings.integrationsError}
    <p class="sp-note" role="status">The integration runtime is not running, so nothing syncs. Settings below are kept and apply when it starts.</p>
  {/if}
  {#if settings.integrationsError}<p class="sp-error" role="alert">{settings.integrationsError}</p>{/if}

  <ul class="sp-list" aria-label="Integrations">
    {#each settings.integrations as e (e.config.id)}
      {@const label = INTEGRATION_STATE_LABEL[e.config.enabled ? e.status.state : 'disabled'] ?? { text: e.status.state, tone: 'neutral' }}
      {@const id = e.config.id}
      <li class="sp-card">
        <div class="sp-row between">
          <div class="sp-row">
            <span class="sp-title">{e.config.title || id}</span>
            <span class="sp-mono sp-meta">{e.config.kind} · {id}</span>
            <Badge tone={label.tone}>{label.text}</Badge>
          </div>
          <div class="sp-row">
            <Button size="sm" icon={RefreshCw} disabled={settings.busy === `sync:${id}`} onclick={() => settings.syncNow(id)}>Sync now</Button>
            {#if secretOf(e)}
              {@const what = secretOf(e)?.what}
              <Button size="sm" variant="ghost" onclick={() => ((tokenFor = tokenFor === id ? null : id), (tokenError = null))}>{e.status.has_secret ? `Replace ${what}` : `Set ${what}`}</Button>
            {/if}
          </div>
        </div>
        <p class="sp-meta">
          {e.status.items} items · last sync {fmt(e.status.last_sync)}{#if e.status.next_sync} · next {fmt(e.status.next_sync)}{/if} · every {Math.round(e.config.interval_secs / 60)} min · {secretOf(e)?.what ?? 'token'} {e.status.has_secret ? 'stored' : 'none'}{#if e.status.has_secret && settings.secretHints[id]} (<span class="sp-mono">{settings.secretHints[id]}</span>){/if}
        </p>
        {#if repos(e).length}<p class="sp-meta">Repos: <span class="sp-mono">{repos(e).join(', ')}</span></p>{/if}
        {#if e.status.error}<p class="sp-error">{redactError(e.status.error)}</p>{/if}

        {#if tokenFor === id}
          {@const sec = secretOf(e)}
          <form class="sp-row" onsubmit={(ev) => submitToken(ev, id)} aria-label="{sec?.form ?? 'Personal access token'} for {id}">
            <label class="sp-field"
              ><span>{sec?.field ?? 'Personal access token'}</span>
              <input bind:this={tokenInput} type="password" autocomplete="off" spellcheck="false" required />
            </label>
            <Button type="submit" variant="primary" disabled={settings.busy === `token:${id}`}>{sec?.save ?? 'Save token'}</Button>
          </form>
          {#if tokenError}<p class="sp-error" role="alert">{tokenError}</p>{/if}
        {/if}

        <div class="sp-row">
          <label class="sp-check"><input type="checkbox" bind:checked={removeItems[id]} /> Also delete its {e.status.items} synced items</label>
          <Button size="sm" variant="danger" disabled={settings.busy === `disconnect:${id}`} onclick={() => settings.disconnect(id, !!removeItems[id])}>Disconnect…</Button>
        </div>
      </li>
    {/each}
  </ul>
  {#if !settings.integrations.length && !settings.integrationsError}<p class="sp-meta">No integrations yet.</p>{/if}

  <form class="sp-card" onsubmit={connect} aria-labelledby="gh-h">
    <h4 id="gh-h" class="sp-title">Connect GitHub</h4>
    <p class="sp-meta">Syncs open pull requests and CI from the repos you list, and nothing else. You set the token in the next step.</p>
    <div class="sp-row">
      <label class="sp-field"><span>Id</span><input bind:value={ghId} spellcheck="false" autocomplete="off" required /></label>
      <label class="sp-field"><span>Title (optional)</span><input bind:value={ghTitle} placeholder="GitHub" autocomplete="off" /></label>
      <label class="sp-field"><span>Every (seconds, optional)</span><input bind:value={ghInterval} inputmode="numeric" placeholder="300" /></label>
    </div>
    <label class="sp-field"
      ><span>Repos, one owner/name per line</span>
      <textarea bind:value={ghRepos} placeholder="octo-org/api&#10;octo-org/web" spellcheck="false"></textarea>
    </label>
    {#if ghError}<p class="sp-error" role="alert">{ghError}</p>{/if}
    <div class="sp-row"><Button type="submit" variant="primary" icon={Plus} disabled={settings.busy === 'connect'}>Connect</Button></div>
  </form>

  <form class="sp-card" onsubmit={connectCalendar} aria-labelledby="cal-h">
    <h4 id="cal-h" class="sp-title">Connect a calendar</h4>
    <p class="sp-meta">Syncs meetings from an ICS feed into the roles you choose. 1:1 and manager meetings are written only when eng-manager is chosen. You set the feed URL in the next step.</p>
    <div class="sp-row">
      <label class="sp-field"><span>Id</span><input bind:value={calId} spellcheck="false" autocomplete="off" required /></label>
      <label class="sp-field"><span>Title (optional)</span><input bind:value={calTitle} placeholder="Calendar" autocomplete="off" /></label>
    </div>
    <fieldset class="sp-row" aria-label="Roles for this calendar">
      <legend class="sp-meta">Roles</legend>
      {#each hub.roles as r (r.id)}
        <label class="sp-check"><input type="checkbox" checked={calRoles.includes(r.id)} onchange={(ev) => toggleCalRole(r.id, ev.currentTarget.checked)} /> {r.title}</label>
      {/each}
    </fieldset>
    {#if calError}<p class="sp-error" role="alert">{calError}</p>{/if}
    <div class="sp-row"><Button type="submit" variant="primary" icon={Plus} disabled={settings.busy === 'connect'}>Connect calendar</Button></div>
  </form>
</section>
