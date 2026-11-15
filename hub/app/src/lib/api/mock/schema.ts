// Typed-item `fields` validation for the mock, following the core
// (halp-domain task_type.rs: JSON Schema 2020-12 with formats checked).
// Covers the keywords the first-party packs use: type, enum, required,
// properties, additionalProperties: false, items, minItems/maxItems,
// minLength/maxLength (in code points), pattern, minimum/maximum, const,
// contains, if/then/else and the formats uri, date-time and date. Unknown
// keywords are ignored, as a JSON Schema validator would.

import type { JsonSchema } from '../types';

export class FieldError extends Error {}

const MAX_FIELDS_BYTES = 32 * 1024;
const MAX_FIELDS_DEPTH = 8;

const RFC3339 = /^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])[Tt]([01]\d|2[0-3]):[0-5]\d:([0-5]\d|60)(\.\d+)?([Zz]|[+-]([01]\d|2[0-3]):[0-5]\d)$/;
const DATE = /^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])$/;
const SCHEME = /^[A-Za-z][A-Za-z0-9+.-]*:/;

function isUri(s: string): boolean {
  if (!SCHEME.test(s) || /\s/.test(s)) return false;
  try {
    new URL(s);
    return true;
  } catch {
    return false;
  }
}

function typeOk(t: string, v: unknown): boolean {
  switch (t) {
    case 'string':
      return typeof v === 'string';
    case 'integer':
      return typeof v === 'number' && Number.isInteger(v);
    case 'number':
      return typeof v === 'number' && Number.isFinite(v);
    case 'boolean':
      return typeof v === 'boolean';
    case 'array':
      return Array.isArray(v);
    case 'object':
      return typeof v === 'object' && v !== null && !Array.isArray(v);
    case 'null':
      return v === null;
    default:
      return true;
  }
}

function depth(v: unknown): number {
  if (Array.isArray(v)) return 1 + Math.max(0, ...v.map(depth));
  if (v && typeof v === 'object') return 1 + Math.max(0, ...Object.values(v).map(depth));
  return 0;
}

function check(schema: JsonSchema, v: unknown, path: string): void {
  const types = schema.type === undefined ? [] : Array.isArray(schema.type) ? schema.type : [schema.type];
  if (types.length && !types.some((t) => typeOk(t, v))) throw new FieldError(`${path}: expected ${types.join(' or ')}`);
  if (schema.enum && !schema.enum.some((e) => JSON.stringify(e) === JSON.stringify(v))) throw new FieldError(`${path}: not one of the allowed values`);
  if (Object.hasOwn(schema, 'const') && JSON.stringify(schema.const) !== JSON.stringify(v)) throw new FieldError(`${path}: must be ${JSON.stringify(schema.const)}`);
  if (typeof v === 'number') {
    if (typeof schema.minimum === 'number' && v < schema.minimum) throw new FieldError(`${path}: must be >= ${schema.minimum}`);
    if (typeof schema.maximum === 'number' && v > schema.maximum) throw new FieldError(`${path}: must be <= ${schema.maximum}`);
  }
  if (typeof v === 'string') {
    const n = Array.from(v).length;
    if (typeof schema.minLength === 'number' && n < schema.minLength) throw new FieldError(`${path}: shorter than ${schema.minLength}`);
    if (typeof schema.maxLength === 'number' && n > schema.maxLength) throw new FieldError(`${path}: longer than ${schema.maxLength}`);
    if (typeof schema.pattern === 'string' && !new RegExp(schema.pattern, 'u').test(v)) throw new FieldError(`${path}: does not match the pattern`);
    if (schema.format === 'uri' && !isUri(v)) throw new FieldError(`${path}: not a URI`);
    if (schema.format === 'date-time' && !RFC3339.test(v)) throw new FieldError(`${path}: not an RFC 3339 date-time`);
    if (schema.format === 'date' && !DATE.test(v)) throw new FieldError(`${path}: not a date`);
  }
  if (Array.isArray(v)) {
    if (typeof schema.minItems === 'number' && v.length < schema.minItems) throw new FieldError(`${path}: fewer than ${schema.minItems} items`);
    if (typeof schema.maxItems === 'number' && v.length > schema.maxItems) throw new FieldError(`${path}: more than ${schema.maxItems} items`);
    if (schema.items) v.forEach((x, i) => check(schema.items!, x, `${path}/${i}`));
    if (isSchema(schema.contains) && !v.some((x) => valid(schema.contains as JsonSchema, x))) throw new FieldError(`${path}: no item matches the required shape`);
  }
  if (typeOk('object', v)) {
    const o = v as Record<string, unknown>;
    const props = schema.properties ?? {};
    for (const k of Object.keys(o)) {
      const p = Object.hasOwn(props, k) ? props[k] : undefined;
      if (p) check(p, o[k], `${path}/${k}`);
      else if (schema.additionalProperties === false) throw new FieldError(`${path}: unknown field \`${k}\``);
    }
    for (const r of schema.required ?? []) if (!Object.hasOwn(o, r)) throw new FieldError(`${path}: missing field \`${r}\``);
  }
  // if/then/else: `if` only selects; its failure is never an error by itself.
  if (isSchema(schema.if)) {
    const branch = valid(schema.if, v) ? schema.then : schema.else;
    if (isSchema(branch)) check(branch, v, path);
  }
}

function isSchema(s: unknown): s is JsonSchema {
  return typeof s === 'object' && s !== null && !Array.isArray(s);
}

function valid(schema: JsonSchema, v: unknown): boolean {
  try {
    check(schema, v, '');
    return true;
  } catch (e) {
    if (e instanceof FieldError) return false;
    throw e;
  }
}

/**
 * Like TaskTypeRegistry::validate_fields: untyped items take no fields, an
 * unknown type is refused, and a typed item's fields must match its schema.
 */
export function validateFields(schemaFor: (typeId: string) => JsonSchema | undefined, typeId: string | null, fields: Record<string, unknown>): void {
  const raw = JSON.stringify(fields);
  if (new TextEncoder().encode(raw).length > MAX_FIELDS_BYTES) throw new FieldError(`fields exceed ${MAX_FIELDS_BYTES} bytes`);
  if (depth(fields) > MAX_FIELDS_DEPTH) throw new FieldError(`fields nest deeper than ${MAX_FIELDS_DEPTH}`);
  if (typeId === null) {
    if (Object.keys(fields).length) throw new FieldError('fields require a type_id');
    return;
  }
  const schema = schemaFor(typeId);
  if (!schema) throw new FieldError(`unknown task type ${typeId}`);
  try {
    check(schema, fields, '');
  } catch (e) {
    if (e instanceof FieldError) throw new FieldError(`fields do not match ${typeId}: ${e.message}`);
    throw e;
  }
}
