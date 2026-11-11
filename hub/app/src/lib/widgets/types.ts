import type { Component } from 'svelte';
import type { JsonSchema, Widget } from '../api/types';
import type { IconComponent } from '../design/icon';

export interface WidgetProps {
  widget: Widget;
  /** True in ambient (wall) mode: larger type, no editing affordances. */
  ambient: boolean;
}

export type WidgetComponent = Component<WidgetProps>;

export interface WidgetEntry {
  kind: string;
  title: string;
  icon: IconComponent;
  component: WidgetComponent;
  /** Hidden in ambient mode (interactive-only widgets like quick capture). */
  ambient: 'show' | 'hide';
  minW: number;
  minH: number;
  /** JSON Schema for `widget.config`, when the kind takes any. */
  config?: JsonSchema;
  /** Placeholder kinds render sample data until their phase lands. */
  placeholder?: { phase: string; note: string };
}
