import { cn } from "cn"
import { Loader2Icon } from "lucide-react"

/** A spinning icon. `role="status"` + label unless it's decorative (inside a pending Button, which announces itself). */
function Spinner({
  className,
  label = "Loading",
  decorative = false,
  ...props
}: React.ComponentProps<"svg"> & { label?: string; decorative?: boolean }) {
  return (
    <Loader2Icon
      data-slot="spinner"
      {...(decorative ? { "aria-hidden": true } : { role: "status", "aria-label": label })}
      className={cn("size-3.5 animate-spin motion-reduce:animate-none", className)}
      {...props}
    />
  )
}

export { Spinner }
