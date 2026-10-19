import type { EventsSocketLike } from './eventsSocket';

/** Tests: a WebSocket stand-in the test drives (`open()`, `send(msg)` as the server, `close()`). */
export class FakeSocket implements EventsSocketLike {
  static all: FakeSocket[] = [];
  readyState = 0;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  constructor(public url: string) {
    FakeSocket.all.push(this);
  }
  open() {
    this.readyState = 1;
    this.onopen?.(new Event('open'));
  }
  send(msg: unknown) {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(msg) }));
  }
  close() {
    this.readyState = 3;
    this.onclose?.(new CloseEvent('close'));
  }
}
