// Reduced-motion aware durations for Svelte transitions (CSS uses tokens).

class Motion {
  reduced = $state(false);

  constructor() {
    if (typeof window !== 'undefined' && window.matchMedia) {
      const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
      this.reduced = mq.matches;
      mq.addEventListener('change', (e) => (this.reduced = e.matches));
    }
  }

  /** Duration in ms, or 0 when the user prefers reduced motion. */
  ms(base: number): number {
    return this.reduced ? 0 : base;
  }
}

export const motion = new Motion();
