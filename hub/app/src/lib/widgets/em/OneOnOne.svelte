<script lang="ts">
  import MessagesSquare from '@lucide/svelte/icons/messages-square';
  import Play from '@lucide/svelte/icons/play';
  import Square from '@lucide/svelte/icons/square';
  import Plus from '@lucide/svelte/icons/plus';
  import Lock from '@lucide/svelte/icons/lock';
  import Eye from '@lucide/svelte/icons/eye';
  import EyeOff from '@lucide/svelte/icons/eye-off';
  import CornerDownRight from '@lucide/svelte/icons/corner-down-right';
  import type { Item, JsonValue } from '../../api/types';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { query } from '../../stores/query.svelte';
  import { toasts } from '../../design/toast.svelte';
  import { toISODate } from '../../util/dates';
  import Badge from '../../design/Badge.svelte';
  import Button from '../../design/Button.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { markPrivate } from '../../privacy/index.svelte';
  import { PRIVATE_TITLE } from '../../privacy/private';
  import { saveConfig, typedItems } from './actions';
  import {
    agendaJson,
    agingLabel,
    commitmentColumns,
    elapsedLabel,
    EM_TYPES,
    MAX_AGENDA,
    nextOneOnOne,
    planEnd,
    readOneOnOnes,
    readPeople,
    type AgendaItem,
    type OneOnOne,
  } from './logic';
  import { whenLabel } from './format';

  let { widget, ambient }: WidgetProps = $props();

  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const today = $derived(hub.day?.date ?? toISODate(new Date(minute)));
  const peopleQ = typedItems(EM_TYPES.person);
  const onesQ = typedItems(EM_TYPES.oneOnOne);
  const commitQ = typedItems(EM_TYPES.commitment);

  const people = $derived(readPeople(peopleQ.data?.items ?? []));
  const ones = $derived(readOneOnOnes(onesQ.data?.items ?? []));

  let picked = $state<string>('');
  $effect(() => {
    const v = widget.config.person_id;
    if (typeof v === 'string') picked = v;
  });
  // The picked person, else whoever's one-on-one comes next.
  const person = $derived.by(() => {
    const p = people.find((x) => x.id === picked);
    if (p) return p;
    const soonest = [...people].sort((a, b) => (nextOneOnOne(ones, a.id, minute)?.scheduledFor ?? Infinity) - (nextOneOnOne(ones, b.id, minute)?.scheduledFor ?? Infinity))[0];
    return soonest ?? null;
  });
  const current = $derived(person ? nextOneOnOne(ones, person.id, minute) : null);
  const running = $derived(current?.status === 'in_progress');
  const peopleMap = $derived(new Map(people.map((p) => [p.id, p])));
  const theirs = $derived.by(() => {
    if (!person) return { iOwe: [], waitingOn: [] };
    const cols = commitmentColumns(commitQ.data?.items ?? [], peopleMap, today);
    return { iOwe: cols.iOwe.filter((r) => r.c.personId === person.id), waitingOn: cols.waitingOn.filter((r) => r.c.personId === person.id) };
  });

  function pick(id: string) {
    picked = id;
    hidePrivate();
    if (!ambient) void saveConfig(widget.id, { ...widget.config, person_id: id });
  }

  // ---------------------------------------------------------------- Writes

  let busy = $state(false);
  async function write(fn: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await fn();
    } finally {
      busy = false;
    }
  }

  async function updateFields(o: OneOnOne, fields: Record<string, JsonValue>) {
    return hub.run('tasks.update', { id: o.item.id, patch: { fields }, if_version: o.item.version });
  }

  async function createOne(agenda: AgendaItem[]): Promise<Item | undefined> {
    if (!person) return undefined;
    const when = new Date(minute + person.cadenceDays * 86_400_000);
    when.setHours(10, 0, 0, 0);
    return hub.run('tasks.create', {
      title: `1:1 with ${person.name}`,
      kind: 'bullet',
      type_id: EM_TYPES.oneOnOne,
      fields: { person_id: person.id, scheduled_for: when.toISOString(), status: 'scheduled', agenda: agendaJson(agenda) },
      roles: [hub.roleId],
    });
  }

  let draft = $state('');
  function addItem(e: SubmitEvent) {
    e.preventDefault();
    const text = draft.replace(/\s+/g, ' ').trim().slice(0, 300);
    if (!text) return;
    void write(async () => {
      const item: AgendaItem = { text, added_by: 'me', carried_over: false, done: false };
      const ok = current ? await updateFields(current, { ...current.item.fields, agenda: agendaJson([...current.agenda, item].slice(0, MAX_AGENDA)) }) : await createOne([item]);
      if (ok) draft = '';
    });
  }

  function toggle(i: number) {
    const o = current;
    if (!o) return;
    const agenda = o.agenda.map((a, j) => (j === i ? { ...a, done: !a.done } : a));
    void write(() => updateFields(o, { ...o.item.fields, agenda: agendaJson(agenda) }));
  }

  function start() {
    const o = current;
    if (!o) return;
    void write(() => updateFields(o, { ...o.item.fields, status: 'in_progress', started_at: new Date().toISOString() }));
  }

  /** End: mark done; unchecked items move to the next one-on-one. */
  function end() {
    const o = current;
    if (!o || !person) return;
    void write(async () => {
      const plan = planEnd(o, ones, new Date().toISOString(), person.cadenceDays);
      if (!(await updateFields(o, plan.ended))) return;
      const n = plan.next;
      if (n?.kind === 'update') await hub.run('tasks.update', { id: n.id, patch: { fields: n.fields }, if_version: n.version });
      else if (n?.kind === 'create') {
        await hub.run('tasks.create', { title: `1:1 with ${person.name}`, kind: 'bullet', type_id: EM_TYPES.oneOnOne, fields: n.fields, roles: [hub.roleId] });
      }
      const moved = o.agenda.filter((a) => !a.done).length;
      if (person.item.fields.last_one_on_one !== today) {
        await hub.run('tasks.update', { id: person.item.id, patch: { fields: { ...person.item.fields, last_one_on_one: today } } });
      }
      toasts.show(moved ? `One-on-one ended. ${moved} open item${moved === 1 ? '' : 's'} carried to the next one.` : 'One-on-one ended.', { tone: 'success', timeout: 3500 });
    });
  }

  // ---------------------------------------------------------------- Private notes
  // Loaded only after an explicit reveal, never shown blurred-but-present
  // before it, hidden again when you pick someone else or after two minutes.

  const REVEAL_MS = 120_000;
  let revealedFor = $state<string | null>(null);
  let revealedAt = $state(0);
  const revealed = $derived(!!person && revealedFor === person.id && clock.now - revealedAt < REVEAL_MS);
  const notesQ = query(
    () => (revealed && person ? hub.dispatch('tasks.list', { type_id: EM_TYPES.note, role: hub.roleId, limit: 200 }) : Promise.resolve({ items: [] as Item[] })),
    () => [revealed, person?.id, hub.itemsRev],
  );
  const notes = $derived(
    revealed && person
      ? (notesQ.data?.items ?? [])
          .filter((n) => n.type_id === EM_TYPES.note && n.fields.person_id === person.id && typeof n.fields.body === 'string')
          .sort((a, b) => String(b.fields.created_at ?? b.created_at).localeCompare(String(a.fields.created_at ?? a.created_at)))
      : [],
  );
  function reveal() {
    if (!person) return;
    revealedFor = person.id;
    revealedAt = clock.now;
  }
  function hidePrivate() {
    revealedFor = null;
    noteDraft = '';
  }
  let noteDraft = $state('');
  function addNote(e: SubmitEvent) {
    e.preventDefault();
    const body = noteDraft.trim().slice(0, 4000);
    if (!body || !person) return;
    const pid = person.id;
    void write(async () => {
      const res = await hub.run('tasks.create', {
        title: PRIVATE_TITLE,
        kind: 'bullet',
        type_id: EM_TYPES.note,
        fields: { person_id: pid, body, created_at: new Date().toISOString() },
        roles: [hub.roleId],
      });
      if (res) {
        markPrivate(res.id);
        noteDraft = '';
        revealedAt = clock.now;
      }
    });
  }
  const noteTime = (n: Item) => {
    const t = Date.parse(String(n.fields.created_at ?? n.created_at));
    return Number.isFinite(t) ? whenLabel(t, minute) : '';
  };
