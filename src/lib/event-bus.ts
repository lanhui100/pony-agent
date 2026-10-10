export type EventCallback<T = any> = (payload: T) => void;

export interface EventBusContract {
  emit<T = any>(event: string, payload?: T): void;
  on<T = any>(event: string, callback: EventCallback<T>): () => void;
  once<T = any>(event: string, callback: EventCallback<T>): () => void;
  off<T = any>(event: string, callback: EventCallback<T>): void;
  clear(): void;
}

export class EventBus implements EventBusContract {
  emit<T = any>(_event: string, _payload?: T): void {
    throw new Error("Not implemented: EventBus.emit");
  }

  on<T = any>(_event: string, _callback: EventCallback<T>): () => void {
    throw new Error("Not implemented: EventBus.on");
  }

  once<T = any>(_event: string, _callback: EventCallback<T>): () => void {
    throw new Error("Not implemented: EventBus.once");
  }

  off<T = any>(_event: string, _callback: EventCallback<T>): void {
    throw new Error("Not implemented: EventBus.off");
  }

  clear(): void {
    throw new Error("Not implemented: EventBus.clear");
  }
}

export const eventBus = new EventBus();
