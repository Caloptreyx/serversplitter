ALTER TABLE "servers" DROP CONSTRAINT IF EXISTS "servers_parent_uuid_fkey";
ALTER TABLE "servers" ADD CONSTRAINT "servers_parent_uuid_fkey" FOREIGN KEY ("parent_uuid") REFERENCES "servers"("uuid") ON DELETE CASCADE;
