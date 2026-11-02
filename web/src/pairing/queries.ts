export const PAIRING_KEY = ['ui-pairing'];

/** Whether the page was sent here from a pairing link that was used up or too old (read once, at load). */
export const linkExpired = typeof location !== 'undefined' && new URLSearchParams(location.search).get('pairing') === 'expired';
