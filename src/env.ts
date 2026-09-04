import { z } from 'zod'

const r = z
  .object({
    API_ID: z.coerce.number(),
    API_HASH: z.string(),
    BOT_TOKEN: z.string(),
    ADMIN_ID: z.coerce.number(),
    DUMP_CHANNEL_ID: z.union([z.coerce.number(), z.string()]),
    DATABASE_URL: z.string().optional(),
    DATABASE_DIR: z.string().default('./bot-data/db'),
  })
  .safeParse(process.env)

if (!r.success) {
  throw new Error(`Invalid env:\n${z.prettifyError(r.error)}`)
}

export const env = r.data
