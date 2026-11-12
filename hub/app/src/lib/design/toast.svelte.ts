export interface ToastAction {
  label: string;
  run: () => void | Promise<void>;
}

export interface ToastItem {
  id: number;
  message: string;
  tone: 'neutral' | 'success' | 'danger' | 'agent';
  action?: ToastAction;
  timeout: number;
}

class Toasts {
  items = $state<ToastItem[]>([]);
  private seq = 0;

  show(message: string, opts: { tone?: ToastItem['tone']; action?: ToastAction; timeout?: number } = {}): number {
    const id = ++this.seq;
    const item: ToastItem = { id, message, tone: opts.tone ?? 'neutral', action: opts.action, timeout: opts.timeout ?? 5000 };
    this.items = [...this.items.slice(-3), item];
    if (item.timeout > 0) setTimeout(() => this.dismiss(id), item.timeout);
    return id;
  }

  error(message: string): number {
    return this.show(message, { tone: 'danger', timeout: 7000 });
  }

  dismiss(id: number): void {
    this.items = this.items.filter((t) => t.id !== id);
  }
}

export const toasts = new Toasts();
