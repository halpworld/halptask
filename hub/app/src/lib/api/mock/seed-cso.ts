// The Chief Sustainability Officer's mock day: the pack's types (exactly as
// in hub/packs/cso/pack.json), a fictional mid-size manufacturer, its
// disclosure deadlines, supplier data requests, evidence, targets, board
// brief, regulatory radar and grid readings.
//
// DEMO DATA. The company, people and every figure are invented for the
// mock. Regulatory dates and certainties are illustrative and marked
// [verify]; never rely on them. Dates are relative to `today` (the mock
// opens on 2026-11-10, the SB 253 Scope 1 and 2 filing day in the plan).
// tests/cso-seed.test.ts checks every item against the schemas.

import type { Item, TaskType, Widget } from '../types';
import { addDays } from '../../util/dates';

/** The CSO pack's task types, exactly as in the pack. */
export const CSO_TYPES: TaskType[] = [
  {
    "id": "cso.data_request",
    "title": "Data request",
    "icon": "database",
    "schema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "description": "One ESG data point asked of an owner or supplier. The carbon platform or ERP stays the system of record; source_ref points there.",
      "type": "object",
      "properties": {
        "supplier": {
          "type": "string",
          "minLength": 1,
          "maxLength": 200
        },
        "owner": {
          "type": "string",
          "minLength": 1,
          "maxLength": 120
        },
        "scope": {
          "enum": [
            "scope1",
            "scope2",
            "scope3"
          ]
        },
        "category": {
          "type": "integer",
          "minimum": 1,
          "maximum": 15
        },
        "metric": {
          "type": "string",
          "minLength": 1,
          "maxLength": 200
        },
        "unit": {
          "type": "string",
          "minLength": 1,
          "maxLength": 40
        },
        "period": {
          "type": "string",
          "pattern": "^[0-9]{4}(-(Q[1-4]|H[12]|0[1-9]|1[0-2]))?$",
          "maxLength": 7
        },
        "due": {
          "type": "string",
          "format": "date",
          "maxLength": 10
        },
        "requested_on": {
          "type": "string",
          "format": "date",
          "maxLength": 10
        },
        "status": {
          "enum": [
            "requested",
            "chased",
            "received",
            "validated",
            "rejected"
          ]
        },
        "chase_count": {
          "type": "integer",
          "minimum": 0,
          "maximum": 1000
        },
        "last_chased": {
          "type": "string",
          "format": "date-time",
          "maxLength": 40
        },
        "source_ref": {
          "type": "string",
          "minLength": 1,
          "maxLength": 200
        }
      },
      "required": [
        "supplier",
        "metric",
        "status"
      ],
      "additionalProperties": false
    },
    "display": {
      "primary_fields": [
        "supplier",
        "metric",
        "period"
      ],
      "badge_field": "status"
    }
  },
  {
    "id": "cso.evidence",
    "title": "Evidence",
    "icon": "file-check",
    "schema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "description": "A file that backs a data point, identified by its SHA-256. Once approved it must not change: x-halp-immutable-when asks the core to refuse agent and integration writes to an approved item. The core enforces this: only the user approves, approval cannot be undone, and only appends to the approval chain are allowed afterwards.",
      "x-halp-immutable-when": {
        "status": [
          "approved"
        ]
      },
      "type": "object",
      "properties": {
        "file_name": {
          "type": "string",
          "minLength": 1,
          "maxLength": 255,
          "pattern": "^[^/\\\\]+$"
        },
        "sha256": {
          "type": "string",
          "pattern": "^[0-9a-f]{64}$",
          "minLength": 64,
          "maxLength": 64
        },
        "size_bytes": {
          "type": "integer",
          "minimum": 0,
          "maximum": 1000000000000
        },
        "source_system": {
          "type": "string",
          "minLength": 1,
          "maxLength": 120
        },
        "data_point_id": {
          "type": "string",
          "pattern": "^[A-Za-z0-9_-]+$",
          "minLength": 1,
          "maxLength": 64
        },
        "data_point": {
          "type": "string",
          "minLength": 1,
          "maxLength": 200
        },
        "status": {
          "enum": [
            "draft",
            "in_review",
            "approved",
            "rejected"
          ]
        },
        "approvals": {
          "type": "array",
          "maxItems": 20,
          "items": {
            "type": "object",
            "properties": {
              "reviewer": {
                "type": "string",
                "minLength": 1,
                "maxLength": 120
              },
              "decision": {
                "enum": [
                  "approved",
                  "rejected",
                  "changes_requested"
                ]
              },
              "at": {
                "type": "string",
                "format": "date-time",
                "maxLength": 40
              },
              "note": {
                "type": "string",
                "maxLength": 500
              }
            },
            "required": [
              "reviewer",
              "decision",
              "at"
            ],
            "additionalProperties": false
          }
        }
      },
      "required": [
        "file_name",
        "sha256",
        "status"
      ],
      "additionalProperties": false,
      "if": {
        "properties": {
          "status": {
            "const": "approved"
          }
        },
        "required": [
          "status"
        ]
      },
      "then": {
        "required": [
          "approvals"
        ],
        "properties": {
          "approvals": {
            "contains": {
              "properties": {
                "decision": {
                  "const": "approved"
                }
              },
              "required": [
                "decision"
              ]
            }
          }
        }
      }
    },
    "display": {
      "primary_fields": [
        "file_name",
        "source_system"
      ],
      "badge_field": "status"
    }
  },
  {
    "id": "cso.disclosure",
    "title": "Disclosure",
    "icon": "landmark",
    "schema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "description": "A reporting deadline. regulatory_certainty says whether the date is settled law; anything but confirmed is shown as uncertain.",
      "type": "object",
      "properties": {
        "framework": {
          "enum": [
            "csrd_esrs",
            "sb253",
            "sb261",
            "issb",
            "other"
          ]
        },
        "name": {
          "type": "string",
          "minLength": 1,
          "maxLength": 200
        },
        "jurisdiction": {
          "type": "string",
          "minLength": 1,
          "maxLength": 80
        },
        "reporting_period": {
          "type": "string",
          "pattern": "^[0-9]{4}(-(Q[1-4]|H[12]|0[1-9]|1[0-2]))?$",
          "maxLength": 7
        },
        "due": {
          "type": "string",
          "format": "date",
          "maxLength": 10
        },
        "regulatory_certainty": {
          "enum": [
            "confirmed",
            "proposed",
            "litigated",
            "delayed"
          ]
        },
        "certainty_note": {
          "type": "string",
          "maxLength": 500
        },
        "source_url": {
          "type": "string",
          "format": "uri",
          "pattern": "^https://",
          "maxLength": 2048
        },
        "owner": {
          "type": "string",
          "minLength": 1,
          "maxLength": 120
        },
        "status": {
          "enum": [
            "not_started",
            "drafting",
            "in_review",
            "assured",
            "filed"
          ]
        }
      },
      "required": [
        "framework",
        "due",
        "regulatory_certainty"
      ],
      "additionalProperties": false
    },
    "display": {
      "primary_fields": [
        "framework",
        "jurisdiction",
        "due"
      ],
      "badge_field": "regulatory_certainty"
    }
  },
  {
    "id": "cso.target",
    "title": "Emissions target",
    "icon": "trending-down",
    "schema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "description": "An SBTi-style absolute target. Actuals are reported figures only, each with the source it came from; missing years stay missing.",
      "type": "object",
      "properties": {
        "name": {
          "type": "string",
          "minLength": 1,
          "maxLength": 120
        },
        "scope": {
          "enum": [
            "scope1",
            "scope2",
            "scope1_2",
            "scope3",
            "scope1_2_3"
          ]
        },
        "unit": {
          "type": "string",
          "minLength": 1,
          "maxLength": 40
        },
        "framework": {
          "enum": [
            "sbti_near_term",
            "sbti_net_zero",
            "internal",
            "other"
          ]
        },
        "validation": {
          "enum": [
            "none",
            "committed",
            "validated"
          ]
        },
        "base_year": {
          "type": "integer",
          "minimum": 1990,
          "maximum": 2100
        },
        "base_value": {
          "type": "number",
          "minimum": 0
        },
        "target_year": {
          "type": "integer",
          "minimum": 1990,
          "maximum": 2100
        },
        "target_value": {
          "type": "number",
          "minimum": 0
        },
        "actuals": {
          "type": "array",
          "maxItems": 100,
          "items": {
            "type": "object",
            "properties": {
              "year": {
                "type": "integer",
                "minimum": 1990,
                "maximum": 2100
              },
              "value": {
                "type": "number",
                "minimum": 0
              },
              "source_ref": {
                "type": "string",
                "minLength": 1,
                "maxLength": 200
              }
            },
            "required": [
              "year",
              "value"
            ],
            "additionalProperties": false
          }
        }
      },
      "required": [
        "scope",
        "base_year",
        "base_value",
        "target_year",
        "target_value"
      ],
      "additionalProperties": false
    },
    "display": {
      "primary_fields": [
        "scope",
        "target_year"
      ],
      "badge_field": "validation"
    }
  },
  {
    "id": "cso.reg_change",
    "title": "Regulatory change",
    "icon": "radar",
    "schema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "type": "object",
      "properties": {
        "title": {
          "type": "string",
          "minLength": 1,
          "maxLength": 200
        },
        "jurisdiction": {
          "type": "string",
          "minLength": 1,
          "maxLength": 80
        },
        "instrument": {
          "type": "string",
          "minLength": 1,
          "maxLength": 200
        },
        "stage": {
          "enum": [
            "proposed",
            "consultation",
            "adopted",
            "in_force",
            "delayed",
            "litigated",
            "withdrawn"
          ]
        },
        "effective_date": {
          "type": "string",
          "format": "date",
          "maxLength": 10
        },
        "impact": {
          "enum": [
            "high",
            "medium",
            "low"
          ]
        },
        "impact_note": {
          "type": "string",
          "maxLength": 500
        },
        "source_url": {
          "type": "string",
          "format": "uri",
          "pattern": "^https://",
          "maxLength": 2048
        },
        "last_reviewed": {
          "type": "string",
          "format": "date",
          "maxLength": 10
        }
      },
      "required": [
        "title",
        "jurisdiction",
        "stage",
        "source_url"
      ],
      "additionalProperties": false
    },
    "display": {
      "primary_fields": [
        "jurisdiction",
        "stage",
        "effective_date"
      ],
      "badge_field": "impact"
    }
  },
  {
    "id": "cso.board_brief",
    "title": "Board brief section",
    "icon": "presentation",
    "schema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "description": "One section of the board brief. Pre-disclosure figures: data class cso.unpublished. A section can only be ready or presented when every figure names its source item.",
      "x-halp-data-class": "cso.unpublished",
      "type": "object",
      "properties": {
        "section": {
          "type": "string",
          "minLength": 1,
          "maxLength": 120
        },
        "meeting_date": {
          "type": "string",
          "format": "date",
          "maxLength": 10
        },
        "status": {
          "enum": [
            "draft",
            "ready",
            "presented"
          ]
        },
        "narrative": {
          "type": "string",
          "maxLength": 4000
        },
        "figures": {
          "type": "array",
          "maxItems": 40,
          "items": {
            "type": "object",
            "properties": {
              "label": {
                "type": "string",
                "minLength": 1,
                "maxLength": 120
              },
              "value": {
                "type": "number"
              },
              "unit": {
                "type": "string",
                "minLength": 1,
                "maxLength": 40
              },
              "source_item_id": {
                "type": "string",
                "pattern": "^[A-Za-z0-9_-]+$",
                "minLength": 1,
                "maxLength": 64
              },
              "note": {
                "type": "string",
                "maxLength": 300
              }
            },
            "required": [
              "label",
              "value",
              "unit"
            ],
            "additionalProperties": false
          }
        }
      },
      "required": [
        "section",
        "status"
      ],
      "additionalProperties": false,
      "if": {
        "properties": {
          "status": {
            "enum": [
              "ready",
              "presented"
            ]
          }
        },
        "required": [
          "status"
        ]
      },
      "then": {
        "properties": {
          "figures": {
            "items": {
              "required": [
                "source_item_id"
              ]
            }
          }
        }
      }
    },
    "display": {
      "primary_fields": [
        "section",
        "meeting_date"
      ],
      "badge_field": "status"
    }
  },
  {
    "id": "cso.energy_reading",
    "title": "Energy reading",
    "icon": "zap",
    "schema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "description": "Latest grid carbon intensity and site power for one zone or meter, written by an integration (Electricity Maps, MQTT). One item per source and zone; series holds recent samples, oldest first.",
      "type": "object",
      "properties": {
        "source": {
          "enum": [
            "electricity_maps",
            "mqtt",
            "manual"
          ]
        },
        "zone": {
          "type": "string",
          "pattern": "^[A-Za-z0-9_./:-]+$",
          "minLength": 1,
          "maxLength": 64
        },
        "zone_label": {
          "type": "string",
          "minLength": 1,
          "maxLength": 120
        },
        "site": {
          "type": "string",
          "minLength": 1,
          "maxLength": 120
        },
        "gco2_per_kwh": {
          "type": "number",
          "minimum": 0,
          "maximum": 3000
        },
        "power_kw": {
          "type": "number",
          "minimum": -1000000,
          "maximum": 1000000
        },
        "renewable_pct": {
          "type": "number",
          "minimum": 0,
          "maximum": 100
        },
        "estimated": {
          "type": "boolean",
          "description": "The source marks the value as estimated (modelled), not measured; absent when it does not say."
        },
        "at": {
          "type": "string",
          "format": "date-time",
          "maxLength": 40
        },
        "series": {
          "type": "array",
          "maxItems": 192,
          "items": {
            "type": "object",
            "properties": {
              "at": {
                "type": "string",
                "format": "date-time",
                "maxLength": 40
              },
              "gco2_per_kwh": {
                "type": "number",
                "minimum": 0,
                "maximum": 3000
              },
              "power_kw": {
                "type": "number",
                "minimum": -1000000,
                "maximum": 1000000
              }
            },
            "required": [
              "at"
            ],
            "additionalProperties": false
          }
        }
      },
      "required": [
        "source",
        "zone",
        "at"
      ],
      "additionalProperties": false
    },
    "display": {
      "primary_fields": [
        "zone",
        "gco2_per_kwh"
      ],
      "badge_field": "source"
    }
  }
];