</script>

{#if peopleQ.loading && !peopleQ.data}
  <Skeleton lines={6} />
{:else if (peopleQ.error && !peopleQ.data)}
  <EmptyState compact title="Couldn’t load this card" hint={peopleQ.error} guide={false} />
{:else if !person}
  <EmptyState compact icon={MessagesSquare} title="No one to meet yet" hint="Add your team as people first; one-on-ones then appear for each of them." />
{:else}
  <div class="oo">
  <div class="top">
    <label class="pick">
      <span class="visually-hidden">Person</span>
      <select class="pii" value={person.id} disabled={ambient} onchange={(e) => pick(e.currentTarget.value)}>
        {#each people as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
      </select>
    </label>
    <span class="when">
      {#if running && current?.startedAt}
        <span class="live" role="timer" aria-label="One-on-one running for {elapsedLabel(clock.now - current.startedAt)}"><span class="dot" aria-hidden="true"></span><span class="num">{elapsedLabel(clock.now - current.startedAt)}</span></span>
      {:else}
        {whenLabel(current?.scheduledFor, minute)}
      {/if}
    </span>
    {#if !ambient && current}
      {#if running}
        <Button size="sm" variant="primary" icon={Square} disabled={busy} onclick={end}>End</Button>
      {:else}
        <Button size="sm" icon={Play} disabled={busy} onclick={start}>Start</Button>
      {/if}
    {/if}
  </div>

  <div class="body">
    <section class="agenda" aria-label="Agenda">
      <h3 class="head">Agenda</h3>
      {#if !current || current.agenda.length === 0}
        <p class="none">Nothing on the agenda yet.</p>
      {:else}
        <ul class="items">
          {#each current.agenda as a, i (i + a.text)}
            <li class:done={a.done}>
              <label>
                <input type="checkbox" checked={a.done} disabled={ambient || busy} onchange={() => toggle(i)} />
                <span class="t pii">{a.text}</span>
              </label>
              {#if a.carried_over}<span class="tag" title="Carried over from last time"><CornerDownRight size={11} aria-hidden="true" /><span class="visually-hidden">Carried over</span></span>{/if}
              {#if a.added_by !== 'me'}<Badge variant="outline">{a.added_by === 'them' ? 'them' : 'agent'}</Badge>{/if}
            </li>
          {/each}
        </ul>
      {/if}
      {#if !ambient}
        <form class="add" onsubmit={addItem}>
          <label class="visually-hidden" for="{widget.id}-add">Add an agenda item</label>
          <input id="{widget.id}-add" class="pii" type="text" maxlength="300" placeholder="Add an item…" bind:value={draft} disabled={busy} />
          <button type="submit" class="addbtn" aria-label="Add agenda item" disabled={busy || !draft.trim()}><Plus size={14} aria-hidden="true" /></button>
        </form>
      {/if}
    </section>

    <aside class="side">
      {#if person.goals.length}
        <section aria-label="Growth goals">
          <h3 class="head">Growth goals</h3>
          <ul class="goals">{#each person.goals as g (g)}<li class="pii">{g}</li>{/each}</ul>
        </section>
      {/if}
      <section aria-label="Open commitments with {person.name}">
        <h3 class="head">Open commitments</h3>
        {#if theirs.iOwe.length + theirs.waitingOn.length === 0}
          <p class="none">None either way.</p>
        {:else}
          <ul class="commit">
            {#each theirs.iOwe as r (r.c.item.id)}<li><span class="dir" title="You owe">↑</span><span class="pii">{r.c.what}</span> <span class="age">{agingLabel(r)}</span></li>{/each}
            {#each theirs.waitingOn as r (r.c.item.id)}<li><span class="dir" title="Waiting on them">↓</span><span class="pii">{r.c.what}</span> <span class="age">{agingLabel(r)}</span></li>{/each}
          </ul>
        {/if}
      </section>

      <section class="private" aria-label="Private notes">
        <h3 class="head"><Lock size={11} aria-hidden="true" /> Private notes</h3>
        {#if !revealed}
          <div class="veil" aria-hidden="true"><span></span><span></span><span></span></div>
          {#if !ambient}
            <Button size="sm" variant="ghost" icon={Eye} onclick={reveal}>Reveal notes</Button>
          {/if}
          <p class="fine">Only you can see these. Never shown in search, activity or to agents.</p>
        {:else}
          {#if notes.length === 0}<p class="none">No private notes about {person.name}.</p>{/if}
          <ul class="notes">
            {#each notes as n (n.id)}
              <li><span class="nbody pii-strong">{n.fields.body}</span><span class="nt">{noteTime(n)}</span></li>
            {/each}
          </ul>
          <form class="note" onsubmit={addNote}>
            <label class="visually-hidden" for="{widget.id}-note">New private note</label>
            <textarea id="{widget.id}-note" class="pii-strong" rows="2" maxlength="4000" placeholder="A private note…" bind:value={noteDraft} disabled={busy}></textarea>
            <span class="noteacts">
              <Button size="sm" variant="ghost" icon={EyeOff} onclick={hidePrivate}>Hide</Button>
              <Button size="sm" type="submit" disabled={busy || !noteDraft.trim()}>Save note</Button>
            </span>
          </form>
        {/if}
      </section>
    </aside>
  </div>
  </div>
{/if}

<style>
  .oo {
    container-type: inline-size;
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }
  .pick select {
    font: inherit;
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    color: var(--text);
    background: var(--surface-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 3px var(--space-2);
    max-width: 14em;
  }
  .pick select:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .when {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--accent-text);
    font-weight: var(--weight-semibold);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: var(--radius-full);
    background: var(--accent);
    animation: pulse 1.6s var(--ease-in-out) infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot {
      animation: none;
    }
  }
  .body {
    display: grid;
    grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr);
    gap: var(--space-4);
  }
  @container (max-width: 420px) {
    .body {
      grid-template-columns: 1fr;
    }
  }
  .head {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 0 0 var(--space-1);
    font-family: var(--font-label);
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
    color: var(--text-3);
  }
  .none,
  .fine {
    margin: 0 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .items,
  .goals,
  .commit,
  .notes {
    margin: 0 0 var(--space-2);
    padding: 0;
    list-style: none;
  }
  .items li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 3px 0;
  }
  .items label {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    color: var(--text);
    cursor: pointer;
  }
  .items input {
    margin-top: 3px;
    accent-color: var(--accent);
  }
  .items li.done .t {
    color: var(--text-3);
    text-decoration: line-through;
  }
  .tag {
    display: inline-flex;
    color: var(--accent-text);
  }
  .add {
    display: flex;
    gap: var(--space-1);
  }
  .add input,
  .note textarea {
    flex: 1;
    min-width: 0;
    font: inherit;
    font-size: var(--text-sm);
    color: var(--text);
    background: var(--surface-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 4px var(--space-2);
  }
  .add input:focus-visible,
  .note textarea:focus-visible,
  .addbtn:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .addbtn {
    display: inline-grid;
    place-items: center;
    width: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--text-2);
    cursor: pointer;
  }
  .addbtn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .goals li,
  .commit li {
    font-size: var(--text-xs);
    color: var(--text-2);
    padding: 2px 0;
  }
  .goals li::before {
    content: '·';
    margin-right: var(--space-1);
    color: var(--accent-text);
  }
  .dir {
    display: inline-block;
    width: 1.1em;
    color: var(--accent-text);
  }
  .age {
    color: var(--text-3);
  }
  .private {
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
  }
  .veil {
    display: grid;
    gap: 5px;
    margin-bottom: var(--space-2);
    filter: blur(2px);
  }
  .veil span {
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--skeleton);
  }
  .veil span:nth-child(2) {
    width: 80%;
  }
  .veil span:nth-child(3) {
    width: 55%;
  }
  .notes li {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 3px 0;
    border-bottom: 1px solid var(--border);
  }
  .nbody {
    font-size: var(--text-xs);
    color: var(--text);
    white-space: pre-wrap;
  }
  .nt {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .noteacts {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-1);
  }
</style>
