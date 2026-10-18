import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from '@ui/lib/utils'

const alertVariants = cva(
  "group/alert flex w-full items-start gap-2 rounded-md border px-3 py-2 text-left text-sm [&>svg]:mt-0.5 [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-current",
  {
    variants: {
      variant: {
        info: "border-border bg-card text-card-foreground [&>svg]:text-primary",
        warning: "border-transparent bg-waiting-soft text-foreground [&>svg]:text-waiting",
        error: "border-transparent bg-failed-soft text-foreground [&>svg]:text-failed",
      },
    },
    defaultVariants: {
      variant: "info",
    },
  }
)

function Alert({
  className,
  variant,
  ...props
}: React.ComponentProps<"div"> & VariantProps<typeof alertVariants>) {
  return (
    <div
      data-slot="alert"
      className={cn(alertVariants({ variant }), className)}
      {...props}
    />
  )
}

function AlertTitle({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-title"
      className={cn(
        "font-medium [&_a]:underline [&_a]:underline-offset-3",
        className
      )}
      {...props}
    />
  )
}

function AlertDescription({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-description"
      className={cn(
        "text-sm text-pretty text-foreground [&_a]:underline [&_a]:underline-offset-3 [&_p:not(:last-child)]:mb-2",
        className
      )}
      {...props}
    />
  )
}

function AlertAction({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-action"
      className={cn("ml-auto flex shrink-0 items-center gap-1 self-center", className)}
      {...props}
    />
  )
}

export { Alert, AlertTitle, AlertDescription, AlertAction }
