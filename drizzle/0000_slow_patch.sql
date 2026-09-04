CREATE TABLE "requests" (
	"id" serial PRIMARY KEY NOT NULL,
	"telegram_id" bigint NOT NULL,
	"chat_id" bigint NOT NULL,
	"apple_track_id" text NOT NULL,
	"is_cache_hit" boolean NOT NULL,
	"duration_ms" integer,
	"status" text NOT NULL,
	"error_reason" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "tracks" (
	"id" serial PRIMARY KEY NOT NULL,
	"apple_track_id" text NOT NULL,
	"message_id" integer NOT NULL,
	"file_id" text NOT NULL,
	"file_unique_id" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "tracks_apple_track_id_unique" UNIQUE("apple_track_id")
);
--> statement-breakpoint
CREATE TABLE "users" (
	"telegram_id" bigint PRIMARY KEY NOT NULL,
	"name" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE INDEX "requests_apple_track_id_idx" ON "requests" USING btree ("apple_track_id");--> statement-breakpoint
CREATE INDEX "requests_telegram_id_idx" ON "requests" USING btree ("telegram_id");--> statement-breakpoint
CREATE INDEX "requests_created_at_idx" ON "requests" USING btree ("created_at");