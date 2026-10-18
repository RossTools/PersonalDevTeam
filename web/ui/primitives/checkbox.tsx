"use client"

import { cn } from '@ui/lib/utils'
import {
  Checkbox as CheckboxPrimitive,
  composeRenderProps,
  type CheckboxProps,
} from "react-aria-components"
import { CheckIcon, MinusIcon } from "lucide-react"

/** The box plus its label (children), as one React Aria `<label>`; the whole row is the hit target. */
function Checkbox({ className, children, ...props }: CheckboxProps) {
  return (
    <CheckboxPrimitive
      data-slot="checkbox"
      className={composeRenderProps(className, (className) =>
        cn(
          "group/checkbox inline-flex items-center gap-2 text-sm text-foreground outline-none select-none pointer-coarse:min-h-control-touch data-disabled:cursor-not-allowed data-disabled:opacity-50",
          className
        )
      )}
      {...props}
    >
      {composeRenderProps(
        children,
        (children, { isSelected, isIndeterminate }) => (
          <>
            <span
              data-slot="checkbox-indicator"
              aria-hidden
              className="flex size-3.5 shrink-0 items-center justify-center rounded-sm border border-input bg-card text-primary-foreground transition-colors group-data-focus-visible/checkbox:ring-2 group-data-focus-visible/checkbox:ring-ring group-data-invalid/checkbox:border-destructive group-data-selected/checkbox:border-primary group-data-selected/checkbox:bg-primary group-data-indeterminate/checkbox:border-primary group-data-indeterminate/checkbox:bg-primary motion-reduce:transition-none [&>svg]:size-3"
            >
              {isIndeterminate ? <MinusIcon /> : isSelected ? <CheckIcon /> : null}
            </span>
            {children}
          </>
        )
      )}
    </CheckboxPrimitive>
  )
}

export { Checkbox }
