import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { formatJsonValue } from "@/lib/format"
import { cn } from "@/lib/utils"

export interface FieldRow {
  field: string
  value: unknown
  omitted: boolean
}

export function FieldTable({
  rows,
  caption,
}: {
  rows: FieldRow[]
  caption?: string
}) {
  return (
    <Table>
      {caption ? <caption className="sr-only">{caption}</caption> : null}
      <TableHeader>
        <TableRow>
          <TableHead className="font-mono text-[11px] tracking-wide text-muted-foreground">
            field
          </TableHead>
          <TableHead className="font-mono text-[11px] tracking-wide text-muted-foreground">
            value
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((row) => (
          <TableRow key={row.field}>
            <TableCell className="font-mono text-xs text-muted-foreground">
              {row.field}
            </TableCell>
            <TableCell
              className={cn(
                "max-w-[36rem] font-mono text-xs break-all whitespace-pre-wrap",
                row.omitted && "text-muted-foreground"
              )}
            >
              {row.omitted ? "omitted" : formatJsonValue(row.value)}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
