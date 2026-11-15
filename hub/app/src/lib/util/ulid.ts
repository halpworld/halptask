// Minimal ULID (Crockford base32, 48-bit time + 80-bit randomness).
const ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';

function randomBytes(n: number): Uint8Array {
  const out = new Uint8Array(n);
  globalThis.crypto.getRandomValues(out);
  return out;
}

export function ulid(now: number = Date.now()): string {
  let time = '';
  let t = Math.floor(now);
  for (let i = 0; i < 10; i++) {
    time = ALPHABET[t % 32] + time;
    t = Math.floor(t / 32);
  }
  const bytes = randomBytes(16);
  let rand = '';
  for (let i = 0; i < 16; i++) rand += ALPHABET[(bytes[i] ?? 0) % 32];
  return time + rand;
}
