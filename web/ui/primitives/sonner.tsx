"use client"

import { readThemePref } from "../theme"
import { Toaster as Sonner, type ToasterProps } from "sonner"
import { CircleCheckIcon, InfoIcon, TriangleAlertIcon, OctagonXIcon, Loader2Icon } from "lucide-react"

const Toaster = ({ ...props }: ToasterProps) => {
  // Our own theme preference (ui/theme.ts) instead of next-themes.
  const theme = readThemePref()

  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group"
      icons={{
        success: (
          <CircleCheckIcon className="size-4" />
        ),
        info: (
          <InfoIcon className="size-4" />
        ),
        warning: (
          <TriangleAlertIcon className="size-4" />
        ),
        error: (
          <OctagonXIcon className="size-4" />
        ),
        loading: (
          <Loader2Icon className="size-4 animate-spin motion-reduce:animate-none" />
        ),
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
          // Above dialogs and menus (tokens.css layers); inline so it beats Sonner's own z-index.
          zIndex: "var(--pdt-z-toast)",
        } as React.CSSProperties
      }
      toastOptions={{
        classNames: {
          toast: "cn-toast font-sans text-sm shadow-popover",
          description: "text-meta text-muted-foreground",
          success: "[&_[data-icon]]:text-running",
          error: "[&_[data-icon]]:text-failed",
          info: "[&_[data-icon]]:text-link",
          warning: "[&_[data-icon]]:text-waiting",
          actionButton: "bg-primary! text-primary-foreground!",
        },
      }}
      {...props}
    />
  )
}

export { Toaster }
