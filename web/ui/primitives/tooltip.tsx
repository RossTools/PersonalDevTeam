import * as React from "react"
import { cn } from "cn"
import {
  Focusable,
  OverlayArrow,
  Tooltip as TooltipPrimitive,
  TooltipTrigger as TooltipTriggerPrimitive,
} from "react-aria-components"

/**
 * Children: the trigger, then the Tooltip. A React Aria control (Button, Link, …) takes the trigger props itself; any
 * other element is wrapped in `Focusable` (it must forward its ref and be focusable with an interactive role).
 */
function TooltipTrigger({
  delay = 600,
  closeDelay = 200,
  wrapTrigger = false,
  children,
  ...props
}: React.ComponentProps<typeof TooltipTriggerPrimitive> & { wrapTrigger?: boolean }) {
  const [trigger, tooltip] = React.Children.toArray(children)

  return (
    <TooltipTriggerPrimitive
      data-slot="tooltip-trigger"
      delay={delay}
      closeDelay={closeDelay}
      {...props}
    >
      {wrapTrigger ? (
        <Focusable>
          {trigger as React.ComponentProps<typeof Focusable>["children"]}
        </Focusable>
      ) : (
        trigger
      )}
      {tooltip}
    </TooltipTriggerPrimitive>
  )
}

function Tooltip({
  className,
  placement = "top",
  offset = 6,
  crossOffset = 0,
  children,
  ...props
}: Omit<
  React.ComponentProps<typeof TooltipPrimitive>,
  "children" | "className"
> & {
  className?: string
  children?: React.ReactNode
}) {
  return (
    <TooltipPrimitive
      data-slot="tooltip-content"
      placement={placement}
      offset={offset}
      crossOffset={crossOffset}
      className={cn(
        "z-(--pdt-z-menu) inline-flex w-fit max-w-xs origin-(--trigger-anchor-point) items-center gap-1.5 rounded-md bg-foreground px-2 py-1 text-xs text-background shadow-popover duration-(--pdt-duration) has-data-[slot=kbd]:pr-1 motion-reduce:animate-none data-entering:animate-in data-entering:fade-in-0 data-entering:zoom-in-95 data-exiting:animate-out data-exiting:fade-out-0 data-exiting:zoom-out-95 **:data-[slot=kbd]:relative **:data-[slot=kbd]:isolate **:data-[slot=kbd]:rounded-sm",
        className
      )}
      {...props}
    >
      {children}
      <OverlayArrow
        className="size-2 rotate-45 rounded-[2px] bg-foreground fill-foreground"
        style={({ placement, defaultStyle }) => ({
          ...defaultStyle,
          rotate: "0deg",
          translate: "0 0",
          transform:
            placement === "bottom"
              ? "translate(-50%, calc(50% + 2px)) rotate(45deg)"
              : placement === "top"
                ? "translate(-50%, calc(-50% - 2px)) rotate(45deg)"
                : placement === "left"
                  ? "translate(calc(-50% - 2px), -50%) rotate(45deg)"
                  : "translate(calc(50% + 2px), -50%) rotate(45deg)",
        })}
      />
    </TooltipPrimitive>
  )
}

export { Tooltip, TooltipTrigger }
