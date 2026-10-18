import type { ReactElement } from 'react';
import { Pressable, type PressEvent } from 'react-aria-components';

const clickOnKeyboard = (e: PressEvent) => {
  if (e.pointerType === 'keyboard') (e.target as HTMLElement).click();
};

/**
 * React Aria triggers (MenuTrigger, DialogTrigger) hand their behaviour to the child through a press responder, which
 * React Aria components (Button, our IconButton) pick up by themselves. A plain DOM element can't, so it's wrapped in
 * `Pressable`. Pressable's keyboard handling would swallow the element's own Enter/Space click; `clickOnKeyboard`
 * gives it back, so `<button onClick>` keeps working from the keyboard.
 */
export function pressableChild(child: ReactElement): ReactElement {
  if (typeof child.type !== 'string') return child;
  return <Pressable onPress={clickOnKeyboard}>{child as ReactElement<Record<string, unknown>, string>}</Pressable>;
}
