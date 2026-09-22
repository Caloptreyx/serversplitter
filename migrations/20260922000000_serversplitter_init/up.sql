ALTER TABLE "servers" ADD COLUMN IF NOT EXISTS "parent_uuid" UUID REFERENCES "servers"("uuid") ON DELETE CASCADE;
ALTER TABLE "servers" ADD COLUMN IF NOT EXISTS "splits" INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS "servers_parent_uuid_idx" ON "servers"("parent_uuid");
