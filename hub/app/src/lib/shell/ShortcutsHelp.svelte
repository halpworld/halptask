<script lang="ts">
  import { hub } from '../stores/hub.svelte';
  import Dialog from '../design/Dialog.svelte';
  import Kbd from '../design/Kbd.svelte';
  import { SHORTCUTS } from './keys';

  const groups = ['General', 'View', 'Navigate', 'Edit'] as const;
</script>

<Dialog open={hub.helpOpen} title="Keyboard shortcuts" description="Single keys work whenever you are not typing in a field." onclose={() => (hub.helpOpen = false)}>
  <div class="cols">
    {#each groups as g (g)}
      <section>
        <h3 class="label">{g}</h3>
        <dl>
          {#each SHORTCUTS.filter((s) => s.group === g) as s (s.keys)}
            <dt>{s.label}</dt>
            <dd><Kbd keys={s.keys} /></dd>
          {/each}
        </dl>
      </section>
    {/each}
    <section>
      <h3 class="label">In a task list</h3>
      <dl>
        <dt>Move between rows</dt>
        <dd><Kbd keys="j" /> <Kbd keys="k" /></dd>
        <dt>Complete</dt>
        <dd><Kbd keys="x" /></dd>
        <dt>Focus this task</dt>
        <dd><Kbd keys="f" /></dd>
        <dt>Delete (asks first)</dt>
        <dd><Kbd keys="Delete" /></dd>
      </dl>
    </section>
  </div>
</Dialog>

<style>
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-5) var(--space-6);
  }
  h3 {
    margin-bottom: var(--space-2);
  }
  dl {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-2) var(--space-3);
    margin: 0;
    font-size: var(--text-sm);
  }
  dt {
    color: var(--text-2);
  }
  dd {
    margin: 0;
    text-align: right;
  }
</style>
