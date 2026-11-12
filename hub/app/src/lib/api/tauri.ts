import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { Transport } from './transport';
import type { HubEvent, HubRequest, HubResponse } from './types';

export const EVENT_CHANNEL = 'halp://event';

export class TauriTransport implements Transport {
  readonly kind = 'tauri' as const;

  async send(request: HubRequest): Promise<HubResponse> {
    try {
      return await invoke<HubResponse>('dispatch', { request });
    } catch (err) {
      // invoke rejects on IPC failure (command missing, ACL denied, serialisation).
      return { ok: false, error: { code: 'internal', message: String(err) } };
    }
  }

  subscribe(listener: (event: HubEvent) => void): () => void {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    listen<HubEvent>(EVENT_CHANNEL, (e) => listener(e.payload)).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }
}
