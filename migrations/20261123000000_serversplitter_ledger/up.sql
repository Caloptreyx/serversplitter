-- What every split was granted and what of it was taken from its master's limits. Deleting a split returns
-- exactly the charged amounts; limits changed outside the splitter show up as drift instead of growing the pool.
CREATE TABLE IF NOT EXISTS "serversplitter_splits" (
    "split_uuid" UUID PRIMARY KEY REFERENCES "servers"("uuid") ON DELETE CASCADE,
    "master_uuid" UUID NOT NULL REFERENCES "servers"("uuid") ON DELETE CASCADE,
    "cpu" INTEGER NOT NULL,
    "memory" BIGINT NOT NULL,
    "swap" BIGINT NOT NULL,
    "disk" BIGINT NOT NULL,
    "allocations" INTEGER NOT NULL,
    "databases" INTEGER NOT NULL,
    "backups" INTEGER NOT NULL,
    "schedules" INTEGER NOT NULL,
    "charged_cpu" INTEGER NOT NULL,
    "charged_memory" BIGINT NOT NULL,
    "charged_swap" BIGINT NOT NULL,
    "charged_disk" BIGINT NOT NULL,
    "port_from_master" BOOLEAN NOT NULL DEFAULT FALSE,
    "created" TIMESTAMP NOT NULL DEFAULT NOW(),
    "updated" TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS "serversplitter_splits_master_uuid_idx" ON "serversplitter_splits"("master_uuid");

-- Existing splits: granted = current limits. A resource counts as charged the way 1.5.2 credited it back on delete
-- (both master and split limited). Swap was never taken from the master before.
INSERT INTO "serversplitter_splits" (
    "split_uuid", "master_uuid", "cpu", "memory", "swap", "disk", "allocations", "databases", "backups", "schedules",
    "charged_cpu", "charged_memory", "charged_swap", "charged_disk"
)
SELECT
    s."uuid", s."parent_uuid", s."cpu", s."memory", s."swap", s."disk",
    s."allocation_limit", s."database_limit", s."backup_limit", s."schedule_limit",
    CASE WHEN p."cpu" > 0 AND s."cpu" > 0 THEN s."cpu" ELSE 0 END,
    CASE WHEN p."memory" > 0 AND s."memory" > 0 THEN s."memory" ELSE 0 END,
    0,
    CASE WHEN p."disk" > 0 AND s."disk" > 0 THEN s."disk" ELSE 0 END
FROM "servers" s
JOIN "servers" p ON p."uuid" = s."parent_uuid"
ON CONFLICT DO NOTHING;

-- Split subusers whose permissions are kept as set on the split: syncing never re-adds or changes them.
CREATE TABLE IF NOT EXISTS "serversplitter_custom_subusers" (
    "split_uuid" UUID NOT NULL REFERENCES "servers"("uuid") ON DELETE CASCADE,
    "user_uuid" UUID NOT NULL REFERENCES "users"("uuid") ON DELETE CASCADE,
    "created" TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY ("split_uuid", "user_uuid")
);

-- Keep what owners customised: split subusers whose permissions differ from theirs on the master.
INSERT INTO "serversplitter_custom_subusers" ("split_uuid", "user_uuid")
SELECT ss."server_uuid", ss."user_uuid"
FROM "server_subusers" ss
JOIN "servers" s ON s."uuid" = ss."server_uuid"
JOIN "server_subusers" ms ON ms."server_uuid" = s."parent_uuid" AND ms."user_uuid" = ss."user_uuid"
WHERE ms."permissions" IS DISTINCT FROM ss."permissions"
    OR ms."ignored_files" IS DISTINCT FROM ss."ignored_files"
ON CONFLICT DO NOTHING;

-- Set on splits suspended because their master was; unsuspending the master lifts only these.
ALTER TABLE "servers" ADD COLUMN IF NOT EXISTS "split_suspended_by_master" BOOLEAN NOT NULL DEFAULT FALSE;
