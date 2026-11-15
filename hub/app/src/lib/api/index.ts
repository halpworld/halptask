import { HubClient } from './client';
import { isTauri, type Transport } from './transport';

export * from './types';
export { HubClient, HubApiError, type ConsentHandler, type ConsentOutcome } from './client';
export type { Transport } from './transport';

/**
 * Picks the transport at runtime. Both are loaded lazily so the mock (and its
 * seed data) is split into its own chunk that a Tauri window never fetches.
 */
export async function createClient(): Promise<HubClient> {
  let transport: Transport;
  if (isTauri()) {
    const { TauriTransport } = await import('./tauri');
    transport = new TauriTransport();
  } else {
    const { MockTransport } = await import('./mock/mock');
    const mock = new MockTransport();
    transport = mock;
    (globalThis as { __halpMock?: unknown }).__halpMock = mock;
  }
  return new HubClient(transport);
}
