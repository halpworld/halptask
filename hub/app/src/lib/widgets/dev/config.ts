// Config schemas for the developer widgets (widget.config), JSON Schema like
// the pack's task types. Widgets also re-check every value when they read it.

import type { JsonSchema } from '../../api/types';

const obj = (properties: Record<string, JsonSchema> = {}): JsonSchema => ({ type: 'object', properties, additionalProperties: false });
const HHMM = { type: 'string', pattern: '^([01]\\d|2[0-3]):[0-5]\\d$' };

export const DEV_CONFIG: Record<string, JsonSchema> = {
  'pr-queue': obj({ stale_hours: { type: 'integer', minimum: 1, maximum: 720, description: 'Hours until a waiting review counts as stale.' } }),
  'my-prs': obj(),
  'ci-radar': obj(),
  'context-restore': obj(),
  'focus-protector': obj({
    minutes: { type: 'integer', minimum: 5, maximum: 240, description: 'Length of a deep work session.' },
    session: {
      type: 'object',
      description: 'The running session, written by the widget.',
      properties: {
        item_id: { type: ['string', 'null'], maxLength: 64 },
        started_at: { type: 'string', format: 'date-time', maxLength: 40 },
        minutes: { type: 'integer', minimum: 1, maximum: 480 },
      },
      required: ['started_at', 'minutes'],
      additionalProperties: false,
    },
  }),
  'ask-catcher': obj(),
  'day-plan': obj({
    day_start: { ...HHMM, description: 'Start of the working day, local HH:MM.' },
    day_end: { ...HHMM, description: 'End of the working day, local HH:MM.' },
    default_minutes: { type: 'integer', minimum: 5, maximum: 240, description: 'Minutes for a task without an estimate.' },
  }),
};
