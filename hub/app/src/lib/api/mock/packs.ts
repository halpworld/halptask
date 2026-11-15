// Browser stand-in for the packs.* commands (P5-B, docs/hub/P5-EXTENSIONS.md
// §3.4, §4, §7). It follows the rules the UI can observe: preview binds a
// preview_id to the bundle's SHA-256 and lives 10 minutes, install installs
// exactly the previewed bytes, an update that adds capabilities shows a
// permission diff, a downgrade or same version is refused, a bad signature is
// an error (never unsigned), key pinning, uninstall is blocked by dependants,
// revoking a key disables its packs, built-in packs cannot be disabled, and
// agents see list rows without the user-only fields (CONTRACT v1.10). Cryptography is not simulated: the "signature" is a
// key_id plus a sig string, and `sig: "BAD"` fails like a tampered bundle.
//
// A `.halppack` the mock accepts is the real container (§3.2):
//   { format: "halp-pack/1", manifest: "<json text>", signature: {alg, key_id, sig}|null, files: {} }
// and only these manifest fields are read: id, version, title, description,
// license, publisher.name, extends.id, role, task_types[], modules[],
// ui_widgets[], prompts.add[], migrations[], files. `sampleBundle` builds one.

import type { Actor, Args, EntityRef, ErrorCode, JsonSchema, PackKey, PackPreview, PackTrust, PackView, PermissionChange, PermissionDiff, Tier } from '../types';

export interface PackCommandDef {
  tier: Tier;
  description: string;
  input: JsonSchema;
  user_only?: boolean;
  run(args: Args, ctx: { actor: Actor }): unknown;
}

export interface PacksHost {
  now: () => Date;
  fail: (code: ErrorCode, message: string) => Error;
  event: (ctx: { actor: Actor }, command: string, entity: EntityRef, summary: string) => void;
}

/** Previews live this long (§3.4). */
export const PREVIEW_TTL_MS = 10 * 60_000;

/** Registered as a first-party key, so a bundle signed with it reads as HalpTask. */
export const FIRST_PARTY_KEY_ID = 'f1a2b3c4d5e6f708';
/** Trusted by the seed data: a community publisher. */
export const ACME_KEY_ID = 'acac0000acac0001';
/** A key nobody has trusted yet: a bundle signed with it shows the trust checkbox. */
export const UNKNOWN_KEY_ID = '5eed5eed5eed5eed';

interface Rec {
  view: PackView;
  perms: PermissionChange[];
  /** The version before the last update, for rollback. */
  prev: { view: PackView; perms: PermissionChange[] } | null;
}

const S = {
  str: (): JsonSchema => ({ type: 'string' }),
  bool: (): JsonSchema => ({ type: ['boolean', 'null'] }),
  nstr: (): JsonSchema => ({ type: ['string', 'null'] }),
  obj: (properties: Record<string, JsonSchema> = {}, required: string[] = []): JsonSchema => ({ type: 'object', properties, required, additionalProperties: false }),
};

/** A deterministic 64-hex stand-in for SHA-256 (the mock has no sync digest). */
export function fakeDigest(seed: string): string {
  let out = '';
  let h = 0x811c9dc5;
  for (let round = 0; out.length < 64; round++) {
    for (let i = 0; i < seed.length; i++) {
      h ^= seed.charCodeAt(i) + round;
      h = Math.imul(h, 0x01000193) >>> 0;
    }
    out += h.toString(16).padStart(8, '0');
  }
  return out.slice(0, 64);
}


const K256 = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