export const CSO_WIDGETS = ["deadline-wall","grid-carbon","trajectory","data-chase","evidence","board-brief","reg-radar","now","agenda","task-list","activity","quick-capture"];

/** The pack's default layout. */
export function csoLayoutWidgets(): Widget[] {
  const w = (id: string, kind: string, title: string, x: number, y: number, ww: number, h: number, config: Widget['config'] = {}): Widget => ({ id, kind, title, x, y, w: ww, h, config });
  return [
    w('deadline-wall', 'deadline-wall', 'Disclosure deadlines', 0, 0, 5, 4, { horizon_days: 800 }),
    w('grid-carbon', 'grid-carbon', 'Grid carbon', 5, 0, 3, 4, { stale_minutes: 30, low: 150, high: 400 }),
    w('trajectory', 'trajectory', 'Trajectory', 8, 0, 4, 4),
    w('data-chase', 'data-chase', 'Data chase', 0, 4, 7, 5, { chase_after_days: 7 }),
    w('evidence', 'evidence', 'Evidence locker', 7, 4, 5, 5),
    w('board-brief', 'board-brief', 'Board brief', 0, 9, 5, 5),
    w('reg-radar', 'reg-radar', 'Regulatory radar', 5, 9, 4, 5),
    w('activity', 'activity', 'Activity', 9, 9, 3, 5),
  ];
}

