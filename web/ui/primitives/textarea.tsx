import * as React from "react"
import { cn } from "cn"
import {
  composeRenderProps,
  TextArea as TextareaPrimitive,
} from "react-aria-components"

import { fieldControlClass } from "@ui/primitives/input"

function Textarea({
  className,
  ...props
}: React.ComponentProps<typeof TextareaPrimitive>) {
  return (
    <TextareaPrimitive
      data-slot="textarea"
      className={composeRenderProps(className, (className) =>
        cn(
          fieldControlClass,
          "flex field-sizing-content min-h-16 px-2 py-1.5 pointer-coarse:text-base",
          className
        )
      )}
      {...props}
    />
  )
}

export { Textarea }
