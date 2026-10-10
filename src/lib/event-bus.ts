export type EventCallback<T = any> = (payload: T) => void;

export interface EventBusContract {
  emit<T = any>(event: string, payload?: T): void;
  on<T = any>(event: string, callback: EventCallback<T>): () => void;
  once<T = any>(event: string, callback: EventCallback<T>): () => void;
  off<T = any>(event: string, callback: EventCallback<T>): void;
  clear(): void;
}

export class EventBus implements EventBusContract {
  private listeners: Map<string, Set<EventCallback>> = new Map();

  emit<T = any>(event: string, payload?: T): void {
    const handlers = this.listeners.get(event);
    if (!handlers || handlers.size === 0) return;

    // Clone to safely handle unsubscriptions during emission
    const toNotify = Array.from(handlers);
    for (const cb of toNotify) {
      try {
        cb(payload);
      } catch (err) {
        console.error(`[EventBus] Error in listener for event "${event}":`, err);
      }
    }
  }

  on<T = any>(event: string, callback: EventCallback<T>): () => void {
    let handlers = this.listeners.get(event);
    if (!handlers) {
      handlers = new Set();
      this.listeners.set(event, handlers);
    }
    handlers.add(callback as EventCallback);

    return () => {
      this.off(event, callback);
    };
  }

  once<T = any>(event: string, callback: EventCallback<T>): () => void {
    const wrapper: EventCallback<T> = (payload: T) => {
      this.off(event, wrapper);
      callback(payload);
    };
    return this.on(event, wrapper);
  }

  off<T = any>(event: string, callback: EventCallback<T>): void {
    const handlers = this.listeners.get(event);
    if (handlers) {
      handlers.delete(callback as EventCallback);
      if (handlers.size === 0) {
        this.listeners.delete(event);
      }
    }
  }

  clear(): void {
    this.listeners.clear();
  }
}

export const eventBus = new EventBus();