export interface CsoSeed {
  id: string;
  title: string;
  status?: Item['status'];
  due?: number | null;
  tags?: string[];
  notes?: string;
  type_id?: string;
  fields?: Item['fields'];
  kind?: Item['kind'];
  origin?: string;
}

/** A stable, fake SHA-256 (64 hex) for demo evidence. Not a hash of anything. */
export function demoSha(seed: string): string {
  let h = 2166136261;
  for (const c of seed) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
  let out = '';
  for (let i = 0; i < 8; i++) {
    h ^= h << 13;
    h >>>= 0;
    h ^= h >>> 17;
    h ^= h << 5;
    h >>>= 0;
    out += h.toString(16).padStart(8, '0');
  }
  return out;
}

const MINUTE = 60_000;
const DEMO = 'Demo data.';

export function csoSeeds(today: string, nowIso: string): CsoSeed[] {
  const now = Date.parse(nowIso);
  const at = (mins: number) => new Date(now + mins * MINUTE).toISOString();
  const day = (n: number) => addDays(today, n);
  const year = Number(today.slice(0, 4));

  /** Half-hourly grid intensity ending `endMins` from now: a smooth daily shape, deterministic. */
  const series = (endMins: number, base: number, swing: number, withPower = false) =>
    Array.from({ length: 48 }, (_, i) => {
      const mins = endMins - (47 - i) * 30;
      const hour = new Date(now + mins * MINUTE).getHours() + new Date(now + mins * MINUTE).getMinutes() / 60;
      const g = Math.round(base + swing * Math.sin(((hour - 7) / 24) * 2 * Math.PI) + ((i * 37) % 11) - 5);
      const s: Record<string, string | number> = { at: at(mins), gco2_per_kwh: Math.max(0, g) };
      if (withPower) s.power_kw = Math.round(820 + 260 * Math.sin(((hour - 4) / 24) * 2 * Math.PI) + ((i * 13) % 17));
      return s;
    });
  const se3 = series(-6, 38, 14);
  const pl = series(-190, 640, 90);
  const on = series(-4, 72, 25, true);
  const last = (s: ReturnType<typeof series>) => s[s.length - 1]!;

  const approve = (reviewer: string, mins: number, note?: string) => ({ reviewer, decision: 'approved', at: at(mins), ...(note ? { note } : {}) });

  return [
    // ----------------------------------------------------------- Disclosures
    {
      id: 'cso-1',
      title: 'File SB 253 Scope 1 & 2 report (FY2025)',
      status: 'in_progress',
      due: 0,
      tags: ['disclosure'],
      type_id: 'cso.disclosure',
      fields: { framework: 'sb253', name: 'SB 253 · Scope 1 & 2, FY2025', jurisdiction: 'California, US', reporting_period: '2025', due: day(0), regulatory_certainty: 'confirmed', certainty_note: `${DEMO} Filing date as set in the plan [verify with CARB].`, source_url: 'https://ww2.arb.ca.gov/our-work/programs/california-corporate-greenhouse-gas-ghg-reporting-and-climate-related-financial', owner: 'Ingrid Sandberg', status: 'in_review' },
      notes: 'Limited assurance letter received. CFO sign-off pending.',
    },
    {
      id: 'cso-disc-sb253-s3',
      title: 'SB 253 Scope 3 report (FY2026)',
      due: 240,
      tags: ['disclosure'],
      type_id: 'cso.disclosure',
      fields: { framework: 'sb253', name: 'SB 253 · Scope 3, FY2026', jurisdiction: 'California, US', reporting_period: '2026', due: day(240), regulatory_certainty: 'proposed', certainty_note: `${DEMO} Scope 3 timing depends on CARB rulemaking [verify].`, source_url: 'https://ww2.arb.ca.gov/our-work/programs/california-corporate-greenhouse-gas-ghg-reporting-and-climate-related-financial', owner: 'Ingrid Sandberg', status: 'not_started' },
    },
    {
      id: 'cso-disc-sb261',
      title: 'SB 261 climate-related financial risk report',
      due: 51,
      tags: ['disclosure'],
      type_id: 'cso.disclosure',
      fields: { framework: 'sb261', name: 'SB 261 · Climate risk report', jurisdiction: 'California, US', reporting_period: '2025', due: day(51), regulatory_certainty: 'litigated', certainty_note: `${DEMO} Enforcement paused by a Ninth Circuit injunction pending appeal [verify].`, source_url: 'https://ww2.arb.ca.gov/our-work/programs/california-corporate-greenhouse-gas-ghg-reporting-and-climate-related-financial', owner: 'Tomasz Wójcik', status: 'drafting' },
    },
    {
      id: 'cso-disc-csrd',
      title: 'CSRD sustainability statement (wave 2, FY2027)',
      due: 538,
      tags: ['disclosure'],
      type_id: 'cso.disclosure',
      fields: { framework: 'csrd_esrs', name: 'CSRD / ESRS · Wave 2, FY2027', jurisdiction: 'EU (Sweden)', reporting_period: '2027', due: day(538), regulatory_certainty: 'delayed', certainty_note: `${DEMO} Moved by the Omnibus stop-the-clock directive; thresholds still being amended [verify].`, source_url: 'https://finance.ec.europa.eu/capital-markets-union-and-financial-markets/company-reporting-and-auditing/company-reporting/corporate-sustainability-reporting_en', owner: 'Ingrid Sandberg', status: 'drafting' },
    },
    {
      id: 'cso-disc-issb',
      title: 'IFRS S2 climate disclosures for lead investor',
      due: 24,
      tags: ['disclosure'],
      type_id: 'cso.disclosure',
      fields: { framework: 'issb', name: 'IFRS S2 · Investor request', jurisdiction: 'Investor (voluntary)', reporting_period: '2025', due: day(24), regulatory_certainty: 'confirmed', certainty_note: `${DEMO} Date agreed with the investor.`, source_url: 'https://www.ifrs.org/issued-standards/ifrs-sustainability-standards-navigator/ifrs-s2-climate-related-disclosures/', owner: 'Tomasz Wójcik', status: 'drafting' },
    },
    {
      id: 'cso-disc-secr',
      title: 'Statutory sustainability report: board approval',
      due: -3,
      tags: ['disclosure'],
      type_id: 'cso.disclosure',
      fields: { framework: 'other', name: 'Annual report · Sustainability section', jurisdiction: 'Sweden', reporting_period: '2025', due: day(-3), regulatory_certainty: 'confirmed', certainty_note: DEMO, source_url: 'https://www.riksdagen.se/', owner: 'Ingrid Sandberg', status: 'in_review' },
    },
    {
      id: 'cso-disc-cdp',
      title: 'CDP Climate 2026 response',
      status: 'done',
      due: -40,
      tags: ['disclosure'],
      type_id: 'cso.disclosure',
      fields: { framework: 'other', name: 'CDP Climate 2026', jurisdiction: 'Voluntary', reporting_period: '2025', due: day(-40), regulatory_certainty: 'confirmed', source_url: 'https://www.cdp.net/', owner: 'Amara Okafor', status: 'filed' },
    },

    // ----------------------------------------------------- Supplier data chase
    { id: 'cso-req-nordwind', title: 'Nordwind Logistics: Q3 freight tonne-km', due: -6, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'Nordwind Logistics', owner: 'Amara Okafor', scope: 'scope3', category: 4, metric: 'Freight tonne-km by mode', unit: 't·km', period: '2026-Q3', due: day(-6), requested_on: day(-34), status: 'chased', chase_count: 3, last_chased: at(-9 * 24 * 60) } },
    { id: 'cso-req-halvorsen', title: 'Halvorsen Steel: product carbon footprint', due: 3, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'Halvorsen Steel', owner: 'Amara Okafor', scope: 'scope3', category: 1, metric: 'PCF, hot-rolled coil', unit: 'kgCO2e/t', period: '2025', due: day(3), requested_on: day(-21), status: 'chased', chase_count: 1, last_chased: at(-4 * 24 * 60) } },
    { id: 'cso-req-polymer', title: 'Baltic Polymer: resin volumes and EF', due: -1, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'Baltic Polymer Sp. z o.o.', owner: 'Tomasz Wójcik', scope: 'scope3', category: 1, metric: 'Resin purchased and emission factor', unit: 'kgCO2e/kg', period: '2025', due: day(-1), requested_on: day(-15), status: 'requested', chase_count: 0 } },
    { id: 'cso-req-airfreight', title: 'SkyCargo Nordic: air freight emissions', due: 12, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'SkyCargo Nordic', owner: 'Amara Okafor', scope: 'scope3', category: 4, metric: 'Air freight emissions (GLEC)', unit: 'tCO2e', period: '2026-Q3', due: day(12), requested_on: day(-9), status: 'requested', chase_count: 0 } },
    { id: 'cso-req-castings', title: 'Ostrava Castings: energy use per part', due: 20, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'Ostrava Castings a.s.', owner: 'Tomasz Wójcik', scope: 'scope3', category: 1, metric: 'Electricity and gas per casting', unit: 'kWh/part', period: '2025', due: day(20), requested_on: day(-2), status: 'requested', chase_count: 0 } },
    { id: 'cso-req-vattenfall', title: 'Plant A electricity: supplier-specific EF', due: -10, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'Nordic Power Supply AB', owner: 'Ingrid Sandberg', scope: 'scope2', metric: 'Market-based emission factor', unit: 'gCO2e/kWh', period: '2025', due: day(-10), requested_on: day(-40), status: 'validated', chase_count: 1, last_chased: at(-30 * 24 * 60), source_ref: 'invoice-batch NPS-2025-12' } },
    { id: 'cso-req-fuel', title: 'Plant B: diesel and LPG for forklifts', due: -14, tags: ['chase'], type_id: 'cso.data_request', fields: { supplier: 'Plant B, Gdańsk (internal)', owner: 'Tomasz Wójcik', scope: 'scope1', metric: 'Diesel and LPG consumed', unit: 'litres', period: '2025', due: day(-14), requested_on: day(-45), status: 'validated', chase_count: 0, source_ref: 'ERP fuel ledger FY2025' } },
    { id: 'cso-req-travel', title: 'TMC: business travel FY2025', due: -8, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'NorthStar Travel Management', owner: 'Amara Okafor', scope: 'scope3', category: 6, metric: 'Flights and hotel nights', unit: 'tCO2e', period: '2025', due: day(-8), requested_on: day(-30), status: 'received', chase_count: 2, last_chased: at(-12 * 24 * 60), source_ref: 'TMC export 2026-10' } },
    { id: 'cso-req-waste', title: 'Ontario plant: waste to landfill', due: -5, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'GreenCycle Waste Ontario', owner: 'Amara Okafor', scope: 'scope3', category: 5, metric: 'Waste by treatment route', unit: 't', period: '2025', due: day(-5), requested_on: day(-28), status: 'received', chase_count: 1, last_chased: at(-15 * 24 * 60), source_ref: 'GreenCycle annual statement' } },
    { id: 'cso-req-packaging', title: 'Lindö Packaging: PCF figures', due: -2, tags: ['chase'], origin: 'agent:carbon-platform', type_id: 'cso.data_request', fields: { supplier: 'Lindö Packaging', owner: 'Amara Okafor', scope: 'scope3', category: 1, metric: 'PCF, corrugated board', unit: 'kgCO2e/t', period: '2025', due: day(-2), requested_on: day(-25), status: 'rejected', chase_count: 2, last_chased: at(-6 * 24 * 60), source_ref: 'Supplier sheet v1 (industry averages, rejected)' } },

    // -------------------------------------------------------------- Evidence
    {
      id: 'cso-ev-nps-invoices',
      kind: 'bullet',
      title: 'NPS electricity invoices FY2025.pdf',
      origin: 'agent:docstore',
      type_id: 'cso.evidence',
      fields: { file_name: 'NPS_electricity_invoices_FY2025.pdf', sha256: demoSha('nps-invoices'), size_bytes: 4_218_331, source_system: 'SharePoint · Sustainability', data_point_id: 'cso-req-vattenfall', data_point: 'Scope 2 market-based EF, Plant A', status: 'approved', approvals: [approve('Amara Okafor', -12 * 24 * 60), approve('Ingrid Sandberg', -11 * 24 * 60, 'Matches the guarantee-of-origin certificates.')] },
    },
    {
      id: 'cso-ev-fuel-ledger',
      kind: 'bullet',
      title: 'Plant B fuel ledger FY2025.xlsx',
      origin: 'agent:docstore',
      type_id: 'cso.evidence',
      fields: { file_name: 'PlantB_fuel_ledger_FY2025.xlsx', sha256: demoSha('fuel-ledger'), size_bytes: 386_120, source_system: 'ERP export', data_point_id: 'cso-req-fuel', data_point: 'Scope 1 mobile combustion, Plant B', status: 'approved', approvals: [{ reviewer: 'Amara Okafor', decision: 'changes_requested', at: at(-20 * 24 * 60), note: 'LPG column in kg, not litres.' }, approve('Amara Okafor', -16 * 24 * 60), approve('Ingrid Sandberg', -15 * 24 * 60)] },
    },
    {
      id: 'cso-ev-tmc',
      kind: 'bullet',
      title: 'TMC export 2026-10.csv',
      origin: 'agent:docstore',
      type_id: 'cso.evidence',
      fields: { file_name: 'TMC_export_2026-10.csv', sha256: demoSha('tmc-export'), size_bytes: 912_004, source_system: 'NorthStar portal', data_point_id: 'cso-req-travel', data_point: 'Scope 3 cat 6, business travel', status: 'in_review', approvals: [approve('Amara Okafor', -2 * 24 * 60)] },
    },
    {
      id: 'cso-ev-assurance',
      kind: 'bullet',
      title: 'Limited assurance letter FY2025.pdf',
      origin: 'agent:docstore',
      type_id: 'cso.evidence',
      fields: { file_name: 'Limited_assurance_letter_FY2025.pdf', sha256: demoSha('assurance'), size_bytes: 1_204_511, source_system: 'Auditor portal', data_point: 'Scope 1 & 2 inventory FY2025', status: 'in_review', approvals: [{ reviewer: 'Ingrid Sandberg', decision: 'changes_requested', at: at(-26 * 60), note: 'Waiting for the signed version.' }] },
    },
    {
      id: 'cso-ev-lindo',
      kind: 'bullet',
      title: 'Lindö PCF sheet v1.xlsx',
      origin: 'agent:docstore',
      type_id: 'cso.evidence',
      fields: { file_name: 'Lindo_PCF_sheet_v1.xlsx', sha256: demoSha('lindo-v1'), size_bytes: 88_340, source_system: 'Supplier upload', data_point_id: 'cso-req-packaging', data_point: 'PCF, corrugated board', status: 'rejected', approvals: [{ reviewer: 'Amara Okafor', decision: 'rejected', at: at(-6 * 24 * 60), note: 'Industry averages, not primary data.' }] },
    },
    {
      id: 'cso-ev-meter',
      kind: 'bullet',
      title: 'Ontario sub-meter readings Oct.csv',
      origin: 'agent:docstore',
      type_id: 'cso.evidence',
      fields: { file_name: 'Ontario_submeter_2026-10.csv', sha256: demoSha('ontario-meter'), size_bytes: 2_045_998, source_system: 'MQTT archive', data_point: 'Scope 2 location-based, Ontario plant', status: 'draft', approvals: [] },
    },

    // --------------------------------------------------------------- Targets
    {
      id: 'cso-target-s12',
      kind: 'bullet',
      title: 'Scope 1 + 2: −42% by 2030',
      type_id: 'cso.target',
      fields: {
        name: 'Scope 1 + 2 near-term',
        scope: 'scope1_2',
        unit: 'tCO2e',
        framework: 'sbti_near_term',
        validation: 'validated',
        base_year: 2021,
        base_value: 48200,
        target_year: 2030,
        target_value: 27956,
        actuals: [
          { year: 2021, value: 48200, source_ref: 'GHG inventory FY2021 (assured)' },
          { year: 2022, value: 46350, source_ref: 'GHG inventory FY2022 (assured)' },
          // 2023: no assured figure (inventory restated, not yet re-assured): a gap.
          { year: 2024, value: 41980, source_ref: 'GHG inventory FY2024 (assured)' },
          { year: 2025, value: 39870 },
        ],
      },
      notes: `${DEMO} FY2025 is preliminary and has no source until the assurance letter is signed.`,
    },
    {
      id: 'cso-target-s3',
      kind: 'bullet',
      title: 'Scope 3: −25% by 2030',
      type_id: 'cso.target',
      fields: {
        name: 'Scope 3 near-term',
        scope: 'scope3',
        unit: 'tCO2e',
        framework: 'sbti_near_term',
        validation: 'validated',
        base_year: 2021,
        base_value: 612000,
        target_year: 2030,
        target_value: 459000,
        actuals: [
          { year: 2021, value: 612000, source_ref: 'Scope 3 screening FY2021' },
          { year: 2022, value: 598400, source_ref: 'Scope 3 inventory FY2022' },
          { year: 2023, value: 604900, source_ref: 'Scope 3 inventory FY2023' },
          { year: 2025, value: 571300 },
        ],
      },
    },

    // ----------------------------------------------------- Regulatory radar
    { id: 'cso-reg-omnibus', kind: 'bullet', title: 'EU Omnibus I: CSRD scope and thresholds', origin: 'agent:reg-digest', type_id: 'cso.reg_change', fields: { title: 'Omnibus I: CSRD scope and thresholds', jurisdiction: 'EU', instrument: 'Directive amending 2022/2464', stage: 'proposed', effective_date: day(420), impact: 'high', impact_note: `${DEMO} May take the Swedish entity out of scope for wave 2 [verify].`, source_url: 'https://eur-lex.europa.eu/', last_reviewed: day(-6) } },
    { id: 'cso-reg-sb261', kind: 'bullet', title: 'SB 261 injunction', origin: 'agent:reg-digest', type_id: 'cso.reg_change', fields: { title: 'SB 261 enforcement injunction', jurisdiction: 'California, US', instrument: 'Ninth Circuit order', stage: 'litigated', impact: 'high', impact_note: `${DEMO} Keep drafting; do not plan around the filing date [verify].`, source_url: 'https://ww2.arb.ca.gov/', last_reviewed: day(-2) } },
    { id: 'cso-reg-carb', kind: 'bullet', title: 'CARB SB 253 implementing regulation', origin: 'agent:reg-digest', type_id: 'cso.reg_change', fields: { title: 'CARB SB 253/261 implementing regulation', jurisdiction: 'California, US', instrument: 'CCR Title 17', stage: 'adopted', effective_date: day(-30), impact: 'medium', impact_note: `${DEMO} Fee schedule and first deadline [verify].`, source_url: 'https://ww2.arb.ca.gov/', last_reviewed: day(-45) } },
    { id: 'cso-reg-cbam', kind: 'bullet', title: 'CBAM definitive period', origin: 'agent:reg-digest', type_id: 'cso.reg_change', fields: { title: 'CBAM definitive period', jurisdiction: 'EU', instrument: 'Regulation (EU) 2023/956', stage: 'in_force', effective_date: `${year}-01-01`, impact: 'medium', impact_note: `${DEMO} Steel coil imports into Plant A need verified embedded emissions.`, source_url: 'https://taxation-customs.ec.europa.eu/carbon-border-adjustment-mechanism_en', last_reviewed: day(-12) } },
    { id: 'cso-reg-uksrs', kind: 'bullet', title: 'UK Sustainability Reporting Standards', origin: 'agent:reg-digest', type_id: 'cso.reg_change', fields: { title: 'UK SRS (ISSB-based) consultation', jurisdiction: 'United Kingdom', instrument: 'UK SRS S1 and S2', stage: 'consultation', impact: 'low', impact_note: `${DEMO} No UK entity today; watch only.`, source_url: 'https://www.gov.uk/', last_reviewed: day(-60) } },

    // ---------------------------------------------------------- Board brief
    {
      id: 'cso-brief-perf',
      kind: 'bullet',
      title: 'Board brief: Climate performance',
      type_id: 'cso.board_brief',
      fields: {
        section: 'Climate performance',
        meeting_date: day(9),
        status: 'draft',
        narrative: `${DEMO} Scope 1 + 2 is below the SBTi pathway on the last assured year. FY2025 stays out of the brief until it is assured.`,
        figures: [
          { label: 'Scope 1 + 2, FY2024', value: 41980, unit: 'tCO2e', source_item_id: 'cso-target-s12', note: 'Assured inventory FY2024.' },
          { label: 'Plant A market-based EF', value: 21, unit: 'gCO2e/kWh', source_item_id: 'cso-req-vattenfall' },
          { label: 'Supplier data received', value: 64, unit: '%', note: 'Needs a source item before ready.' },
        ],
      },
    },
    {
      id: 'cso-brief-reg',
      kind: 'bullet',
      title: 'Board brief: Disclosure readiness',
      type_id: 'cso.board_brief',
      fields: {
        section: 'Disclosure readiness',
        meeting_date: day(9),
        status: 'ready',
        narrative: `${DEMO} SB 253 Scope 1 and 2 is filed on time if the CFO signs today. SB 261 and CSRD wave 2 dates are not certain.`,
        figures: [{ label: 'Scope 1 + 2 base year', value: 48200, unit: 'tCO2e', source_item_id: 'cso-target-s12' }],
      },
    },
    {
      id: 'cso-brief-supply',
      kind: 'bullet',
      title: 'Board brief: Supply chain',
      type_id: 'cso.board_brief',
      fields: {
        section: 'Supply chain data',
        meeting_date: day(9),
        status: 'draft',
        narrative: `${DEMO} Two carriers are late with Q3 freight data. Packaging PCF rejected: industry averages, not primary data.`,
        figures: [{ label: 'Lindö PCF (rejected)', value: 0, unit: 'kgCO2e/t', source_item_id: 'cso-req-lindo-old' }],
      },
    },

    // ------------------------------------------------------- Grid readings
    { id: 'cso-grid-se3', kind: 'bullet', title: 'Grid: SE3 (Plant A, Västerås)', origin: 'agent:electricity-maps', type_id: 'cso.energy_reading', fields: { source: 'electricity_maps', zone: 'SE-SE3', zone_label: 'Sweden SE3 · Plant A', site: 'Plant A, Västerås', gco2_per_kwh: last(se3).gco2_per_kwh!, renewable_pct: 71, at: last(se3).at!, series: se3 } },
    { id: 'cso-grid-ca-on', kind: 'bullet', title: 'Site meter: Ontario plant', origin: 'agent:mqtt', type_id: 'cso.energy_reading', fields: { source: 'mqtt', zone: 'CA-ON', zone_label: 'Ontario · Plant C', site: 'Plant C, Hamilton', gco2_per_kwh: last(on).gco2_per_kwh!, power_kw: last(on).power_kw!, at: last(on).at!, series: on } },
    { id: 'cso-grid-pl', kind: 'bullet', title: 'Grid: Poland (Plant B, Gdańsk)', origin: 'agent:electricity-maps', type_id: 'cso.energy_reading', fields: { source: 'electricity_maps', zone: 'PL', zone_label: 'Poland · Plant B', site: 'Plant B, Gdańsk', gco2_per_kwh: last(pl).gco2_per_kwh!, renewable_pct: 27, at: last(pl).at!, series: pl } },

    // --------------------------------------------------------- Plain tasks
    { id: 'cso-task-cfo', title: 'Get CFO sign-off on SB 253 report', status: 'in_progress', due: 0, tags: ['approval'], type_id: 'base.task', fields: { priority: 'urgent' } },
    { id: 'cso-task-s2', title: 'Approve Scope 2 market-based figure', due: 0, tags: ['approval'] },
    { id: 'cso-task-restate', title: 'Re-assure restated FY2023 inventory', due: 30, tags: ['sbti'], notes: 'Until then 2023 stays a gap on the trajectory.' },
    { id: 'cso-task-board', title: 'Board pre-read: ESG quarterly', due: 7, tags: ['board'] },
  ];
}
