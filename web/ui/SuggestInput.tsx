import { useContext, useState, type KeyboardEvent } from 'react';
import { ComboBox, ComboBoxStateContext } from 'react-aria-components';
import { cn } from '@ui/lib/utils';
import { Input as InputPrimitive } from '@ui/primitives/input';
import { SelectItem, SelectList, SelectPopover } from '@ui/primitives/select';

export interface SuggestInputProps {
  /** Offered in a list under the field, filtered by what has been typed. */
  suggestions: string[];
  /** A suggestion was picked (click, tap, or arrow keys + Enter) or typed text was committed; the field then clears. */
  onCommit: (value: string) => void;
  'aria-label': string;
  placeholder?: string;
  /** sm 24px, md 28px (default); 44px on touch screens. */
  size?: 'sm' | 'md';
  maxLength?: number;
  disabled?: boolean;
  /** Keys that commit the typed text, besides Enter (e.g. ','). */
  commitKeys?: string[];
  className?: string;
}

/** A text field that adds things: pick a suggestion (one click) or type a new value and press Enter. */
export function SuggestInput({
  suggestions,
  onCommit,
  placeholder,
  size = 'md',
  maxLength,
  disabled,
  commitKeys = [],
  className,
  ...aria
}: SuggestInputProps) {
  const [text, setText] = useState('');
  const needle = text.trim().toLowerCase();
  const items = suggestions.filter((s) => s.toLowerCase().includes(needle)).map((s) => ({ id: s }));
  const commit = (value: string) => {
    setText('');
    const v = value.trim();
    if (v) onCommit(v);
  };
  return (
    <ComboBox
      aria-label={aria['aria-label']}
      inputValue={text}
      onInputChange={setText}
      selectedKey={null}
      onSelectionChange={(k) => { if (k !== null) commit(String(k)); }}
      allowsCustomValue
      menuTrigger="focus"
      items={items}
      isDisabled={disabled}
      className={cn('min-w-0', className)}
    >
      <SuggestField text={text} commit={commit} commitKeys={commitKeys} placeholder={placeholder} size={size} maxLength={maxLength} />
      <SelectPopover placement="bottom start" className="max-h-60 w-auto min-w-(--trigger-width)">
        <SelectList>{(item: { id: string }) => <SelectItem id={item.id}>{item.id}</SelectItem>}</SelectList>
      </SelectPopover>
    </ComboBox>
  );
}

function SuggestField({
  text,
  commit,
  commitKeys,
  placeholder,
  size,
  maxLength,
}: {
  text: string;
  commit: (v: string) => void;
  commitKeys: string[];
  placeholder?: string;
  size: 'sm' | 'md';
  maxLength?: number;
}) {
  const state = useContext(ComboBoxStateContext);
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key !== 'Enter' && !commitKeys.includes(e.key)) return;
    // Enter on a highlighted suggestion is React Aria's: it picks it (onSelectionChange commits).
    if (e.key === 'Enter' && state?.isOpen && state.selectionManager.focusedKey != null) return;
    e.preventDefault();
    commit(text);
    state?.close();
  };
  return <InputPrimitive autoComplete="off" placeholder={placeholder} size={size} maxLength={maxLength} onKeyDown={onKeyDown} />;
}
