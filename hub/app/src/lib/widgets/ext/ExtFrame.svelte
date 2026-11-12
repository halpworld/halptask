<script lang="ts">
  // A Tier C widget (docs/hub/P5-EXTENSIONS.md §6.3, §6.4): the pack's page
  // in a sandboxed iframe (`allow-scripts` only, so its origin is opaque and
  // it has no Tauri IPC), inside host chrome that names the pack and its
  // trust. All talk goes over one MessagePort through `ExtBridge`; window
  // messages carry no authority. Any load after the first tears it down.
  import { onDestroy } from 'svelte';
  import Puzzle from '@lucide/svelte/icons/puzzle';
  import type { WidgetProps } from '../types';
  import EmptyState from '../../design/EmptyState.svelte';
  import { hub } from '../../stores/hub.svelte';
  import { packs } from '../../stores/packs.svelte';
  import { toasts } from '../../design/toast.svelte';
  import { ExtBridge, LoadGuard, MAX_HANDSHAKE_RETRIES, isFrameSrc, isHandshakeRetry, parseExtKind, safeTokens } from './bridge';
  import { extOpenItemMessage } from '../../settings/packs';
  import { extCall, extWidget, type WidgetAnswer } from './host';
  import ExtUnavailable from './ExtUnavailable.svelte';

  let { widget, ambient }: WidgetProps = $props();

  const k = $derived(parseExtKind(widget.kind));
  let answer = $state<WidgetAnswer | null>(null);
  let tornDown = $state(false);
  let frame = $state<HTMLIFrameElement | null>(null);
  let wrap = $state<HTMLDivElement | null>(null);
  let wantHeight = $state<number | null>(null);

  let port: MessagePort | null = null;
  let bridge: ExtBridge | null = null;
  let guard = new LoadGuard();
  let retries = 0;
  let dataTimer: ReturnType<typeof setTimeout> | null = null;

  // Pack metadata for the chrome (untrusted text; rendered as text only).
  let asked = false;
  const pack = $derived(k ? packs.packs.find((p) => p.id === k.packId) : undefined);
  $effect(() => {
    if (k && !pack && !asked && hub.status === 'ready' && hub.transport === 'tauri') {
      asked = true;
      void packs.load();
    }
  });

  // Look the widget up (and get a one-time ticket for its page) once per kind.
  $effect(() => {
    const kind = k;
    teardownPort();
    answer = null;
    tornDown = false;
    guard = new LoadGuard();
    retries = 0;
    if (!kind) return;
    let live = true;
    void extWidget(kind.packId, kind.widget).then((a) => {
      if (!live) return;
      // Never mount a src that is not this widget's own halpext URL.
      if (a.state === 'ready' && (a.kind !== kind.kind || !isFrameSrc(a.src, kind))) answer = { state: 'unavailable' };
      else answer = a;
    });
    return () => {
      live = false;
    };
  });

  const ready = $derived(answer?.state === 'ready' ? answer : null);
  const trust = $derived(ready?.trust ?? pack?.trust ?? 'unknown');
  const publisher = $derived(pack?.publisher || pack?.title || k?.packId || '');

  function context() {
    const css = typeof getComputedStyle === 'function' ? getComputedStyle(document.documentElement) : null;
    return {
      theme: hub.resolvedTheme,
      tokens: css ? safeTokens((n) => css.getPropertyValue(n)) : {},
      ambient,
      privacy_blur: hub.privacy,
      config: widget.config ?? {},
    };
  }

  function teardownPort() {
    bridge?.close();
    bridge = null;
    if (port) {
      port.onmessage = null;
      port.close();
    }
    port = null;
  }

  function teardown(reason: string) {
    teardownPort();
    tornDown = true;
    if (frame) frame.src = 'about:blank';
    console.warn(`halp: custom widget ${widget.kind} torn down: ${reason}`);
  }

  /** Opens a fresh port to the frame and sends `ui/initialize` with it. */
  function handshake() {
    const kind = k;
    const r = ready;
    const win = frame?.contentWindow;
    if (!kind || !r || !win) return;
    teardownPort();
    const ch = new MessageChannel();
    const b = new ExtBridge(
      {
        call: (name, args) => extCall(kind.packId, kind.widget, name, args),
        openItem: (id) => openItem(id),
        sizeChanged: (h) => (wantHeight = h),
        now: () => performance.now(),
      },
      r.calls,
    );
    ch.port1.onmessage = (e: MessageEvent) => {
      void b.handle(e.data).then((res) => {
        if (res && port === ch.port1) ch.port1.postMessage(res);
      });
    };
    port = ch.port1;
    bridge = b;
    // Target '*': the frame's origin is opaque. The message carries no data
    // beyond what the widget is given anyway.
    win.postMessage({ jsonrpc: '2.0', method: 'ui/initialize', params: context() }, '*', [ch.port2]);
  }

  function onLoad() {
    if (tornDown) return;
    if (guard.onLoad() === 'teardown') {
      teardown('the frame navigated');
      return;
    }
    handshake();
  }

  function onWindowMessage(e: MessageEvent) {
    // Only a handshake retry from this very frame; everything else is ignored.
    if (tornDown || !isHandshakeRetry(e, frame?.contentWindow)) return;
    if (retries >= MAX_HANDSHAKE_RETRIES) return;
    retries++;
    handshake();
  }

  // The title is never shown: the lookup runs as the user, so it would let a
  // widget surface a private item's title (or one under privacy blur).
  function openItem(_id: string) {
    toasts.show(extOpenItemMessage(publisher), { timeout: 4000 });
  }

  // Host context and data changes go over the port only.
  $effect(() => {
    const msg = { jsonrpc: '2.0', method: 'ui/notifications/host-context-changed', params: { theme: hub.resolvedTheme, ambient, privacy_blur: hub.privacy } };
    port?.postMessage(msg);
  });
  $effect(() => {
    void hub.itemsRev;
    if (dataTimer) clearTimeout(dataTimer);
    dataTimer = setTimeout(() => port?.postMessage({ jsonrpc: '2.0', method: 'ui/notifications/data-changed', params: { entities: ['items'] } }), 400);
  });

  const height = $derived(wantHeight === null || !wrap ? null : Math.max(48, Math.min(wantHeight, wrap.clientHeight)));

  onDestroy(() => {
    teardownPort();
    if (dataTimer) clearTimeout(dataTimer);
  });
