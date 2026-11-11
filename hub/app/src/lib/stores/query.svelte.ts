import { untrack } from 'svelte';

export interface Query<T> {
  readonly data: T | undefined;
  readonly error: string | null;
  readonly loading: boolean;
}

/**
 * Run `fetcher` now and again whenever `deps()` changes. Only `deps` is
 * tracked, so reading state inside `fetcher` never causes surprise refetches.
 * Must be called during component initialisation.
 */
export function query<T>(fetcher: () => Promise<T>, deps: () => unknown): Query<T> {
  let data = $state<T | undefined>(undefined);
  let error = $state<string | null>(null);
  let loading = $state(true);

  $effect(() => {
    deps();
    let cancelled = false;
    untrack(() => {
      fetcher().then(
        (d) => {
          if (cancelled) return;
          data = d;
          error = null;
          loading = false;
        },
        (e: unknown) => {
          if (cancelled) return;
          error = e instanceof Error ? e.message : String(e);
          loading = false;
        },
      );
    });
    return () => {
      cancelled = true;
    };
  });

  return {
    get data() {
      return data;
    },
    get error() {
      return error;
    },
    get loading() {
      return loading;
    },
  };
}
