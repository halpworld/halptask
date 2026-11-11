<script lang="ts">
  interface Props {
    lines?: number;
    height?: string;
    width?: string;
    radius?: string;
  }
  let { lines = 1, height = '12px', width, radius = 'var(--radius-sm)' }: Props = $props();
  // Slightly varied widths read as text instead of bars.
  const widths = ['92%', '78%', '85%', '64%', '88%', '72%'];
</script>

<div class="sk-group" aria-hidden="true">
  {#each Array.from({ length: lines }, (_, i) => i) as i (i)}
    <span class="sk" style:height style:width={width ?? widths[i % widths.length]} style:border-radius={radius}></span>
  {/each}
</div>

<style>
  .sk-group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .sk {
    display: block;
    background: linear-gradient(90deg, var(--skeleton) 0%, var(--skeleton-shine) 40%, var(--skeleton) 80%);
    background-size: 300% 100%;
    animation: shine 1.6s var(--ease-in-out) infinite;
  }
  @keyframes shine {
    from {
      background-position: 100% 0;
    }
    to {
      background-position: -100% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .sk {
      animation: none;
    }
  }
</style>
