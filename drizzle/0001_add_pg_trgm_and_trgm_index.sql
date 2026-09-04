CREATE EXTENSION IF NOT EXISTS pg_trgm;--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "tracks_search_trgm_idx" ON "tracks" USING gin (("title" || ' ' || "artist" || ' ' || "album") gin_trgm_ops);
