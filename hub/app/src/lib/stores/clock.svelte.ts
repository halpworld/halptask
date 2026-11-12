// One shared ticking clock for the whole window (countdowns, "12m ago",
// the ambient clock). A single interval instead of one per widget.

class Clock {
  now = $state(Date.now());
  private timer: ReturnType<typeof setInterval> | null = null;

  start(): void {
    if (this.timer || typeof window === 'undefined') return;
    // Align to the next whole second so every display ticks together.
    const align = 1000 - (Date.now() % 1000);
    setTimeout(() => {
      this.now = Date.now();
      this.timer = setInterval(() => (this.now = Date.now()), 1000);
    }, align);
  }
}

export const clock = new Clock();
