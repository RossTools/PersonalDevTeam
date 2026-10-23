const PREF = 'pdt.desktopNotifications';
const LOCK = 'pdt.desktopNotifications';

/** Whether this browser shows desktop notifications while PDT's page is hidden. */
export function desktopNotificationsOn(): boolean {
  return typeof localStorage !== 'undefined' && localStorage.getItem(PREF) === '1'
    && typeof Notification !== 'undefined' && Notification.permission === 'granted';
}

/** Turns desktop notifications on (asking the browser's permission) or off. Resolves to whether they are on. */
export async function setDesktopNotifications(on: boolean): Promise<boolean> {
  if (!on || typeof Notification === 'undefined') {
    localStorage.removeItem(PREF);
    return false;
  }
  const permission = Notification.permission === 'default' ? await Notification.requestPermission() : Notification.permission;
  if (permission !== 'granted') {
    localStorage.removeItem(PREF);
    return false;
  }
  localStorage.setItem(PREF, '1');
  return true;
}

/**
 * Makes this tab the one of its browser that shows desktop notifications, once no other tab holds the Web Lock
 * (`navigator.locks`). Without Web Locks every tab counts as the leader. Returns a function that lets go.
 */
export function holdLeadership(onLeader: () => void): () => void {
  const locks = typeof navigator !== 'undefined' ? navigator.locks : undefined;
  if (!locks) {
    onLeader();
    return () => {};
  }
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => { release = resolve; });
  const abort = new AbortController();
  locks.request(LOCK, { signal: abort.signal }, () => {
    onLeader();
    return held;
  }).catch(() => {
    // Aborted before it was granted.
  });
  return () => {
    abort.abort();
    release();
  };
}

/** Shows a desktop notification (one per `tag`: the OS replaces, doesn't stack). */
export function showDesktop(title: string, body: string, tag: string, onClick: () => void): Notification | null {
  if (typeof Notification === 'undefined') return null;
  try {
    const d = new Notification(title, { body, tag });
    d.onclick = () => {
      window.focus();
      onClick();
      d.close();
    };
    return d;
  } catch {
    // Some browsers only allow notifications from a service worker.
    return null;
  }
}
