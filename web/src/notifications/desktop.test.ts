import { afterEach, describe, expect, it, vi } from 'vitest';
import { stubProperty } from '../test-utils';
import { desktopNotificationsOn, holdLeadership, setDesktopNotifications, showDesktop } from './desktop';

const PREF = 'pdt.desktopNotifications';

/** A browser Notification with this permission (and what requestPermission answers); it records what it shows. */
function stubNotification(permission: NotificationPermission, asked: NotificationPermission = permission) {
  const shown: { title: string; options: NotificationOptions; close: ReturnType<typeof vi.fn>; onclick: (() => void) | null }[] = [];
  const N = vi.fn(function (this: (typeof shown)[number], title: string, options: NotificationOptions) {
    this.title = title;
    this.options = options;
    this.close = vi.fn();
    this.onclick = null;
    shown.push(this);
  }) as unknown as typeof Notification & { requestPermission: ReturnType<typeof vi.fn> };
  Object.assign(N, { permission, requestPermission: vi.fn().mockResolvedValue(asked) });
  vi.stubGlobal('Notification', N);
  return { N, shown };
}

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe('desktop notifications', () => {
  it('are on only when turned on here and the browser allows them', () => {
    stubNotification('granted');
    expect(desktopNotificationsOn()).toBe(false);
    localStorage.setItem(PREF, '1');
    expect(desktopNotificationsOn()).toBe(true);
    stubNotification('denied');
    expect(desktopNotificationsOn()).toBe(false);
    vi.stubGlobal('Notification', undefined);
    expect(desktopNotificationsOn()).toBe(false);
  });

  it("turning them on asks the browser's permission when it hasn't been asked", async () => {
    const { N } = stubNotification('default', 'granted');
    await expect(setDesktopNotifications(true)).resolves.toBe(true);
    expect(N.requestPermission).toHaveBeenCalled();
    expect(localStorage.getItem(PREF)).toBe('1');
  });

  it('stay off when the browser refuses, or has refused before (without asking again)', async () => {
    const asked = stubNotification('default', 'denied');
    localStorage.setItem(PREF, '1');
    await expect(setDesktopNotifications(true)).resolves.toBe(false);
    expect(localStorage.getItem(PREF)).toBeNull();
    expect(asked.N.requestPermission).toHaveBeenCalled();
    const refused = stubNotification('denied');
    await expect(setDesktopNotifications(true)).resolves.toBe(false);
    expect(refused.N.requestPermission).not.toHaveBeenCalled();
  });

  it('turning them off, or a browser without notifications, clears the setting', async () => {
    stubNotification('granted');
    localStorage.setItem(PREF, '1');
    await expect(setDesktopNotifications(false)).resolves.toBe(false);
    expect(localStorage.getItem(PREF)).toBeNull();
    vi.stubGlobal('Notification', undefined);
    localStorage.setItem(PREF, '1');
    await expect(setDesktopNotifications(true)).resolves.toBe(false);
    expect(localStorage.getItem(PREF)).toBeNull();
  });

  it('shows one per tag; clicking it focuses the window, opens the item and closes it', () => {
    const { shown } = stubNotification('granted');
    const focus = vi.spyOn(window, 'focus').mockImplementation(() => {});
    const onClick = vi.fn();
    expect(showDesktop('PDT-1 needs you', 'Approve a command', 'session:s1', onClick)).not.toBeNull();
    expect(shown[0]).toMatchObject({ title: 'PDT-1 needs you', options: { body: 'Approve a command', tag: 'session:s1' } });
    shown[0].onclick!();
    expect(focus).toHaveBeenCalled();
    expect(onClick).toHaveBeenCalled();
    expect(shown[0].close).toHaveBeenCalled();
  });

  it('shows nothing where the browser has no notifications, or allows them only from a service worker', () => {
    vi.stubGlobal('Notification', undefined);
    expect(showDesktop('t', 'b', 'x', () => {})).toBeNull();
    vi.stubGlobal('Notification', vi.fn(() => {
      throw new TypeError('Illegal constructor');
    }));
    expect(showDesktop('t', 'b', 'x', () => {})).toBeNull();
  });
});

describe('holdLeadership', () => {
  it('without Web Locks, every tab leads', () => {
    stubProperty(navigator, 'locks', undefined);
    const onLeader = vi.fn();
    holdLeadership(onLeader)();
    expect(onLeader).toHaveBeenCalledTimes(1);
  });

  it('leads once it holds the lock, and lets go of it (or stops waiting) when released', async () => {
    let granted: (() => Promise<void>) | null = null;
    let signal: AbortSignal | undefined;
    const request = vi.fn((_name: string, opts: { signal: AbortSignal }, cb: () => Promise<void>) => {
      signal = opts.signal;
      granted = cb;
      return new Promise<void>(() => {});
    });
    stubProperty(navigator, 'locks', { request });
    const onLeader = vi.fn();
    const release = holdLeadership(onLeader);
    expect(request.mock.calls[0][0]).toBe('pdt.desktopNotifications');
    expect(onLeader).not.toHaveBeenCalled();
    let held = false;
    const lock = granted!().then(() => { held = true; });
    expect(onLeader).toHaveBeenCalledTimes(1);
    await Promise.resolve();
    expect(held).toBe(false);
    release();
    await lock;
    expect(held).toBe(true);
    expect(signal!.aborted).toBe(true);
  });

  it("a request aborted before it was granted isn't an error", async () => {
    stubProperty(navigator, 'locks', { request: vi.fn().mockRejectedValue(new DOMException('aborted', 'AbortError')) });
    const onLeader = vi.fn();
    holdLeadership(onLeader)();
    await new Promise((r) => setTimeout(r, 0));
    expect(onLeader).not.toHaveBeenCalled();
  });
});
