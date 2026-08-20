export function formatUnixMicros(micros: number): string {
  if (!Number.isInteger(micros) || micros < 0) {
    return String(micros)
  }
  const millis = Math.trunc(micros / 1000)
  const date = new Date(millis)
  if (Number.isNaN(date.getTime())) {
    return `${micros}`
  }
  return `${date.toISOString()} · ${micros}`
}

export function formatOmitted(field: string): string {
  return `${field} omitted`
}

export function formatDiskFree(basisPoints: number): string {
  return `${(basisPoints / 100).toFixed(2)}% (${basisPoints} bp)`
}

export function formatJsonValue(value: unknown): string {
  if (value === undefined) {
    return "omitted"
  }
  if (typeof value === "string") {
    return value
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value)
  }
  return JSON.stringify(value)
}
