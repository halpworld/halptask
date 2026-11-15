import type { Transport } from './transport';
import type {
  Args,
  ArgsOf,
  CommandName,
  ErrorCode,
  HubEvent,
  HubResponse,
  PendingConfirmation,
  ResultOf,
} from './types';
import { ulid } from '../util/ulid';

export class HubApiError extends Error {
  readonly code: ErrorCode;
  readonly command: string;
  readonly confirmationId?: string;

  constructor(command: string, code: ErrorCode, message: string, confirmationId?: string) {
    super(message);
    this.name = 'HubApiError';
    this.code = code;
    this.command = command;
    this.confirmationId = confirmationId;
  }
}

/**
 * How a parked request of ours ended, when the handler (the hub store) ran
 * the approval itself:
 *  - `approved`: the approved command's result;
 *  - `denied`: denied here, elsewhere (the socket's deny-only user channel),
 *    or expired;
 *  - `error`: the approval ran and failed, or the request could not be
 *    shown again after it changed.
 */
export type ConsentOutcome =
  | { kind: 'approved'; result: unknown }
  | { kind: 'denied'; message: string }
  | { kind: 'error'; code: ErrorCode; message: string };

/**
 * Called when a dispatch returns `confirmation_required`. The handler must
 * render in app chrome (never inside a widget). It answers either
 *  - `true` / `false`: the client approves or denies; when the approval is
 *    refused with `conflict` and a new `confirmation_id` (the request changed
 *    since it was shown), the client asks again with `changed: true`; or
 *  - a ConsentOutcome, when the handler already decided with the core.
 */
export type ConsentHandler = (pending: PendingConfirmation, changed: boolean) => Promise<boolean | ConsentOutcome>;

/** Upper bound on re-asks for one call (each needs a real change in the core). */
const MAX_REASKS = 8;

export class HubClient {
  private consentHandler: ConsentHandler | null = null;

  constructor(readonly transport: Transport) {}

  get kind(): Transport['kind'] {
    return this.transport.kind;
  }

  setConsentHandler(handler: ConsentHandler | null): void {
    this.consentHandler = handler;
  }

  /** Low-level: send one request and return the envelope as-is. */
  async dispatchRaw(command: string, args: Args = {}): Promise<HubResponse> {
    return this.transport.send({ command, args, request_id: ulid() });
  }

  /**
   * Typed dispatch. Unwraps the envelope, throws `HubApiError` on failure and
   * routes `confirmation_required` through the consent handler.
   */
  async dispatch<K extends CommandName>(command: K, args: ArgsOf<K>): Promise<ResultOf<K>>;
  async dispatch(command: string, args?: Args): Promise<unknown>;
  async dispatch(command: string, args: Args = {}): Promise<unknown> {
    const res = await this.dispatchRaw(command, args);
    if (res.ok) return res.result;

    const { error } = res;
    if (error.code === 'confirmation_required' && error.confirmation_id && this.consentHandler) {
      return this.confirm(command, args, error.confirmation_id, error.message);
    }
    throw new HubApiError(command, error.code, error.message, error.confirmation_id);
  }

  private async confirm(command: string, args: Args, firstId: string, message: string): Promise<unknown> {
    let id = firstId;
    for (let round = 0; ; round++) {
      const pending = await this.describePending(id, command, args, message);
      const answer = await this.consentHandler!(pending, round > 0);
      if (typeof answer === 'object') {
        if (answer.kind === 'approved') return answer.result;
        if (answer.kind === 'denied') throw new HubApiError(command, 'denied', answer.message, id);
        throw new HubApiError(command, answer.code, answer.message, id);
      }
      if (!answer) {
        await this.dispatchRaw('consent.deny', { id });
        throw new HubApiError(command, 'denied', 'Declined by you', id);
      }
      const approval = await this.dispatchRaw('consent.approve', { id });
      if (approval.ok) return approval.result;
      const e = approval.error;
      // v1.4: the request changed since it was shown; it is pending again under a new id.
      if (e.code === 'conflict' && e.confirmation_id && e.confirmation_id !== id && round < MAX_REASKS) {
        id = e.confirmation_id;
        message = e.message;
        continue;
      }
      throw new HubApiError(command, e.code, e.message, e.confirmation_id);
    }
  }

  onEvent(listener: (event: HubEvent) => void): () => void {
    return this.transport.subscribe(listener);
  }

  private async describePending(
    id: string,
    command: string,
    args: Args,
    message: string,
  ): Promise<PendingConfirmation> {
    const res = await this.dispatchRaw('consent.get', { id });
    if (res.ok) return res.result as PendingConfirmation;
    // Core without consent.get: fall back to what we know locally.
    return {
      id,
      command,
      args,
      actor: 'user',
      tier: 'T3',
      summary: message,
      created_at: new Date().toISOString(),
    };
  }
}
