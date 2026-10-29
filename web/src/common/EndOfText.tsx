// The room below an editor's text (PDT-264): it fills what is left of the page, and a tap there puts the caret at the end.

/**
 * Fills the rest of a column under an editor; a tap on it calls `onTap` (focus the text's end). The press keeps the
 * editor's focus, so a phone's keyboard stays up. At least the page's bottom margin (small on phones, where the
 * keyboard sits right below).
 */
export function EndOfText({ onTap }: { onTap: () => void }) {
  return (
    <div
      aria-hidden
      data-testid="end-of-text"
      className="min-h-4 flex-1 cursor-text md:min-h-16"
      onMouseDown={(e) => e.preventDefault()}
      onClick={onTap}
    />
  );
}
