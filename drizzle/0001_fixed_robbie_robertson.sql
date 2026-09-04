ALTER TABLE "tracks" ADD COLUMN "title" text;--> statement-breakpoint
ALTER TABLE "tracks" ADD COLUMN "artist" text;--> statement-breakpoint
ALTER TABLE "tracks" ADD COLUMN "album" text;--> statement-breakpoint
ALTER TABLE "tracks" ADD COLUMN "duration" integer;--> statement-breakpoint
ALTER TABLE "tracks" ADD COLUMN "bit_depth" integer;--> statement-breakpoint
ALTER TABLE "tracks" ADD COLUMN "sample_rate" integer;--> statement-breakpoint
CREATE INDEX "tracks_title_idx" ON "tracks" USING btree ("title");--> statement-breakpoint
CREATE INDEX "tracks_artist_idx" ON "tracks" USING btree ("artist");