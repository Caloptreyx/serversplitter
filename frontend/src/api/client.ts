import { z } from 'zod';
import { axiosInstance } from '@/api/axios.ts';
import { serverSchema } from '@/lib/schemas/server/server.ts';
import { parseFromApi } from '@/lib/serialization/api-transform.ts';

export type Server = z.infer<typeof serverSchema>;

export interface EggItem {
  uuid: string;
  name: string;
  nest_uuid: string;
  nest_name: string;
}

export interface EggRule {
  id: string;
  eggs: string[];
  allowed_eggs: string[];
}

export interface AdminSettingsResponse {
  reserved_cpu: number;
  reserved_memory: number;
  reserved_disk: number;
  include_disk_usage: boolean;
  display_reserved_limits: boolean;
  default_splits: number;
  egg_rules: EggRule[];
  eggs: EggItem[];
}

export interface UpdateAdminSettingsPayload {
  reserved_cpu: number;
  reserved_memory: number;
  reserved_disk: number;
  include_disk_usage: boolean;
  display_reserved_limits: boolean;
  default_splits: number;
}

export interface CreateEggRulePayload {
  eggs: string[];
  allowed_eggs: string[];
}

export interface UpdateEggRulePayload {
  eggs: string[];
  allowed_eggs: string[];
}

export interface SplitterFeatureLimits {
  allocations: number;
  databases: number;
  backups: number;
  schedules: number;
  splits: number;
}

/**
 * In `remaining`, `cpu`/`memory`/`disk` of `-1` mean the master is unlimited for that resource.
 * In `total`, `0` means unlimited.
 */
export interface SplitterResourceLimits {
  cpu: number;
  memory: number;
  disk: number;
  feature_limits: SplitterFeatureLimits;
}

export interface ReservedLimits {
  cpu: number;
  memory: number;
  disk: number;
}

export interface ResourcesData {
  total: SplitterResourceLimits;
  /** Exact maximums the backend accepts for a new split (`-1` = unlimited). */
  remaining: SplitterResourceLimits;
  /** Display-only numbers for the pool stat cards; never use for form bounds. */
  remaining_display: SplitterResourceLimits;
  reserved: ReservedLimits;
  /** When true, one allocation of `remaining.feature_limits.allocations` is not available to resizes. */
  transferable_allocation: boolean;
}

export interface NestEggItem {
  uuid: string;
  name: string;
  description: string | null;
}

export interface ParentServer {
  uuid: string;
  name: string;
}

export interface SplitterClientIndex {
  /** `null` on a child server: splits are managed from the master server only. */
  resources: ResourcesData | null;
  parent: ParentServer | null;
  subservers: Server[];
}

export interface SplitSummary {
  uuid: string;
  name: string;
  cpu: number;
  memory: number;
  disk: number;
}

export interface AdminServerSplits {
  /** The master server, when this server is itself a split. */
  parent: ParentServer | null;
  /** This server's splits, when it is a master server. */
  splits: SplitSummary[];
}

export interface CreateSplitPayload {
  name: string;
  description?: string;
  egg_uuid: string;
  cpu: number;
  memory: number;
  disk: number;
  feature_limits: {
    allocations: number;
    databases: number;
    backups: number;
    schedules: number;
  };
  sync_subusers: boolean;
}

export interface UpdateSplitPayload {
  name?: string;
  description?: string;
  cpu: number;
  memory: number;
  disk: number;
  feature_limits: {
    allocations: number;
    databases: number;
    backups: number;
    schedules: number;
  };
}

// Client API Calls
export async function getClientSplitter(serverUuid: string): Promise<SplitterClientIndex> {
  const { data } = await axiosInstance.get(`/api/client/servers/${serverUuid}/splitter`);
  return {
    resources: data.resources,
    parent: data.parent,
    subservers: data.servers.map((s: unknown) => parseFromApi(serverSchema, s)),
  };
}

export async function getClientSplitterNests(serverUuid: string): Promise<Record<string, NestEggItem[]>> {
  const { data } = await axiosInstance.get(`/api/client/servers/${serverUuid}/splitter/nests`);
  return data;
}

export async function createSplit(serverUuid: string, payload: CreateSplitPayload): Promise<Server> {
  const { data } = await axiosInstance.post(`/api/client/servers/${serverUuid}/splitter`, payload);
  return parseFromApi(serverSchema, data);
}

export async function updateSplit(
  serverUuid: string,
  subserverUuid: string,
  payload: UpdateSplitPayload,
): Promise<Server> {
  const { data } = await axiosInstance.patch(`/api/client/servers/${serverUuid}/splitter/${subserverUuid}`, payload);
  return parseFromApi(serverSchema, data);
}

export async function deleteSplit(serverUuid: string, subserverUuid: string): Promise<void> {
  await axiosInstance.delete(`/api/client/servers/${serverUuid}/splitter/${subserverUuid}`);
}

export async function syncSubusers(serverUuid: string, subserverUuid: string): Promise<void> {
  await axiosInstance.post(`/api/client/servers/${serverUuid}/splitter/${subserverUuid}/subusers-sync`);
}

// Admin API Calls
export async function getAdminSplitterSettings(): Promise<AdminSettingsResponse> {
  const { data } = await axiosInstance.get('/api/admin/extensions/com.caloptreyx.serversplitter/settings');
  return data;
}

export async function updateAdminSplitterSettings(payload: UpdateAdminSettingsPayload): Promise<void> {
  await axiosInstance.put('/api/admin/extensions/com.caloptreyx.serversplitter/settings', payload);
}

export async function createAdminEggRule(payload: CreateEggRulePayload): Promise<EggRule> {
  const { data } = await axiosInstance.post('/api/admin/extensions/com.caloptreyx.serversplitter/egg-rules', payload);
  return data;
}

export async function updateAdminEggRule(ruleId: string, payload: UpdateEggRulePayload): Promise<void> {
  await axiosInstance.put(`/api/admin/extensions/com.caloptreyx.serversplitter/egg-rules/${ruleId}`, payload);
}

export async function deleteAdminEggRule(ruleId: string): Promise<void> {
  await axiosInstance.delete(`/api/admin/extensions/com.caloptreyx.serversplitter/egg-rules/${ruleId}`);
}

export async function getAdminServerSplits(serverUuid: string): Promise<AdminServerSplits> {
  const { data } = await axiosInstance.get(`/api/admin/extensions/com.caloptreyx.serversplitter/servers/${serverUuid}`);
  return data;
}
