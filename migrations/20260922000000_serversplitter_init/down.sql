DROP INDEX IF EXISTS "servers_parent_uuid_idx";
ALTER TABLE "servers" DROP COLUMN IF EXISTS "parent_uuid";
ALTER TABLE "servers" DROP COLUMN IF EXISTS "splits";
