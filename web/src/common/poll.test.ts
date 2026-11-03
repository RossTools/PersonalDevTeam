import { afterEach, describe, expect, it } from 'vitest';
import { pagedPoll, POLL, setStreamOpen, streamPoll } from './poll';

describe('poll tiers', () => {
  afterEach(() => setStreamOpen(false));

  it('a paged list polls live until it is scrolled back past two pages', () => {
    const pages = (n: number) => ({ state: { data: { pages: Array.from({ length: n }) } } });
    expect([pagedPoll({ state: {} }), pagedPoll(pages(2)), pagedPoll(pages(3))]).toEqual([POLL.live, POLL.live, POLL.paged]);
  });

  it('what the notification stream pushes polls slowly only while the stream is connected', () => {
    expect(streamPoll()).toBe(POLL.live);
    setStreamOpen(true);
    expect(streamPoll()).toBe(POLL.pushed);
    setStreamOpen(false);
    expect(streamPoll()).toBe(POLL.live);
  });
});
