<script lang="ts">
  import Button from '../design/Button.svelte';
  import { settings } from '../stores/settings.svelte';
  import { AUDIT_DECISIONS, auditLine } from './admin';

  function apply(e: SubmitEvent) {
    e.preventDefault();
    void settings.loadAudit();
  }
</script>

<section class="sp-section" aria-labelledby="audit-h">
  <h3 id="audit-h">Audit</h3>
  <p class="sp-lead">Every agent call and every confirmed command, newest first. Arguments are kept only as a hash.</p>

  <form class="sp-row" onsubmit={apply} aria-label="Filter the audit log">
    <label class="sp-field"><span>Actor starts with</span><input bind:value={settings.auditActor} placeholder="agent:" spellcheck="false" /></label>
    <label class="sp-field"
      ><span>Decision</span>
      <select bind:value={settings.auditDecision}>
        <option value="">Any</option>
        {#each AUDIT_DECISIONS as d (d)}<option value={d}>{d}</option>{/each}
      </select>
    </label>
    <Button type="submit" disabled={settings.auditLoading}>Apply</Button>
  </form>
  {#if settings.auditDecision}<p class="sp-meta">The decision filter applies to the records loaded so far; load more to search further back.</p>{/if}

  {#if settings.auditError}<p class="sp-error" role="alert">{settings.auditError}</p>{/if}
  {#if !settings.shownAudit.length && !settings.auditLoading}<p class="sp-meta">No records.</p>{/if}
  <ol class="sp-audit" aria-label="Audit records">
    {#each settings.shownAudit as r (r.id)}<li>{auditLine(r)}</li>{/each}
  </ol>
  {#if settings.auditMore}
    <div class="sp-row"><Button disabled={settings.auditLoading} onclick={() => settings.loadAudit(true)}>Load more</Button></div>
  {/if}
</section>
