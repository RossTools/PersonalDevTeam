import { cva, type VariantProps } from "class-variance-authority"
import { cn } from '@ui/lib/utils'

const badgeVariants = cva(
  "group/badge inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-md border border-transparent px-1.5 text-meta font-medium whitespace-nowrap transition-colors duration-(--pdt-duration) focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none [&>svg]:pointer-events-none [&>svg]:size-3!",
  {
    variants: {
      variant: {
        neutral: "bg-muted text-muted-foreground [a]:hover:text-foreground",
        accent: "bg-accent text-link",
        running: "bg-running-soft text-running",
        waiting: "bg-waiting-soft text-waiting",
        failed: "bg-failed-soft text-failed",
        outline: "border-border text-foreground [a]:hover:bg-muted",
      },
      shape: {
        label: "",
        count: "min-w-5 rounded-full px-1 tabular-nums",
      },
    },
    defaultVariants: {
      variant: "neutral",
      shape: "label",
    },
  }
)

function Badge({
  className,
  variant = "neutral",
  shape = "label",
  render,
  ...props
}: React.ComponentProps<"span"> &
  VariantProps<typeof badgeVariants> & {
    render?: (props: React.HTMLAttributes<HTMLElement>) => React.ReactNode
  }) {
  if (render) {
    const renderProps = {
      "data-slot": "badge",
      "data-variant": variant,
      className: cn(badgeVariants({ variant, shape }), className),
      ...props,
    }

    return render(renderProps)
  }

  return (
    <span
      data-slot="badge"
      data-variant={variant}
      className={cn(badgeVariants({ variant, shape }), className)}
      {...props}
    />
  )
}

export { Badge, badgeVariants }
