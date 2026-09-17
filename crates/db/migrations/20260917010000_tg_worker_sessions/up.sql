CREATE TABLE tg_worker_sessions (
    bot_token_hash VARCHAR(64) PRIMARY KEY,
    session_data TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
