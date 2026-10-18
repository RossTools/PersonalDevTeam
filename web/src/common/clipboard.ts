import { runAction } from './useAction';

/** Copies `text` and toasts `success`; a missing or refused clipboard toasts the error instead. */
export const copyToClipboard = (text: string, success: string) =>
  runAction(async () => {
    if (!navigator.clipboard) throw new Error('The clipboard is not available here');
    await navigator.clipboard.writeText(text);
  }, { success });
