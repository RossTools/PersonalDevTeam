"use client"

// Written by hand on React Aria's Calendar (shadcn's generated calendar overwrites our edited primitives and uses newer
// pickers). One month, Monday first, ‹ › in the heading; cells style themselves from React Aria's data attributes.
import { cn } from '@ui/lib/utils'
import {
  Button,
  Calendar as CalendarPrimitive,
  CalendarCell as CalendarCellPrimitive,
  CalendarGrid,
  CalendarGridBody,
  CalendarGridHeader,
  CalendarHeaderCell,
  Heading,
  type CalendarCellProps,
  type CalendarProps,
  type DateValue,
} from "react-aria-components"
import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react"

const nav =
  "inline-flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-disabled:opacity-40 pointer-coarse:size-control-touch [&>svg]:size-4"

function Calendar<T extends DateValue>({ className, ...props }: Omit<CalendarProps<T>, "className"> & { className?: string }) {
  return (
    <CalendarPrimitive data-slot="calendar" firstDayOfWeek="mon" className={cn("flex flex-col gap-1", className)} {...props}>
      <header className="flex items-center gap-1 px-1">
        <Heading className="flex-1 text-sm font-semibold" />
        <Button slot="previous" aria-label="Previous month" className={nav}>
          <ChevronLeftIcon aria-hidden />
        </Button>
        <Button slot="next" aria-label="Next month" className={nav}>
          <ChevronRightIcon aria-hidden />
        </Button>
      </header>
      <CalendarGrid weekdayStyle="narrow" className="w-full border-collapse">
        <CalendarGridHeader>
          {(day) => <CalendarHeaderCell className="pb-1 text-meta font-normal text-muted-foreground">{day}</CalendarHeaderCell>}
        </CalendarGridHeader>
        <CalendarGridBody>{(date) => <CalendarCell date={date} />}</CalendarGridBody>
      </CalendarGrid>
    </CalendarPrimitive>
  )
}

/**
 * A day: muted when unavailable (past), amber with a dot under it today, filled `bg-primary` when chosen. Days of the
 * next or last month are left out.
 */
function CalendarCell({ className, ...props }: Omit<CalendarCellProps, "className"> & { className?: string }) {
  return (
    <CalendarCellPrimitive
      data-slot="calendar-cell"
      className={cn(
        "relative mx-auto flex size-8 cursor-default items-center justify-center rounded-md text-sm tabular-nums outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring pointer-coarse:size-10",
        "data-outside-month:invisible data-unavailable:text-muted-foreground data-unavailable:opacity-60 data-unavailable:hover:bg-transparent data-disabled:text-muted-foreground data-disabled:opacity-60",
        "data-today:font-semibold data-today:text-waiting data-today:after:absolute data-today:after:bottom-0.5 data-today:after:size-1 data-today:after:rounded-full data-today:after:bg-waiting",
        "data-selected:bg-primary data-selected:font-semibold data-selected:text-primary-foreground data-selected:opacity-100 data-selected:after:bg-primary-foreground data-selected:hover:bg-primary",
        className
      )}
      {...props}
    />
  )
}

export { Calendar, CalendarCell }
