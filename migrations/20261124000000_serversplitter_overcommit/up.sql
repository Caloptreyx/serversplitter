-- What the splits of an over-committed master still have to free: its plan total (set by an admin or a billing
-- panel) is below what its splits were charged plus its reserve. Resources its splits give back pay this down
-- before they reach the master.
CREATE TABLE IF NOT EXISTS "serversplitter_overcommit" (
    "master_uuid" UUID PRIMARY KEY REFERENCES "servers"("uuid") ON DELETE CASCADE,
    "cpu" INTEGER NOT NULL DEFAULT 0,
    "memory" BIGINT NOT NULL DEFAULT 0,
    "swap" BIGINT NOT NULL DEFAULT 0,
    "disk" BIGINT NOT NULL DEFAULT 0,
    "created" TIMESTAMP NOT NULL DEFAULT NOW(),
    "updated" TIMESTAMP NOT NULL DEFAULT NOW()
);
