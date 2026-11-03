import type { AriaAttributes, ReactNode, Ref } from 'react';
import type { PressEvent } from 'react-aria-components';
import { Button as ButtonPrimitive } from '@ui/primitives/button';
import { Spinner } from './Spinner';
import { Tooltip } from './Tooltip';

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
export type ButtonSize = 'sm' | 'md' | 'lg';

const variantMap = {
  primary: 'default',
  secondary: 'outline',
  ghost: 'ghost',
  danger: 'destructive',
} as const;

const sizeMap = { sm: 'sm', md: 'default', lg: 'lg' } as const;
const iconSizeMap = { sm: 'icon-sm', md: 'icon', lg: 'icon-lg' } as const;

/** Props shared by Button and IconButton: the familiar DOM-style names, mapped onto React Aria's. */
export interface ButtonBaseProps extends AriaAttributes {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Shows a spinner in place of the content (same width) and ignores presses until it clears. */
  pending?: boolean;
  /** 24px even on touch screens, for fixed-height strips (Banner) where a 44px target would resize the page. */
  compact?: boolean;
  disabled?: boolean;
  /** Called on press: mouse, touch, pen, keyboard, and `click()`. */
  onClick?: (e: PressEvent) => void;
  type?: 'button' | 'submit' | 'reset';
  /** Shown as a Tooltip (hover and keyboard focus); React Aria drops the native title attribute. */
  title?: string;
  id?: string;
  name?: string;
  value?: string;
  form?: string;
  autoFocus?: boolean;
  className?: string;
  ref?: Ref<HTMLButtonElement>;
  /** `slot="drag"` marks a drag handle inside a React Aria row. */
  slot?: string;
  [data: `data-${string}`]: string | number | boolean | undefined;
}

export interface ButtonProps extends ButtonBaseProps {
  /** Label, optionally with a leading icon (`<Plus /> New`). */
  children?: ReactNode;
}

/** @internal Shared by IconButton. */
export function renderButton({
  variant = 'secondary',
  size = 'md',
  pending = false,
  compact = false,
  disabled,
  onClick,
  type = 'button',
  className,
  children,
  icon = false,
  title,
  ...rest
}: ButtonProps & { icon?: boolean }) {
  const button = (
    <ButtonPrimitive
      {...rest}
      variant={variantMap[variant]}
      size={compact ? (icon ? 'icon-compact' : 'compact') : icon ? iconSizeMap[size] : sizeMap[size]}
      type={type}
      isDisabled={disabled}
      isPending={pending}
      onPress={onClick}
      className={className}
    >
      {pending ? (
        <>
          <span className="invisible inline-flex items-center gap-[inherit]">{children}</span>
          <span className="absolute inset-0 flex items-center justify-center">
            <Spinner decorative />
          </span>
        </>
      ) : (
        children
      )}
    </ButtonPrimitive>
  );
  return title ? <Tooltip content={title}>{button}</Tooltip> : button;
}

/**
 * A labelled action. Variants primary | secondary (default) | ghost | danger; sizes sm (24px) | md (28px), both 44px on
 * touch screens, and lg (48px everywhere: the big buttons along the bottom of a phone screen).
 */
export function Button(props: ButtonProps) {
  return renderButton(props);
}
