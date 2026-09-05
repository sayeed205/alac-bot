import { html } from '@mtcute/bun'

export type FormattedString = ReturnType<typeof html>

/**
 * Sanitizes and sorts message entities for Telegram MTProto:
 * 1. Sorts entities strictly by offset ascending, then length descending (so outer containers come before inner elements).
 * 2. Deduplicates / removes redundant nested entities of the exact same type
 *    (e.g., bold inside bold) which Telegram MTProto rejects with ENTITY_BOUNDS_INVALID.
 */
export function sanitizeAndSortEntities<
  T extends { _: string; offset: number; length: number },
>(entities?: T[]): T[] | undefined {
  if (!entities || entities.length <= 1) return entities

  entities.sort((a, b) => {
    if (a.offset !== b.offset) return a.offset - b.offset
    return b.length - a.length
  })

  const sanitized: T[] = []
  for (const ent of entities) {
    const isRedundant = sanitized.some((prev) => {
      if (prev._ !== ent._) return false
      const prevEnd = prev.offset + prev.length
      const entEnd = ent.offset + ent.length
      return ent.offset >= prev.offset && entEnd <= prevEnd
    })
    if (!isRedundant) {
      sanitized.push(ent)
    }
  }
  return sanitized
}

/**
 * Parses dynamic HTML string into mtcute FormattedString while ensuring
 * all entities are properly ordered and valid according to Telegram MTProto rules.
 */
export function parseDynamicHtml(content: string): FormattedString {
  const parsed = html([content] as unknown as TemplateStringsArray)
  if (!parsed.entities || parsed.entities.length <= 1) {
    return parsed
  }
  return {
    text: parsed.text,
    entities: sanitizeAndSortEntities(parsed.entities),
  }
}
