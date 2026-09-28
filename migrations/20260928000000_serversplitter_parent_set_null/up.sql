-- Splits are deleted through the panel (Wings, database hosts) before their master. A split still
-- linked when the master's row goes becomes a standalone server instead of silently losing its row.
ALTER TABLE "servers" DROP CONSTRAINT IF EXISTS "servers_parent_uuid_fkey";
ALTER TABLE "servers" ADD CONSTRAINT "servers_parent_uuid_fkey" FOREIGN KEY ("parent_uuid") REFERENCES "servers"("uuid") ON DELETE SET NULL;