/** Synchronous SHA-256 (hex), so the mock's bundle_sha256 is the real digest of the file's bytes. */
export function sha256Sync(data: Uint8Array): string {
  const h = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
  const len = data.length;
  const padded = new Uint8Array((((len + 9 + 63) >> 6) << 6));
  padded.set(data);
  padded[len] = 0x80;
  const dv = new DataView(padded.buffer);
  dv.setUint32(padded.length - 8, Math.floor((len * 8) / 0x100000000));
  dv.setUint32(padded.length - 4, (len * 8) >>> 0);
  const w = new Uint32Array(64);
  const rotr = (x: number, n: number) => (x >>> n) | (x << (32 - n));
  for (let o = 0; o < padded.length; o += 64) {
    for (let i = 0; i < 16; i++) w[i] = dv.getUint32(o + i * 4);
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(w[i - 15]!, 7) ^ rotr(w[i - 15]!, 18) ^ (w[i - 15]! >>> 3);
      const s1 = rotr(w[i - 2]!, 17) ^ rotr(w[i - 2]!, 19) ^ (w[i - 2]! >>> 10);
      w[i] = (w[i - 16]! + s0 + w[i - 7]! + s1) >>> 0;
    }
    let [a, b, c, d, e, f, g, hh] = h as unknown as number[];
    for (let i = 0; i < 64; i++) {
      const t1 = (hh! + (rotr(e!, 6) ^ rotr(e!, 11) ^ rotr(e!, 25)) + ((e! & f!) ^ (~e! & g!)) + K256[i]! + w[i]!) >>> 0;
      const t2 = ((rotr(a!, 2) ^ rotr(a!, 13) ^ rotr(a!, 22)) + ((a! & b!) ^ (a! & c!) ^ (b! & c!))) >>> 0;
      hh = g; g = f; f = e; e = (d! + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    [a, b, c, d, e, f, g, hh].forEach((v, i) => (h[i] = (h[i]! + v!) >>> 0));
  }
  return Array.from(h, (x) => x.toString(16).padStart(8, '0')).join('');
}

/** A fingerprint whose first 16 hex are `keyId`, like the real key_id (§4.1). */
const fingerprintFor = (keyId: string): string => keyId + fakeDigest(keyId).slice(0, 48);

const b64 = (text: string): string => btoa(unescape(encodeURIComponent(text)));
const unb64 = (s: string): string => decodeURIComponent(escape(atob(s)));

export interface SampleManifest {
  id: string;
  version: string;
  title?: string;
  description?: string;
  license?: string;
  publisher?: { name: string };
  extends?: { id: string; version?: string };
  role?: { id: string; title: string; accent: string };
  task_types?: { id: string; title?: string; schema?: Record<string, unknown> }[];
  modules?: { id: string; title?: string; inputs?: string[] }[];
  ui_widgets?: { kind: string; title?: string; calls?: string[] }[];
  prompts?: { add?: string[] };
  migrations?: { type: string; from: string; ops: unknown[] }[];
  files?: Record<string, { sha256: string; size: number }>;
}

/** A base64 `.halppack` the mock accepts. `signature`: a key id, 'bad', or null for unsigned. */
export function sampleBundle(manifest: SampleManifest, signature: string | 'bad' | null = null): string {
  const sig = signature === null ? null : { alg: 'ed25519', key_id: signature === 'bad' ? ACME_KEY_ID : signature, sig: signature === 'bad' ? 'BAD' : 'c2ln' };
  return b64(JSON.stringify({ format: 'halp-pack/1', manifest: JSON.stringify({ format: 1, license: 'MIT', ...manifest }), signature: sig, files: {} }));
}

/** The community pack's next version: adds a module, a widget call, an annotation and a prompt. */
export const ACME_SRE_V2: SampleManifest = {
  id: 'acme.sre',
  version: '1.3.0',
  title: 'Acme SRE on-call',
  description: 'On-call rota, incident follow-ups and a runbook prompt.',
  publisher: { name: 'Acme Platform Team' },
  extends: { id: 'halp.developer' },
  role: { id: 'sre', title: 'SRE', accent: '#d9822b' },
  task_types: [
    { id: 'acme.sre.incident', title: 'Incident', schema: { 'x-halp-data-class': 'acme.sre.incident', 'x-halp-immutable-when': { field: 'state', equals: 'closed' } } },
    { id: 'acme.sre.followup', title: 'Follow-up' },
  ],
  modules: [
    { id: 'pagerduty-import', title: 'PagerDuty import', inputs: ['integration:pagerduty'] },
    { id: 'severity-rollup', title: 'Severity rollup', inputs: ['acme.sre.incident'] },
  ],
  ui_widgets: [{ kind: 'x.acme.sre.oncall', title: 'On-call board', calls: ['rota.today', 'rota.swap'] }],
  prompts: { add: ['prompts/runbook.md', 'prompts/postmortem.md'] },
  migrations: [{ type: 'acme.sre.incident', from: '<1.3.0', ops: [{ op: 'rename', from: 'sev', to: 'severity' }] }],
  files: { 'prompts/runbook.md': { sha256: fakeDigest('runbook'), size: 1840 }, 'prompts/postmortem.md': { sha256: fakeDigest('postmortem'), size: 2210 }, 'widgets/oncall.html': { sha256: fakeDigest('oncall'), size: 9120 } },
};

/** A first install, unsigned: no capabilities. */
export const READING_LIST: SampleManifest = {
  id: 'local.reading-list',
  version: '0.3.0',
  title: 'Reading list',
  description: 'A tiny role for the articles you mean to read.',
  publisher: { name: 'Somebody on the internet' },
  role: { id: 'reader', title: 'Reader', accent: '#4f7cac' },
  task_types: [{ id: 'local.reading-list.article', title: 'Article' }],
  files: {},
};

const vnum = (v: string): number[] => v.split(/[.+-]/).slice(0, 3).map((n) => parseInt(n, 10) || 0);
export function compareVersions(a: string, b: string): number {
  const x = vnum(a);
  const y = vnum(b);
  for (let i = 0; i < 3; i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) < (y[i] ?? 0) ? -1 : 1;
  return 0;
}

const permKey = (c: PermissionChange) => `${c.kind}\u0000${c.id}`;

/** What a preview asks for, as flat permission rows. */
export function permissionsOf(p: Pick<PackPreview, 'modules' | 'ui_widgets' | 'annotations' | 'data_classes' | 'prompts'>): PermissionChange[] {
  return [
    ...p.modules.map((m) => ({ kind: 'module' as const, id: m.id, detail: m.inputs.length ? `reads ${m.inputs.join(', ')}` : undefined })),
    ...p.ui_widgets.flatMap((w) => w.calls.map((c) => ({ kind: 'ui_widget_call' as const, id: c, detail: w.kind }))),
    ...p.annotations.map((a) => ({ kind: 'annotation' as const, id: `${a.type_id}: ${a.annotation}` })),
    ...p.data_classes.map((d) => ({ kind: 'data_class' as const, id: d })),
    ...p.prompts.map((x) => ({ kind: 'prompt' as const, id: x.name })),
  ];
}

export function diffPermissions(before: PermissionChange[], after: PermissionChange[]): PermissionDiff {
  const b = new Set(before.map(permKey));
  const a = new Set(after.map(permKey));
  return { added: after.filter((c) => !b.has(permKey(c))), removed: before.filter((c) => !a.has(permKey(c))) };
}

const capsOf = (perms: PermissionChange[]) => ({
  modules: perms.filter((p) => p.kind === 'module').length,
  ui_widgets: new Set(perms.filter((p) => p.kind === 'ui_widget_call').map((p) => p.detail)).size,
  prompts: perms.filter((p) => p.kind === 'prompt').length,
  annotations: perms.filter((p) => p.kind === 'annotation').length,
});

interface Stored {
  report: PackPreview;
  expires: number;
}

export class MockPacks {
  private packs = new Map<string, Rec>();
  private keys = new Map<string, PackKey>();
  private previews = new Map<string, Stored>();
  private devPath: string | null = null;
  private n = 0;

  constructor(private readonly host: PacksHost) {
    this.reset();
  }

  reset(): void {
    const at = (d: number) => new Date(this.host.now().getTime() - d * 86_400_000).toISOString();
    this.previews.clear();
    this.devPath = null;
    this.keys = new Map(
      (
        [
          { key_id: FIRST_PARTY_KEY_ID, fingerprint: fingerprintFor(FIRST_PARTY_KEY_ID), label: 'HalpTask', first_party: true, revoked: false, added_at: null, packs: 0 },
          { key_id: ACME_KEY_ID, fingerprint: fingerprintFor(ACME_KEY_ID), label: 'Acme Platform Team', first_party: false, revoked: false, added_at: at(12), packs: 1 },
        ] satisfies PackKey[]
      ).map((k) => [k.key_id, k]),
    );
    const mk = (view: PackView, perms: PermissionChange[] = []): [string, Rec] => [view.id, { view, perms, prev: null }];
    const none = { modules: 0, ui_widgets: 0, prompts: 0, annotations: 0 };
    this.packs = new Map([
      mk({ id: 'halp.developer', version: '0.1.0', title: 'Developer', role_id: 'developer', trust: 'builtin', enabled: true, extends: [], key_fp: null, capabilities: { ...none, prompts: 2 }, status: 'ok', description: 'The built-in developer role: pull requests, CI and a review queue.', publisher: 'HalpTask', previous_version: null }, [
        { kind: 'prompt', id: 'triage' },
        { kind: 'prompt', id: 'standup' },
      ]),
      mk(
        { id: 'acme.sre', version: '1.2.0', title: 'Acme SRE on-call', role_id: 'sre', trust: 'community', enabled: true, extends: ['halp.developer'], key_fp: fingerprintFor(ACME_KEY_ID), capabilities: { modules: 1, ui_widgets: 1, prompts: 1, annotations: 0 }, status: 'ok', description: 'On-call rota and incident follow-ups.', publisher: 'Acme Platform Team', previous_version: null },
        [
          { kind: 'module', id: 'pagerduty-import', detail: 'reads integration:pagerduty' },
          { kind: 'ui_widget_call', id: 'rota.today', detail: 'x.acme.sre.oncall' },
          { kind: 'data_class', id: 'acme.sre.incident' },
          { kind: 'prompt', id: 'runbook' },
        ],
      ),
      mk({ id: 'local.reading-list', version: '0.3.0', title: 'Reading list', role_id: 'reader', trust: 'unsigned', enabled: false, extends: [], key_fp: null, capabilities: none, status: 'disabled', description: 'A tiny role for the articles you mean to read.', publisher: 'Somebody on the internet', previous_version: null }),
    ]);
  }

  private rec(id: string): Rec {
    const r = this.packs.get(id);
    if (!r) throw this.host.fail('not_found', `pack ${id} not found`);
    return r;
  }

  private view(r: Rec, actor?: Actor): PackView {
    const v = JSON.parse(JSON.stringify(r.view)) as PackView;
    // Agents get the list without the user-only fields.
    if (actor !== undefined && actor !== 'user') {
      delete v.description;
      delete v.publisher;
      delete v.previous_version;
      delete v.error;
    }
    return v;
  }

  private keyOf(keyId: string): PackKey | undefined {
    return this.keys.get(keyId);
  }

  private countKeys(): void {
    for (const k of this.keys.values()) k.packs = [...this.packs.values()].filter((r) => r.view.key_fp?.startsWith(k.key_id)).length;
  }

  // ---------------------------------------------------------------- preview

  private preview(bundleBase64: string): PackPreview {
    const bad = (m: string) => this.host.fail('invalid_args', m);
    let bundle: { format?: unknown; manifest?: unknown; signature?: { key_id?: unknown; sig?: unknown } | null };
    let m: SampleManifest;
    let text: string;
    try {
      text = unb64(bundleBase64);
      bundle = JSON.parse(text);
    } catch {
      throw bad('not a halp-pack/1 bundle: it is not base64 JSON');
    }
    if (bundle.format !== 'halp-pack/1' || typeof bundle.manifest !== 'string') throw bad('not a halp-pack/1 bundle');
    try {
      m = JSON.parse(bundle.manifest) as SampleManifest;
    } catch {
      throw bad('the manifest is not JSON');
    }
    if (typeof m.id !== 'string' || !/^[a-z][a-z0-9_.-]{0,63}$/.test(m.id) || typeof m.version !== 'string') throw bad('the manifest needs an id and a version');

    // Signature (§4.2 step 3): a bad one is an error, never unsigned.
    let trust: PackTrust = 'unsigned';
    let signature: PackPreview['signature'] = 'none';
    let keyFp: string | null = null;
    const sig = bundle.signature;
    if (sig) {
      const keyId = String(sig.key_id ?? '');
      if (sig.sig === 'BAD' || !/^[0-9a-f]{16}$/.test(keyId)) throw bad('the signature does not verify');
      keyFp = this.keyOf(keyId)?.fingerprint ?? fingerprintFor(keyId);
      const k = this.keyOf(keyId);
      if (k?.revoked) throw this.host.fail('denied', 'the signing key was revoked');
      if (k) {
        trust = k.first_party ? 'first_party' : 'community';
        signature = 'verified';
      } else signature = 'unknown_key';
    }

    const old = this.packs.get(m.id);
    if (old?.view.trust === 'builtin') throw this.host.fail('conflict', `${m.id} is a built-in pack`);
    // Key pinning (§4.2 step 5).
    if (old) {
      if (compareVersions(m.version, old.view.version) <= 0) throw this.host.fail('conflict', `${m.version} is not newer than the installed ${old.view.version}; use rollback instead`);
      if (old.view.key_fp && old.view.key_fp !== keyFp) throw this.host.fail('denied', keyFp ? 'signed by a different publisher than the installed version' : 'an unsigned update of a signed pack is refused');
    }

    const annotations: PackPreview['annotations'] = [];
    const dataClasses = new Set<string>();
    for (const t of m.task_types ?? []) {
      const s = t.schema ?? {};
      for (const a of ['x-halp-immutable-when', 'x-halp-ext-writable']) if (a in s) annotations.push({ type_id: t.id, annotation: a });
      if (typeof s['x-halp-data-class'] === 'string') dataClasses.add(s['x-halp-data-class']);
    }
    const warnings: string[] = [];
    // Annotations, modules and widgets need a trusted signature: unsigned
    // they are denied; by an unknown key the preview shows the trust the
    // pack gets once that key is trusted, and install needs trust_key.
    const privileged = annotations.length > 0 || !!m.modules?.length || !!m.ui_widgets?.length;
    if (trust === 'unsigned' && privileged) {
      if (signature !== 'unknown_key') throw this.host.fail('denied', 'an unsigned pack cannot use privileged annotations, modules or custom widgets');
      trust = 'community';
      warnings.push('It is signed by a key you have not trusted; installing it trusts that key (trust_key).');
    }
    if (!m.license) warnings.push('The manifest has no license.');
    const parent = m.extends ? this.packs.get(m.extends.id) : undefined;
    if (m.extends && !parent) warnings.push(`It extends ${m.extends.id}, which is not installed.`);

    const prompts = (m.prompts?.add ?? []).map((p) => ({ name: p.replace(/^.*\//, '').replace(/\.md$/, ''), description: `Prompt from ${p}` }));
    const report: PackPreview = {
      preview_id: `pv_${(++this.n).toString(36).padStart(4, '0')}_${fakeDigest(text).slice(0, 8)}`,
      bundle_sha256: sha256Sync(Uint8Array.from(atob(bundleBase64), (c) => c.charCodeAt(0))),
      expires_at: new Date(this.host.now().getTime() + PREVIEW_TTL_MS).toISOString(),
      id: m.id,
      version: m.version,
      title: m.title ?? m.id,
      description: m.description ?? '',
      publisher: m.publisher?.name ?? 'unknown publisher',
      trust,
      signature,
      key_fp: keyFp,
      installed_version: old?.view.version ?? null,
      files: Object.entries(m.files ?? {}).map(([path, f]) => ({ path, size: f.size })),
      chain: m.extends ? [m.extends.id, ...(parent?.view.extends ?? [])] : [],
      role: m.role ?? { id: m.id.split('.').pop() ?? 'role', title: m.title ?? m.id, accent: '#6b7280' },
      types: (m.task_types ?? []).map((t) => ({ id: t.id, title: t.title ?? t.id, data_class: (t.schema?.['x-halp-data-class'] as string | undefined) ?? null })),
      annotations,
      data_classes: [...dataClasses],
      modules: (m.modules ?? []).map((x) => ({ id: x.id, title: x.title ?? x.id, inputs: x.inputs ?? [] })),
      ui_widgets: (m.ui_widgets ?? []).map((w) => ({ kind: w.kind, title: w.title ?? w.kind, calls: w.calls ?? [] })),
      prompts,
      permission_diff: null,
      overrides: [],
      migrations: { types: new Set((m.migrations ?? []).map((x) => x.type)).size, items_checked: (m.migrations ?? []).length ? 14 : 0, items_migrated: (m.migrations ?? []).length ? 14 : 0, items_failed: 0, failed_ids: [] },
      warnings,
    };
    report.permission_diff = old ? diffPermissions(old.perms, permissionsOf(report)) : null;
    this.previews.set(report.preview_id, { report, expires: Date.parse(report.expires_at) });
    return JSON.parse(JSON.stringify(report)) as PackPreview;
  }

  // ---------------------------------------------------------------- install

  private install(previewId: string, trustKey: boolean, ctx: { actor: Actor }): PackView {
    const st = this.previews.get(previewId);
    if (!st) throw this.host.fail('not_found', 'unknown preview; choose the file again');
    if (st.expires <= this.host.now().getTime()) {
      this.previews.delete(previewId);
      throw this.host.fail('conflict', 'the preview expired; choose the file again');
    }
    const p = st.report;
    const old = this.packs.get(p.id);
    if (old && compareVersions(p.version, old.view.version) <= 0) throw this.host.fail('conflict', 'a downgrade is refused; use rollback');
    let trust: PackTrust = p.signature === 'unknown_key' ? 'unsigned' : p.trust;
    if (p.signature === 'unknown_key' && p.trust === 'community' && !trustKey) {
      throw this.host.fail('denied', 'it needs a trusted signature (it is signed by a key you have not trusted: install with trust_key)');
    }
    if (p.signature === 'unknown_key' && trustKey && p.key_fp) {
      const key_id = p.key_fp.slice(0, 16);
      this.keys.set(key_id, { key_id, fingerprint: p.key_fp, label: p.publisher, first_party: false, revoked: false, added_at: this.host.now().toISOString(), packs: 0 });
      trust = 'community';
      this.host.event(ctx, 'packs.keys_add', { kind: 'pack_key', id: key_id }, `Trusted publisher key ${p.publisher}`);
    }
    const perms = permissionsOf(p);
    const view: PackView = {
      id: p.id,
      version: p.version,
      title: p.title,
      role_id: p.role.id,
      trust,
      enabled: true,
      extends: p.chain,
      key_fp: trust === 'unsigned' ? null : p.key_fp,
      capabilities: capsOf(perms),
      status: 'ok',
      description: p.description,
      publisher: p.publisher,
      previous_version: old?.view.version ?? null,
    };
    this.packs.set(p.id, { view, perms, prev: old ? { view: old.view, perms: old.perms } : null });
    this.previews.delete(previewId);
    this.countKeys();
    this.host.event(ctx, 'packs.install', { kind: 'pack', id: p.id }, `${old ? 'Updated' : 'Installed'} ${p.title} ${p.version}`);
    return this.view(this.rec(p.id));
  }

  // --------------------------------------------------------------- commands

  commands(): Record<string, PackCommandDef> {
    const id = S.obj({ id: S.str() }, ['id']);
    return {
      'packs.list': {
        tier: 'T0',
        description: 'Installed extension packs.',
        input: S.obj(),
        run: (_a, ctx) => {
          this.countKeys();
          return { packs: [...this.packs.values()].map((r) => this.view(r, ctx.actor)) };
        },
      },
      'packs.get': {
        tier: 'T0',
        description: 'One installed pack.',
        input: id,
        run: (a, ctx) => {
          this.countKeys();
          return this.view(this.rec(a.id as string), ctx.actor);
        },
      },
      'packs.preview': {
        tier: 'T0',
        user_only: true,
        description: 'Verify a .halppack and report what installing it would do. Installs nothing.',
        input: S.obj({ bundle_base64: S.str() }, ['bundle_base64']),
        run: (a) => this.preview(a.bundle_base64 as string),
      },
      'packs.install': {
        tier: 'T3',
        user_only: true,
        description: 'Install or update exactly the previewed bundle.',
        input: S.obj({ preview_id: S.str(), trust_key: S.bool() }, ['preview_id']),
        run: (a, ctx) => this.install(a.preview_id as string, a.trust_key === true, ctx),
      },
      'packs.uninstall': {
        tier: 'T3',
        user_only: true,
        description: 'Uninstall a pack. Its types become retired.',
        input: id,
        run: (a, ctx) => {
          const r = this.rec(a.id as string);
          if (r.view.trust === 'builtin') throw this.host.fail('conflict', 'built-in packs cannot be uninstalled; disable it instead');
          const dependants = [...this.packs.values()].filter((o) => o.view.extends.includes(r.view.id)).map((o) => o.view.id);
          if (dependants.length) throw this.host.fail('conflict', `${dependants.join(', ')} extend${dependants.length === 1 ? 's' : ''} it`);
          this.packs.delete(r.view.id);
          this.countKeys();
          this.host.event(ctx, 'packs.uninstall', { kind: 'pack', id: r.view.id }, `Uninstalled ${r.view.title}`);
          return { id: r.view.id };
        },
      },
      'packs.enable': {
        tier: 'T3',
        user_only: true,
        description: 'Enable a pack.',
        input: id,
        run: (a, ctx) => this.setEnabled(a.id as string, true, ctx),
      },
      'packs.disable': {
        tier: 'T1',
        user_only: true,
        description: 'Disable a pack: its role is hidden and its types retired.',
        input: id,
        run: (a, ctx) => this.setEnabled(a.id as string, false, ctx),
      },
      'packs.rollback': {
        tier: 'T3',
        user_only: true,
        description: 'Go back to the version before the last update.',
        input: id,
        run: (a, ctx) => {
          const r = this.rec(a.id as string);
          if (!r.prev) throw this.host.fail('conflict', 'there is no earlier version to roll back to');
          const from = r.view.version;
          this.packs.set(r.view.id, { view: { ...r.prev.view, enabled: r.view.enabled }, perms: r.prev.perms, prev: null });
          this.countKeys();
          this.host.event(ctx, 'packs.rollback', { kind: 'pack', id: r.view.id }, `Rolled ${r.view.title} back from ${from}`);
          return this.view(this.rec(r.view.id));
        },
      },
      'packs.reset_layout': {
        tier: 'T1',
        user_only: true,
        description: "Reset the pack's role layout to the pack default.",
        input: id,
        run: (a, ctx) => {
          const r = this.rec(a.id as string);
          this.host.event(ctx, 'packs.reset_layout', { kind: 'pack', id: r.view.id }, `Reset layout of ${r.view.title}`);
          return { id: r.view.id, role_id: r.view.role_id };
        },
      },
      'packs.dev_set': {
        tier: 'T3',
        user_only: true,
        description: 'Turn dev mode on for a folder, or off with null. Not persisted.',
        input: S.obj({ path: S.nstr() }, ['path']),
        run: (a, ctx) => {
          const path = a.path as string | null;
          this.packs.delete('dev.sandbox');
          this.devPath = path;
          if (path) {
            const none = { modules: 0, ui_widgets: 0, prompts: 0, annotations: 0 };
            this.packs.set('dev.sandbox', {
              view: { id: 'dev.sandbox', version: '0.0.0', title: 'Sandbox (dev)', role_id: 'dev-sandbox', trust: 'dev', enabled: true, extends: [], key_fp: null, capabilities: none, status: 'ok', previous_version: null },
              perms: [],
              prev: null,
            });
          }
          this.host.event(ctx, 'packs.dev_set', { kind: 'pack_dev', id: 'dev' }, path ? 'Dev mode on' : 'Dev mode off');
          return { path, packs: path ? 1 : 0 };
        },
      },
      'packs.dev_get': {
        tier: 'T0',
        user_only: true,
        description: 'The dev folder, its packs and the folders that failed to load.',
        input: S.obj(),
        run: () => ({
          path: this.devPath,
          packs: [...this.packs.values()].filter((r) => r.view.trust === 'dev').map((r) => this.view(r)),
          errors: [],
        }),
      },
      'packs.dev_reload': {
        tier: 'T1',
        user_only: true,
        description: 'Read the dev folder again.',
        input: S.obj(),
        run: (_a, ctx) => {
          if (!this.devPath) throw this.host.fail('conflict', 'dev mode is off');
          this.host.event(ctx, 'packs.dev_reload', { kind: 'pack_dev', id: 'dev' }, 'Reloaded dev packs');
          return { path: this.devPath, packs: [...this.packs.values()].filter((r) => r.view.trust === 'dev').length };
        },
      },
      'packs.keys_list': {
        tier: 'T0',
        user_only: true,
        description: 'Trusted publisher keys.',
        input: S.obj(),
        run: () => {
          this.countKeys();
          return { keys: [...this.keys.values()].map((k) => ({ ...k })) };
        },
      },
      'packs.keys_add': {
        tier: 'T3',
        user_only: true,
        description: 'Trust a publisher key.',
        input: S.obj({ pubkey_b64: S.str(), label: S.str() }, ['pubkey_b64', 'label']),
        run: (a, ctx) => {
          const label = (a.label as string).trim();
          if (!label) throw this.host.fail('invalid_args', 'label must not be empty');
          let raw: string;
          try {
            raw = atob((a.pubkey_b64 as string).trim());
          } catch {
            throw this.host.fail('invalid_args', 'pubkey_b64 is not base64');
          }
          if (raw.length !== 32) throw this.host.fail('invalid_args', 'an Ed25519 public key is 32 bytes');
          const fingerprint = fakeDigest(raw);
          const key_id = fingerprint.slice(0, 16);
          if (this.keys.has(key_id)) throw this.host.fail('conflict', 'that key is already in the list');
          const k: PackKey = { key_id, fingerprint, label, first_party: false, revoked: false, added_at: this.host.now().toISOString(), packs: 0 };
          this.keys.set(key_id, k);
          this.host.event(ctx, 'packs.keys_add', { kind: 'pack_key', id: key_id }, `Trusted key ${label}`);
          return { ...k };
        },
      },
      'packs.keys_revoke': {
        tier: 'T3',
        user_only: true,
        description: "Revoke a key: every pack signed with it is disabled and its types retired.",
        input: S.obj({ key_id: S.str() }, ['key_id']),
        run: (a, ctx) => {
          const k = this.keyOf(a.key_id as string);
          if (!k) throw this.host.fail('not_found', `key ${String(a.key_id)} not found`);
          if (k.first_party) throw this.host.fail('denied', 'HalpTask keys are compiled in and cannot be revoked');
          k.revoked = true;
          const disabled: string[] = [];
          for (const r of this.packs.values()) {
            if (r.view.key_fp?.startsWith(k.key_id) && r.view.enabled) {
              r.view.enabled = false;
              r.view.status = 'blocked';
              disabled.push(r.view.id);
            }
          }
          this.countKeys();
          this.host.event(ctx, 'packs.keys_revoke', { kind: 'pack_key', id: k.key_id }, `Revoked key ${k.label}`);
          return { key: { ...k }, disabled_packs: disabled, invalid_items: [] };
        },
      },
    };
  }

  private setEnabled(id: string, enabled: boolean, ctx: { actor: Actor }): PackView {
    const r = this.rec(id);
    if (r.view.trust === 'builtin') throw this.host.fail('conflict', 'built-in packs cannot be disabled');
    if (r.view.trust === 'dev') throw this.host.fail('conflict', 'dev packs follow the dev folder');
    const key = r.view.key_fp ? this.keyOf(r.view.key_fp.slice(0, 16)) : undefined;
    if (enabled && key?.revoked) throw this.host.fail('conflict', 'its signing key was revoked');
    r.view.enabled = enabled;
    r.view.status = enabled ? 'ok' : 'disabled';
    this.host.event(ctx, enabled ? 'packs.enable' : 'packs.disable', { kind: 'pack', id }, `${enabled ? 'Enabled' : 'Disabled'} ${r.view.title}`);
    return this.view(r);
  }
}
