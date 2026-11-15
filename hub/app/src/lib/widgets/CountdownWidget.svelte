<script lang="ts">
  import ShieldCheck from '@lucide/svelte/icons/shield-check';
  import ShieldQuestion from '@lucide/svelte/icons/shield-question';
  import type { WidgetProps } from './types';
  import Countdown from '../design/Countdown.svelte';
  import Badge from '../design/Badge.svelte';

  let { widget, ambient }: WidgetProps = $props();
  const c = $derived(widget.config);
  const target = $derived(typeof c.target === 'string' ? c.target : '');
  const label = $derived(typeof c.label === 'string' ? c.label : widget.title);
  // Regulatory certainty flag (PLAN §1): confirmed | proposed | uncertain.
  const certainty = $derived(typeof c.certainty === 'string' ? c.certainty : null);
  const when = $derived.by(() => {
    if (!target) return '';
    const d = /^\d{4}-\d{2}-\d{2}$/.test(target) ? new Date(`${target}T23:59:59`) : new Date(target);
    return new Intl.DateTimeFormat(undefined, { weekday: 'short', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(d);
  });
</script>

{#if target}
  <div class="cd">
    <span class="what">{label}</span>
    <Countdown {target} size={ambient ? 'xl' : 'lg'} seconds={!ambient} {label} />
    <div class="foot">
      <span class="when num">{when}</span>
      {#if certainty}
        <Badge tone={certainty === 'confirmed' ? 'success' : 'warning'} icon={certainty === 'confirmed' ? ShieldCheck : ShieldQuestion} title="Regulatory certainty">
          {certainty}
        </Badge>
      {/if}
    </div>
  </div>
{:else}
  <p class="muted">Set <code>config.target</code> to an ISO date.</p>
{/if}

<style>
  .cd {
    height: 100%;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-2);
  }
  .what {
    color: var(--text-2);
    font-weight: var(--weight-medium);
    font-size: var(--text-sm);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-xs);
  }
  .muted {
    color: var(--text-3);
  }
</style>