</script>

<svelte:window onmessage={onWindowMessage} />

{#if !k}
  <EmptyState compact icon={Puzzle} title="Unknown widget" hint="No renderer for “{widget.kind}”." />
{:else if answer === null}
  <div class="loading" aria-busy="true"></div>
{:else if answer.state !== 'ready'}
  <ExtUnavailable reason={answer.state} kind={widget.kind} />
{:else if tornDown}
  <EmptyState compact icon={Puzzle} title="Widget stopped" hint="This custom widget tried to load another page, so it was closed." />
{:else}
  <div class="ext" class:dev={trust === 'dev'} class:blurred={hub.privacy} bind:this={wrap}>
    <div class="chrome" title="This widget is from {publisher}. It runs in a sandbox and can only read what an agent could.">
      <span class="pub">{publisher}</span>
      <span class="trust trust-{trust}">{trust === 'dev' ? 'DEV' : trust.replace('_', ' ')}</span>
    </div>
    <!-- svelte-ignore a11y_missing_attribute -->
    <iframe
      bind:this={frame}
      title="Custom widget {widget.kind}"
      sandbox="allow-scripts"
      allow=""
      referrerpolicy="no-referrer"
      loading="eager"
      src={ready?.src}
      style:height={height === null ? undefined : `${height}px`}
      onload={onLoad}
    ></iframe>
  </div>
{/if}

<style>
  .loading {
    height: 100%;
    min-height: 48px;
  }
  .ext {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-height: 0;
  }
  .chrome {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: var(--text-2xs);
    color: var(--text-3);
    overflow: hidden;
  }
  .pub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trust {
    flex: none;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 4px;
  }
  iframe {
    flex: 1;
    width: 100%;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
  }
  .dev iframe {
    border: 3px solid transparent;
    border-image: repeating-linear-gradient(45deg, #d97706 0 8px, #1a1b1e 8px 16px) 3;
  }
  .trust-dev {
    color: #b45309;
    border-color: #d97706;
  }
  /* The host blurs the frame itself; it does not rely on the widget. */
  .blurred iframe {
    filter: blur(8px);
    pointer-events: none;
  }
</style>
