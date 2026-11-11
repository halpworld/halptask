import type { HubEvent, HubRequest, HubResponse } from './types';

/** One interface, two implementations: Tauri IPC and an in-memory mock. */
export interface Transport {
  readonly kind: 'tauri' | 'mock';
  send(request: HubRequest): Promise<HubResponse>;
  /** Subscribe to change events. Returns an unsubscribe function. */
  subscribe(listener: (event: HubEvent) => void): () => void;
}

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
