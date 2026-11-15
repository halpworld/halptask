<script lang="ts">
  import { keyStatus, onKeyStatus, type KeyStatus } from '../api/shell';

  let status = $state<KeyStatus | null>(null);
  let error = $state<string | null>(null);

  // The shell re-sends `ready` once the MCP host is up (or failed).
  $effect(() => {
    let off: (() => void) | null = null;
    let gone = false;
    void onKeyStatus((s) => (status = s)).then((fn) => (gone ? fn() : (off = fn)));
    keyStatus().then(
      (s) => (status ??= s),
      (e) => (error = e instanceof Error ? e.message : String(e)),
    );
    return () => {
      gone = true;
      off?.();
    };
  });

  const ready = $derived(status && status.state === 'ready' ? status : null);
</script>

<section class="sp-section" aria-labelledby="shell-h">
  <h3 id="shell-h">Shell and MCP</h3>
  <dl class="sp-card">
    <div class="sp-row"><dt class="sp-title">Data key</dt><dd>{#if error}<span class="sp-error">{error}</span>{:else if !status}Checking…{:else if ready}Unlocked ({ready.kind}){:else}Starting{/if}</dd></div>
    <div class="sp-row">
      <dt class="sp-title">MCP host</dt>
      <dd>
        {#if !ready}—
        {:else if ready.mcp_error}<span class="sp-error">Not running: {ready.mcp_error}</span>
        {:else if ready.mcp}Listening on <span class="sp-mono">{ready.mcp.socket}</span>
        {:else if status?.state === 'ready' && 'mcp' in status}Not running
        {:else}Starting… (or not reported by this shell){/if}
      </dd>
    </div>
  </dl>
</section>
