// Event names the UI reacts to. A core event's `command` is the name of the
// command that caused it, except for the consent lifecycle events below, which
// no command emits directly. tests/contract.test.ts checks every name here
// against the core's commands.json plus CORE_NON_COMMAND_EVENTS, and checks
// that hub.svelte.ts handles no event name that is not listed here.
// TODO: once hub/docs/CONTRACT.md documents the event names, check against it.

/** Event names the core emits that are not command names. */
export const CORE_NON_COMMAND_EVENTS = ['consent.requested', 'consent.expired', 'consent.superseded', 'integrations.status', 'packs.status'] as const;

export const EV = {
  consentRequested: 'consent.requested',
  consentApprove: 'consent.approve',
  consentDeny: 'consent.deny',
  consentExpired: 'consent.expired',
  /** v1.4: the request changed before approval and was asked again under a new id. */
  consentSuperseded: 'consent.superseded',
  /** v1.4: the user cleared a client's taint. */
  policyClearTaint: 'policy.clear_taint',
  /** v1.5: an integration wrote synced items (actor `integration:<id>`). */
  extUpsert: 'ext.upsert',
  /** v1.5: an integration's state changed (pending, ok, error, needs_auth, ...). */
  integrationsStatus: 'integrations.status',
  /** Also rewrites items when `remove_items` is set. */
  integrationsDisconnect: 'integrations.disconnect',
  importTui: 'import.tui',
  roleSwitch: 'role.switch',
  historyUndo: 'history.undo',
  historyRedo: 'history.redo',
} as const;

/** Event-name prefixes the UI reacts to: each must match some core command. */
export const EV_PREFIX = {
  tasks: 'tasks.',
  capture: 'capture.',
  focus: 'focus.',
  layout: 'layout.',
  /** Agent administration: the settings panel re-reads on these. */
  clients: 'clients.',
  grants: 'grants.',
  rules: 'rules.',
  integrations: 'integrations.',
} as const;

/** Events that remove a pending consent request from the queue. */
export const CONSENT_SETTLED: readonly string[] = [EV.consentApprove, EV.consentDeny, EV.consentExpired, EV.consentSuperseded];
