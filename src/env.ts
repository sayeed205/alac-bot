import { z } from 'zod'

const r = z
  .object({
    API_ID: z.coerce.number(),
    API_HASH: z.string(),
    BOT_TOKEN: z.string(),
    ADMIN_ID: z.coerce.number(),
    DUMP_CHANNEL_ID: z.union([z.coerce.number(), z.string()]),
    DATABASE_URL: z
      .string()
      .default('postgresql://admin:password@localhost:5432/alac_bot'),
    ALAC_MIRROR_URL: z.string().optional(),
    ALAC_API_KEY: z.string().optional(),
    ALAC_WRAPPER_URL: z.string().default('http://127.0.0.1:12340'),
    ALAC_WRAPPER_API_KEY: z.string().optional(),
    LOG_LEVEL: z
      .enum(['trace', 'debug', 'info', 'warn', 'error', 'critical'])
      .default('info'),
  })
  .safeParse(process.env)

if (!r.success) {
  throw new Error(`Invalid env:\n${z.prettifyError(r.error)}`)
}

export const env = r.data
