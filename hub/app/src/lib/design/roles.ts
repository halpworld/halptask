import Terminal from '@lucide/svelte/icons/square-terminal';
import Users from '@lucide/svelte/icons/users';
import Leaf from '@lucide/svelte/icons/leaf';
import type { IconComponent } from './icon';

export interface RoleMeta {
  icon: IconComponent;
  short: string;
  tagline: string;
}

/** Frontend presentation for first-party roles. Unknown roles fall back. */
export const ROLE_META: Record<string, RoleMeta> = {
  developer: { icon: Terminal, short: 'Dev', tagline: 'The Cockpit' },
  'eng-manager': { icon: Users, short: 'EM', tagline: 'The Command Deck' },
  cso: { icon: Leaf, short: 'CSO', tagline: 'The Sustainability Wall' },
};

export function roleMeta(id: string | undefined): RoleMeta {
  return (id && ROLE_META[id]) || { icon: Terminal, short: id ?? '?', tagline: '' };
}

const KNOWN = new Set(Object.keys(ROLE_META));
export function isKnownRole(id: string): boolean {
  return KNOWN.has(id);
}
