"use client"

import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "cn"
import {
  ToggleButton as TogglePrimitive,
  type ToggleButtonProps,
} from "react-aria-components"

const toggleVariants = cva(
  "group/toggle inline-flex items-center justify-center gap-1 rounded-md text-sm font-medium whitespace-nowrap text-muted-foreground transition-colors outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 data-disabled:opacity-50 data-selected:text-foreground motion-reduce:transition-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
  {
    variants: {
      variant: {
        default: "bg-transparent hover:bg-muted data-selected:bg-accent",
        outline: "border border-input bg-transparent hover:bg-muted data-selected:bg-accent",
        // A segment of a SegmentedControl: the selected one is a raised card on the muted track.
        segment: "data-selected:bg-card data-selected:shadow-popover",
      },
      size: {
        default: "h-control-md min-w-control-md px-2.5 pointer-coarse:h-control-touch",
        sm: "h-control-sm min-w-control-sm px-2 text-xs pointer-coarse:h-control-touch",
        lg: "h-9 min-w-9 px-3 pointer-coarse:h-control-touch",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Toggle({
  className,
  variant = "default",
  size = "default",
  ...props
}: ToggleButtonProps & VariantProps<typeof toggleVariants>) {
  return (
    <TogglePrimitive
      data-slot="toggle"
      className={cn(toggleVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Toggle, toggleVariants }
