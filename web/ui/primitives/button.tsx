import type * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "cn"
import {
  Button as ButtonPrimitive,
  Link as LinkPrimitive,
  type ButtonProps as ButtonPrimitiveProps,
  type LinkProps as LinkPrimitiveProps,
} from "react-aria-components"

// Graphite density: md = 28px (h-control-md), sm = 24px (h-control-sm), 44px on coarse pointers. shadcn's variant and
// size names stay (other primitives use them); the public Button (web/ui/Button.tsx) maps PDT's names onto them.
const buttonVariants = cva(
  "group/button relative inline-flex shrink-0 items-center justify-center gap-1.5 rounded-md border border-transparent text-sm font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 data-pending:cursor-wait aria-invalid:border-destructive motion-reduce:transition-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
  {
    variants: {
      variant: {
        default: "bg-primary font-semibold text-primary-foreground hover:brightness-110",
        outline:
          "border-border bg-card text-foreground hover:bg-surface-2 aria-expanded:bg-surface-2 aria-pressed:bg-accent",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-accent aria-expanded:bg-accent",
        ghost:
          "text-muted-foreground hover:bg-muted hover:text-foreground aria-expanded:bg-muted aria-expanded:text-foreground aria-pressed:bg-accent aria-pressed:text-foreground",
        destructive:
          "border-destructive/30 bg-failed-soft font-semibold text-destructive hover:bg-destructive/20 focus-visible:ring-destructive/40",
        link: "h-auto px-0 text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-control-md px-2.5 pointer-coarse:h-control-touch pointer-coarse:px-3",
        xs: "h-control-sm gap-1 px-2 text-xs pointer-coarse:h-control-touch pointer-coarse:px-3",
        sm: "h-control-sm gap-1 px-2 text-xs pointer-coarse:h-control-touch pointer-coarse:px-3",
        lg: "h-9 px-3 pointer-coarse:h-control-touch",
        icon: "size-control-md pointer-coarse:size-control-touch [&_svg:not([class*='size-'])]:size-4",
        "icon-xs": "size-control-sm pointer-coarse:size-control-touch",
        "icon-sm": "size-control-sm pointer-coarse:size-control-touch",
        "icon-lg": "size-9 pointer-coarse:size-control-touch [&_svg:not([class*='size-'])]:size-4",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  ...props
}: Omit<ButtonPrimitiveProps, "className"> &
  React.RefAttributes<HTMLButtonElement> &
  VariantProps<typeof buttonVariants> & {
    className?: string
  }) {
  return (
    <ButtonPrimitive
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  )
}

function LinkButton({
  className,
  variant = "default",
  size = "default",
  ...props
}: Omit<LinkPrimitiveProps, "className"> &
  VariantProps<typeof buttonVariants> & {
    className?: string
  }) {
  return (
    <LinkPrimitive
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  )
}

export { Button, LinkButton, buttonVariants }
