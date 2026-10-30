// The one-line install (public/install.sh, public/install.ps1): the systems the website offers it for, its command,
// and which system a visitor is on.

/** In tab order; macOS first, the one shown without JS. */
export const OSES = [
  { id: 'mac', label: 'macOS', note: 'Apple silicon. Installs into ~/Applications and starts at login.' },
  { id: 'linux', label: 'Linux', note: 'x86_64 or aarch64, glibc 2.35 or newer. Installs into ~/.local/opt/pdt as a systemd user service.' },
  { id: 'windows', label: 'Windows', note: 'In PowerShell. Installs for you only, without admin rights, and starts at login.' },
];

/** The command for `os` (`mac`, `linux` or `windows`), with the scripts served from the site at `root` (a URL ending
 * in /). */
export function installCommand(os, root) {
  return os === 'windows' ? `irm ${new URL('install.ps1', root)} | iex` : `curl -fsSL ${new URL('install.sh', root)} | sh`;
}

/** The visitor's system from `navigator.userAgentData.platform` (when the browser has it) or the user agent: Windows,
 * Linux (not Android), else macOS. */
export function detectOs(platform, userAgent = '') {
  const s = `${platform ?? ''} ${userAgent}`;
  if (/windows|win32|win64/i.test(s)) return 'windows';
  if (/android/i.test(s)) return 'mac';
  if (/linux|x11|cros/i.test(s)) return 'linux';
  return 'mac';
}
