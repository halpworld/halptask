<script lang="ts" module>
  /** Context key: the kind of the card an EmptyState is rendered in. */
  export const FILL_KIND = Symbol('fill-kind');
</script>

<script lang="ts">
  import { setContext, type Snippet } from 'svelte';

  interface Props {
    kind: string;
    children: Snippet;
  }
  let { kind, children }: Props = $props();
  // A getter, so a card whose kind changes keeps the right guide.
  setContext(FILL_KIND, () => kind);
</script>

{@render children()}
