"use client"

import * as React from "react"
import { cn } from '@ui/lib/utils'
import {
  Dialog as AlertDialogPrimitive,
  DialogTrigger as AlertDialogTriggerPrimitive,
  Heading,
  ModalOverlay as ModalOverlayPrimitive,
  Modal as ModalPrimitive,
  type DialogProps as AlertDialogPrimitiveProps,
  type DialogTriggerProps as AlertDialogTriggerPrimitiveProps,
  type ModalOverlayProps as ModalOverlayPrimitiveProps,
} from "react-aria-components"

import { Button } from "@ui/primitives/button"

function AlertDialogTrigger({ ...props }: AlertDialogTriggerPrimitiveProps) {
  return (
    <AlertDialogTriggerPrimitive data-slot="alert-dialog-trigger" {...props} />
  )
}

function AlertDialogOverlay({
  className,
  children,
  ...props
}: Omit<ModalOverlayPrimitiveProps, "className" | "children"> & {
  className?: string
  children: React.ReactNode
}) {
  return (
    <ModalOverlayPrimitive
      data-slot="alert-dialog-overlay"
      className={cn(
        "fixed inset-0 isolate z-(--pdt-z-dialog) bg-backdrop duration-(--pdt-duration) data-entering:animate-in data-entering:fade-in-0 data-exiting:animate-out data-exiting:fade-out-0 motion-reduce:animate-none",
        className
      )}
      {...props}
    >
      {children}
    </ModalOverlayPrimitive>
  )
}

function AlertDialog({
  className,
  size = "default",
  children,
  dialogProps,
  ...props
}: Omit<ModalOverlayPrimitiveProps, "className" | "children"> &
  Pick<React.ComponentProps<typeof ModalPrimitive>, "isDismissable"> & {
    className?: string
    size?: "default" | "sm"
    children: React.ReactNode
    /** Props for the inner React Aria Dialog (aria-describedby). */
    dialogProps?: Omit<AlertDialogPrimitiveProps, "children" | "className" | "style" | "role">
  }) {
  return (
    <AlertDialogOverlay {...props}>
      <ModalPrimitive
        data-slot="alert-dialog-content"
        data-size={size}
        className={cn(
          "group/alert-dialog-content fixed top-1/2 left-1/2 grid w-full -translate-x-1/2 -translate-y-1/2 gap-4 rounded-md border border-border bg-popover p-4 text-popover-foreground shadow-dialog duration-(--pdt-duration) outline-none motion-reduce:animate-none data-entering:animate-in data-entering:fade-in-0 data-entering:zoom-in-95 data-exiting:animate-out data-exiting:fade-out-0 data-exiting:zoom-out-95 data-[size=default]:max-w-xs data-[size=sm]:max-w-xs data-[size=default]:sm:max-w-sm",
          className
        )}
      >
        <AlertDialogPrimitive
          data-slot="alert-dialog"
          role="alertdialog"
          {...dialogProps}
          className="[display:inherit] [gap:inherit] outline-none"
        >
          {children}
        </AlertDialogPrimitive>
      </ModalPrimitive>
    </AlertDialogOverlay>
  )
}

function AlertDialogContent({
  className,
  size = "default",
  children,
  ...props
}: React.ComponentProps<typeof AlertDialog>) {
  return (
    <AlertDialog className={className} size={size} {...props}>
      {children}
    </AlertDialog>
  )
}

function AlertDialogHeader({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-dialog-header"
      className={cn(
        "grid grid-rows-[auto_1fr] place-items-center gap-1.5 text-center has-data-[slot=alert-dialog-media]:grid-rows-[auto_auto_1fr] has-data-[slot=alert-dialog-media]:gap-x-4 sm:group-data-[size=default]/alert-dialog-content:place-items-start sm:group-data-[size=default]/alert-dialog-content:text-left sm:group-data-[size=default]/alert-dialog-content:has-data-[slot=alert-dialog-media]:grid-rows-[auto_1fr]",
        className
      )}
      {...props}
    />
  )
}

function AlertDialogFooter({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-dialog-footer"
      className={cn(
        "-mx-4 -mb-4 flex flex-col-reverse gap-2 rounded-b-md border-t border-border bg-muted p-3 group-data-[size=sm]/alert-dialog-content:grid group-data-[size=sm]/alert-dialog-content:grid-cols-2 sm:flex-row sm:justify-end",
        className
      )}
      {...props}
    />
  )
}

function AlertDialogMedia({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-dialog-media"
      className={cn(
        "mb-2 inline-flex size-10 items-center justify-center rounded-md bg-muted sm:group-data-[size=default]/alert-dialog-content:row-span-2 *:[svg:not([class*='size-'])]:size-6",
        className
      )}
      {...props}
    />
  )
}

function AlertDialogTitle({
  className,
  ...props
}: Omit<React.ComponentProps<typeof Heading>, "slot">) {
  return (
    <Heading
      slot="title"
      data-slot="alert-dialog-title"
      className={cn(
        "font-heading text-base font-medium sm:group-data-[size=default]/alert-dialog-content:group-has-data-[slot=alert-dialog-media]/alert-dialog-content:col-start-2",
        className
      )}
      {...props}
    />
  )
}

function AlertDialogDescription({
  className,
  ...props
}: Omit<React.ComponentProps<"div">, "slot">) {
  return (
    <div
      data-slot="alert-dialog-description"
      className={cn(
        "text-sm text-balance text-muted-foreground md:text-pretty *:[a]:underline *:[a]:underline-offset-3 *:[a]:hover:text-foreground",
        className
      )}
      {...props}
    />
  )
}

function AlertDialogAction({
  className,
  ...props
}: React.ComponentProps<typeof Button>) {
  return (
    <Button
      slot="close"
      data-slot="alert-dialog-action"
      className={cn(className)}
      {...props}
    />
  )
}

function AlertDialogCancel({
  className,
  variant = "outline",
  size = "default",
  ...props
}: React.ComponentProps<typeof Button>) {
  return (
    <Button
      slot="close"
      data-slot="alert-dialog-cancel"
      className={cn(className)}
      variant={variant}
      size={size}
      {...props}
    />
  )
}

export {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogOverlay,
  AlertDialogTitle,
  AlertDialogTrigger,
}
