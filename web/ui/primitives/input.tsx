"use client"

import * as React from "react"
import { cn } from '@ui/lib/utils'
import {
  composeRenderProps,
  Input as InputPrimitive,
} from "react-aria-components"

/** Shared look of text controls (Input, Textarea, Select trigger): Graphite density, tokens only. */
const fieldControlClass =
  "w-full min-w-0 rounded-md border border-input bg-card text-sm text-foreground transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-primary focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:focus-visible:ring-destructive/30 data-invalid:border-destructive motion-reduce:transition-none"

const inputSizeClass = {
  sm: "h-control-sm px-2 text-xs pointer-coarse:h-control-touch pointer-coarse:text-base",
  md: "h-control-md px-2 pointer-coarse:h-control-touch pointer-coarse:text-base",
} as const

function Input({
  className,
  type,
  size = "md",
  ...props
}: Omit<React.ComponentProps<typeof InputPrimitive>, "size"> & { size?: "sm" | "md" }) {
  return (
    <InputPrimitive
      type={type}
      data-slot="input"
      className={composeRenderProps(className, (className) =>
        cn(
          fieldControlClass,
          inputSizeClass[size],
          "py-0 file:inline-flex file:h-5 file:border-0 file:bg-transparent file:text-xs file:font-medium file:text-foreground",
          className
        )
      )}
      {...props}
    />
  )
}

export { Input, fieldControlClass, inputSizeClass }
