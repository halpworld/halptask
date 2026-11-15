// Config schemas for the engineering manager widgets (widget.config). Widgets
// re-check every value when they read it.

import type { JsonSchema } from '../../api/types';

const obj = (properties: Record<string, JsonSchema> = {}): JsonSchema => ({ type: 'object', properties, additionalProperties: false });
const HHMM = { type: 'string', pattern: '^([01]\\d|2[0-3]):[0-5]\\d$' };

export const EM_CONFIG: Record<string, JsonSchema> = {
  // Two orders only. There is deliberately no way to sort people by a number.
  'people-roster': obj({ sort: { enum: ['name', 'next'], description: 'Alphabetical, or by next one-on-one.' } }),
  commitments: obj(),
  'one-on-one': obj({ person_id: { type: 'string', pattern: '^[A-Za-z0-9_-]+$', maxLength: 64, description: 'The person picked last, written by the widget.' } }),
  'meeting-load': obj({
    budget_hours: { type: 'number', minimum: 1, maximum: 80, description: 'Meeting hours per week you are willing to spend.' },
    day_start: { ...HHMM, description: 'Start of the working day, local HH:MM.' },
    day_end: { ...HHMM, description: 'End of the working day, local HH:MM.' },
  }),
  'team-pulse': obj({ weeks: { type: 'integer', minimum: 2, maximum: 26, description: 'Weeks of history in each sparkline.' } }),
  'weekly-update': obj(),
};
