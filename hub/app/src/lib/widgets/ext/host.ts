// The two shell commands behind Tier C frames (src-tauri/src/ext). Both
// answer the main window only; a frame never reaches them (it has no IPC).
// In a browser (`pnpm dev`, the mock) there is no halpext scheme.

import { isTauri } from '../../api/transport';
import type { HubResponse } from '../../api/types';
import { isWindowsUA, type HostCall } from './bridge';

export type WidgetAnswer =
  | { state: 'ready'; kind: string; src: string; calls: string[]; trust: string }
  | { state: 'unsupported' }
  | { state: 'locked' }
  | { state: 'unavailable' }
  /** Not the desktop app (browser preview / mock). */
  | { state: 'browser' };

export async function extWidget(packId: string, widget: string): Promise<WidgetAnswer> {
  if (!isTauri()) return { state: 'browser' };
  if (typeof navigator !== 'undefined' && isWindowsUA(navigator.userAgent)) return { state: 'unsupported' };
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const a = await invoke<WidgetAnswer>('ext_widget', { packId, widget });
    return a && typeof a === 'object' && typeof a.state === 'string' ? a : { state: 'unavailable' };
  } catch {
    return { state: 'unavailable' };
  }
}

export async function extCall(packId: string, widget: string, name: HostCall, args: Record<string, unknown>): Promise<HubResponse<unknown>> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<HubResponse<unknown>>('ext_call', { packId, widget, name, arguments: args });
  } catch (err) {
    return { ok: false, error: { code: 'internal', message: String(err) } };
  }
}
