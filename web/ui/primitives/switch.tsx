import { cn } from '@ui/lib/utils'
import {
  composeRenderProps,
  Switch as SwitchPrimitive,
  type SwitchProps as SwitchPrimitiveProps,
} from "react-aria-components"

/**
 * Track + thumb plus its label (children), as one React Aria `<label>`; the whole row is the hit target. `relative`
 * keeps the visually hidden input inside the label (see checkbox.tsx).
 */
function Switch({
  className,
  size = "default",
  children,
  ...props
}: SwitchPrimitiveProps & {
  size?: "sm" | "default"
}) {
  return (
    <SwitchPrimitive
      data-slot="switch"
      data-size={size}
      className={composeRenderProps(className, (className) =>
        cn(
          "group/switch relative inline-flex items-center gap-2 text-sm text-foreground outline-none select-none pointer-coarse:min-h-control-touch data-disabled:cursor-not-allowed data-disabled:opacity-50",
          className
        )
      )}
      {...props}
    >
      {composeRenderProps(children, (children) => (
        <>
          <span
            data-slot="switch-track"
            aria-hidden
            className="relative inline-flex h-4 w-7 shrink-0 items-center rounded-full border border-transparent bg-input p-px transition-colors group-data-[size=sm]/switch:h-3.5 group-data-[size=sm]/switch:w-6 group-data-focus-visible/switch:ring-2 group-data-focus-visible/switch:ring-ring group-data-invalid/switch:border-destructive group-data-selected/switch:bg-primary motion-reduce:transition-none"
          >
            <span
              data-slot="switch-thumb"
              className="block size-3 rounded-full bg-card shadow-popover transition-transform group-data-[size=sm]/switch:size-2.5 group-data-selected/switch:translate-x-3 group-data-[size=sm]/switch:group-data-selected/switch:translate-x-2.5 motion-reduce:transition-none"
            />
          </span>
          {children}
        </>
      ))}
    </SwitchPrimitive>
  )
}

export { Switch }
