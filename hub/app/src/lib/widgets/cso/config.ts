// Config schemas for the CSO widgets (widget.config). Widgets re-check every
// value when they read it (logic.ts intIn / numIn).

import type { JsonSchema } from '../../api/types';

const obj = (properties: Record<string, JsonSchema> = {}): JsonSchema => ({ type: 'object', properties, additionalProperties: false });
const ITEM_ID = { type: 'string', pattern: '^[A-Za-z0-9_-]+$', maxLength: 64 };

export const CSO_CONFIG: Record<string, JsonSchema> = {
  'deadline-wall': obj({
    horizon_days: { type: 'integer', minimum: 7, maximum: 1825, description: 'How far ahead to show deadlines, in days.' },
    show_filed: { type: 'boolean', description: 'Keep filed disclosures on the wall.' },
  }),
  'data-chase': obj({
    chase_after_days: { type: 'integer', minimum: 1, maximum: 90, description: 'Days without contact before a request is due a chase.' },
  }),
  evidence: obj(),
  trajectory: obj({ target_id: { ...ITEM_ID, description: 'The cso.target item to chart; the first target when unset.' } }),
  'grid-carbon': obj({
    zone: { type: 'string', pattern: '^[A-Za-z0-9_./:-]+$', maxLength: 64, description: 'Grid zone or site to show; all readings when unset.' },
    stale_minutes: { type: 'integer', minimum: 5, maximum: 1440, description: 'Minutes after which a reading is shown as stale.' },
    low: { type: 'number', minimum: 0, maximum: 3000, description: 'gCO2/kWh below which the grid counts as clean.' },
    high: { type: 'number', minimum: 0, maximum: 3000, description: 'gCO2/kWh above which the grid counts as dirty.' },
  }),
  'reg-radar': obj({
    min_impact: { enum: ['high', 'medium', 'low'], description: 'Hide changes below this impact.' },
    show_withdrawn: { type: 'boolean', description: 'Keep withdrawn proposals on the radar.' },
  }),
  'board-brief': obj({ meeting_date: { type: 'string', format: 'date', description: 'Only sections for this board meeting.' } }),
};
