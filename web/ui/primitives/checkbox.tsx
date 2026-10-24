"use client"

import { cn } from '@ui/lib/utils'
import {
  Checkbox as CheckboxPrimitive,
  composeRenderProps,
  type CheckboxProps,
} from "react-aria-components"
import { CheckIcon, MinusIcon } from "lucide-react"

/**
 * The box plus its label (children), as one React Aria `<label>`; the whole row is the hit target. `relative` keeps
 * React Aria's visually hidden (absolute) input inside the label: anchored elsewhere, focusing it on a click scrolls
 * an overflow-hidden ancestor (the app shell) to reach it, shifting the whole UI.
 */
function Checkbox({ className, children, shape = "square", danger = false, ...props }: CheckboxProps & {
  /** `round`: an 18px circle that fills `running` green with a dark check (the todo list). */
  shape?: "square" | "round"
  /** Round only: a red border (an overdue todo). */
  danger?: boolean
}) {
  return (
    <CheckboxPrimitive
      data-slot="checkbox"
      className={composeRenderProps(className, (className) =>
        cn(
          "group/checkbox relative inline-flex items-center gap-2 text-sm text-foreground outline-none select-none pointer-coarse:min-h-control-touch data-disabled:cursor-not-allowed data-disabled:opacity-50",
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
              data-shape={shape}
              className={cn(
                "flex shrink-0 items-center justify-center border transition-colors group-data-focus-visible/checkbox:ring-2 group-data-focus-visible/checkbox:ring-ring group-data-invalid/checkbox:border-destructive motion-reduce:transition-none",
                shape === "round"
                  ? cn(
                      "size-4.5 rounded-full border-[1.5px] bg-transparent text-background group-hover/checkbox:border-foreground group-data-selected/checkbox:border-running group-data-selected/checkbox:bg-running [&>svg]:size-3 [&>svg]:stroke-3",
                      danger ? "border-destructive" : "border-muted-foreground"
                    )
                  : "size-3.5 rounded-sm border-input bg-card text-primary-foreground group-data-selected/checkbox:border-primary group-data-selected/checkbox:bg-primary group-data-indeterminate/checkbox:border-primary group-data-indeterminate/checkbox:bg-primary [&>svg]:size-3"
              )}
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
