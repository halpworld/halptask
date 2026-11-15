import type { Component } from 'svelte';

/** Structural type for bundled lucide icons (and any compatible SVG component). */
export type IconComponent = Component<{
  size?: number | string;
  strokeWidth?: number | string;
  class?: string;
  'aria-hidden'?: boolean | 'true' | 'false';
}>;
