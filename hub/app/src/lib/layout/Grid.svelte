<script lang="ts">
  import { scale, fade } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import GripVertical from '@lucide/svelte/icons/grip-vertical';
  import X from '@lucide/svelte/icons/x';
  import FlaskConical from '@lucide/svelte/icons/flask-conical';
  import Sparkles from '@lucide/svelte/icons/sparkles';
  import CircleHelp from '@lucide/svelte/icons/circle-help';
  import type { Layout, Widget } from '../api/types';
  import { boxToRect, bottom, compact, metricsFor, moveBox, pxToCells, resizeBox, stack, type GridMetrics } from './grid';
  import { resolveWidget } from '../widgets/registry';
  import { fillGuide } from '../widgets/fill';
  import FillContext from '../widgets/FillContext.svelte';
  import FillGuide from '../widgets/FillGuide.svelte';
  import Panel from '../design/Panel.svelte';
  import Badge from '../design/Badge.svelte';
  import IconButton from '../design/IconButton.svelte';
  import { motion } from '../design/motion.svelte';

  interface Props {
    layout: Layout;
    editing: boolean;
    ambient: boolean;
    /** Widget ids changed by someone else (agent, module) → expiry ms. */
    touched: Record<string, number>;
    onmove: (id: string, x: number, y: number) => void;
    onresize: (id: string, w: number, h: number) => void;
    onremove: (id: string) => void;
  }

  let { layout, editing, ambient, touched, onmove, onresize, onremove }: Props = $props();

  const STACK_BELOW = 760;

  let width = $state(0);
  // Geometry transitions only after the first placement, so widgets don't
  // fly in from the top-left corner on load.
  let settled = $state(false);
  $effect(() => {
    if (width > 0 && !settled) requestAnimationFrame(() => requestAnimationFrame(() => (settled = true)));
  });
  let height = $state(0);

  // In-progress pointer gesture. `preview` is what the grid would look like
  // if released now; the dragged widget itself follows the pointer.
  interface Gesture {
    id: string;
    mode: 'move' | 'resize';
    startX: number;
    startY: number;
    dx: number;
    dy: number;
    origin: Widget;
    pointerId: number;
  }
  let gesture = $state<Gesture | null>(null);

  /** The card whose "how to fill" guide is open, if any. */
  let helpFor = $state<string | null>(null);

  const stacked = $derived(width > 0 && width < STACK_BELOW);
  const columns = $derived(layout.columns || 12);

  const visible = $derived.by<Widget[]>(() => {
    let ws = layout.widgets;
    if (ambient) ws = compact(ws.filter((w) => resolveWidget(w.kind).ambient === 'show'));
    return ws;
  });

  const preview = $derived.by<Widget[]>(() => {
    const g = gesture;
    if (!g) return visible;
    const m = metrics;
    const cells = pxToCells(g.dx, g.dy, m);
    if (g.mode === 'move') return moveBox(visible, g.id, g.origin.x + cells.dx, g.origin.y + cells.dy, columns);
    return resizeBox(visible, g.id, g.origin.w + cells.dx, g.origin.h + cells.dy, columns);
  });

  const placed = $derived(stacked ? stack(preview, columns) : preview);
  const rows = $derived(Math.max(1, bottom(placed)));
  const gap = $derived(ambient ? 20 : 14);
  const metrics = $derived<GridMetrics>(
    metricsFor(width, height, stacked ? Math.min(rows, 6) : rows, { columns, gap, minRow: stacked ? 64 : 44, maxRow: ambient ? 180 : 140 }),
  );
  const totalHeight = $derived(rows * metrics.rowHeight + (rows - 1) * metrics.gap);

  function rectFor(w: Widget) {
    const r = boxToRect(w, metrics);
    const g = gesture;
    if (g && g.id === w.id) {
      const o = boxToRect(g.origin, metrics);
      if (g.mode === 'move') return { ...o, left: o.left + g.dx, top: o.top + g.dy };
      return { ...o, width: Math.max(80, o.width + g.dx), height: Math.max(60, o.height + g.dy) };
    }
    return r;
  }

  function begin(e: PointerEvent, w: Widget, mode: Gesture['mode']) {
    if (!editing || stacked || e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const origin = layout.widgets.find((x) => x.id === w.id) ?? w;
    gesture = { id: w.id, mode, startX: e.clientX, startY: e.clientY, dx: 0, dy: 0, origin, pointerId: e.pointerId };
  }

  function track(e: PointerEvent) {
    if (!gesture || e.pointerId !== gesture.pointerId) return;
    gesture = { ...gesture, dx: e.clientX - gesture.startX, dy: e.clientY - gesture.startY };
  }

  function end(e: PointerEvent) {
    const g = gesture;
    if (!g || e.pointerId !== g.pointerId) return;
    gesture = null;
    const cells = pxToCells(g.dx, g.dy, metrics);
    if (!cells.dx && !cells.dy) return;
    if (g.mode === 'move') onmove(g.id, Math.max(0, g.origin.x + cells.dx), Math.max(0, g.origin.y + cells.dy));
    else onresize(g.id, g.origin.w + cells.dx, g.origin.h + cells.dy);
  }

  function cancel() {
    gesture = null;
  }

  /** Arrow keys move, Shift+arrows resize, Delete removes (edit mode only). */
  function onFrameKey(e: KeyboardEvent, w: Widget) {
    if (!editing || e.target !== e.currentTarget) return;
    const d: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const step = d[e.key];
    if (step) {
      e.preventDefault();
      if (e.shiftKey) onresize(w.id, w.w + step[0], w.h + step[1]);
      else onmove(w.id, Math.max(0, w.x + step[0]), Math.max(0, w.y + step[1]));
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      onremove(w.id);
    }
  }

  const inT = (i: number) => ({ start: 0.965, opacity: 0, duration: motion.ms(320), delay: motion.ms(Math.min(i, 10) * 28), easing: cubicOut });
  const outT = () => ({ duration: motion.ms(140) });
</script>

<div class="grid-viewport" class:editing class:stacked bind:clientWidth={width} bind:clientHeight={height}>
  <div class="grid" style:height="{totalHeight}px" class:dragging={!!gesture} class:settled>
    {#if editing && !stacked && width > 0}
      <div class="guides" aria-hidden="true">
        {#each Array.from({ length: columns }, (_, i) => i) as i (i)}
          <span class="guide" style:left="{i * (metrics.colWidth + metrics.gap)}px" style:width="{metrics.colWidth}px"></span>
        {/each}
      </div>
    {/if}

    {#if width > 0}
      {#each placed as w, i (w.id)}
        {@const entry = resolveWidget(w.kind)}
        {@const r = rectFor(w)}
        {@const C = entry.component}
        {@const active = gesture?.id === w.id}
        {@const isTouched = (touched[w.id] ?? 0) > 0}
        {@const label = w.title || entry.title}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div
          class="cell"
          class:active
          class:touched={isTouched}
          class:placeholder={!!entry.placeholder}
          role="group"
          aria-label={editing ? `${label}. Arrow keys move, Shift plus arrows resize, Delete removes.` : label}
          data-widget={w.id}
          data-kind={w.kind}
          tabindex={editing ? 0 : -1}
          style:left="{r.left}px"
          style:top="{r.top}px"
          style:width="{r.width}px"
          style:height="{r.height}px"
          onkeydown={(e) => onFrameKey(e, w)}
          in:scale={inT(i)}
          out:fade={outT()}
        >
          <Panel title={label} icon={entry.icon} tone={w.kind === 'now' ? 'accent' : 'default'} class="widget-panel">
            {#snippet lead()}
              {#if editing && !stacked}
                <span
                  class="handle"
                  role="presentation"
                  title="Drag to move"
                  onpointerdown={(e) => begin(e, w, 'move')}
                  onpointermove={track}
                  onpointerup={end}
                  onpointercancel={cancel}
                >
                  <GripVertical size={14} aria-hidden="true" />
                </span>
              {/if}
            {/snippet}
            {#snippet actions()}
              {#if isTouched}
                <Badge tone="accent" icon={Sparkles} title="Changed by an agent just now">moved</Badge>
              {/if}
              {#if entry.placeholder}
                <Badge tone="warning" variant="outline" icon={FlaskConical} title={entry.placeholder.note}>Sample data · {entry.placeholder.phase}</Badge>
              {/if}
              {#if !ambient && !editing && fillGuide(w.kind)}
                <IconButton
                  size="sm"
                  icon={helpFor === w.id ? X : CircleHelp}
                  label={helpFor === w.id ? `Close how to fill ${label}` : `How to fill ${label}`}
                  aria-expanded={helpFor === w.id}
                  onclick={() => (helpFor = helpFor === w.id ? null : w.id)}
                />
              {/if}
              {#if editing}
                <IconButton size="sm" icon={X} label="Remove {label}" onclick={() => onremove(w.id)} />
              {/if}
            {/snippet}
            {@const helping = helpFor === w.id && !ambient && !editing && !!fillGuide(w.kind)}
            {#if helping}
              <div class="fill-help"><FillGuide kind={w.kind} full /></div>
            {/if}
            <!-- Kept mounted while the guide shows, so typed state survives. -->
            <div class="widget-body" class:hidden={helping}>
              <FillContext kind={!ambient && !editing ? w.kind : ''}><C widget={w} {ambient} /></FillContext>
            </div>
          </Panel>
          {#if editing && !stacked}
            <span
              class="resize"
              role="presentation"
              title="Drag to resize"
              onpointerdown={(e) => begin(e, w, 'resize')}
              onpointermove={track}
              onpointerup={end}
              onpointercancel={cancel}
            ></span>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .fill-help {
    height: 100%;
    overflow: auto;
  }
  .widget-body {
    display: contents;
  }
  .widget-body.hidden {
    display: none;
  }
  .grid-viewport {
    position: relative;
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 2px 2px 24px;
    scrollbar-gutter: stable;
  }
  .grid {
    position: relative;
    width: 100%;
  }
  .guides {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .guide {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--accent) 5%, transparent);
    outline: 1px dashed color-mix(in srgb, var(--accent) 22%, transparent);
  }

  .cell {
    position: absolute;
    border-radius: var(--radius-lg);
  }
  .settled .cell {
    transition:
      left var(--dur-layout) var(--ease-spring),
      top var(--dur-layout) var(--ease-spring),
      width var(--dur-layout) var(--ease-spring),
      height var(--dur-layout) var(--ease-spring),
      box-shadow var(--dur-base) var(--ease-out);
  }
  .cell:focus-visible {
    box-shadow: var(--focus-ring);
  }
  .cell.active {
    z-index: var(--z-widget-drag);
    transition: box-shadow var(--dur-base) var(--ease-out);
    box-shadow: var(--elev-4);
    opacity: 0.94;
  }
  .grid.dragging .cell:not(.active) {
    transition-duration: 200ms;
  }
  .grid.dragging {
    user-select: none;
    cursor: grabbing;
  }

  /* Agent-changed widgets glow briefly in the role accent. */
  .cell.touched::after {
    content: '';
    position: absolute;
    inset: -3px;
    border-radius: calc(var(--radius-lg) + 3px);
    pointer-events: none;
    box-shadow:
      0 0 0 2px var(--accent),
      0 0 24px 4px var(--accent-glow);
    animation: touched 2.4s var(--ease-out) forwards;
  }
  @keyframes touched {
    0% {
      opacity: 0;
    }
    12% {
      opacity: 1;
    }
    70% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cell.touched::after {
      animation: none;
      opacity: 1;
    }
  }

  .editing .cell :global(.widget-panel) {
    border-style: dashed;
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .cell.placeholder :global(.widget-panel) {
    background-image: repeating-linear-gradient(
      135deg,
      transparent 0 14px,
      color-mix(in srgb, var(--warning) 3.5%, transparent) 14px 15px
    );
  }

  .handle {
    display: inline-grid;
    place-items: center;
    width: 20px;
    height: 24px;
    margin-left: -6px;
    border-radius: var(--radius-sm);
    color: var(--text-3);
    cursor: grab;
    touch-action: none;
  }
  .handle:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .resize {
    position: absolute;
    right: 2px;
    bottom: 2px;
    width: 18px;
    height: 18px;
    cursor: nwse-resize;
    touch-action: none;
    border-right: 2px solid var(--accent);
    border-bottom: 2px solid var(--accent);
    border-bottom-right-radius: var(--radius-md);
    opacity: 0.7;
  }
  .resize:hover {
    opacity: 1;
  }
</style>
