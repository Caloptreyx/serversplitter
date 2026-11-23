import { axiosInstance } from '@/api/axios.ts';
import type { Amounts, FeatureLimitsInput, ParentServer, SplitHealth, SwapMode } from './client.ts';

// Wire format is snake_case (no key transform).

const base = '/api/admin/extensions/com.caloptreyx.serversplitter';

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

export interface SplitterSettings {
  reserved_cpu: number;
  reserved_memory: number;
  reserved_disk: number;
  /** Smallest split size. `null` = same as the reserve (the behaviour before these fields existed). */
  minimum_cpu: number | null;
  minimum_memory: number | null;
  minimum_disk: number | null;
  default_splits: number;
  include_disk_usage: boolean;
  display_reserved_limits: boolean;
  /** Suspending or unsuspending a master does the same to its splits. */
  cascade_suspension: boolean;
  /** Changing a master's owner moves its splits to the new owner. */
  cascade_owner: boolean;
  /** Permission changes and removals of master subusers reach the splits they were synced to. */
  mirror_subusers: boolean;
  /** Splits get the master's CPU pinning. */
  inherit_pinned_cpus: boolean;
  swap_mode: SwapMode;
}

export interface AdminSettingsResponse extends SplitterSettings {
  egg_rules: EggRule[];
  eggs: EggItem[];
}

export type UpdateAdminSettingsPayload = SplitterSettings;

export interface EggRulePayload {
  eggs: string[];
  allowed_eggs: string[];
}

export interface AdminSplitSummary {
  uuid: string;
  name: string;
  /** The split's current limits. */
  limits: Amounts;
  /** What the splitter granted it (`null` when untracked). */
  ledger: Amounts | null;
  min: Amounts;
  max: Amounts;
  health: SplitHealth;
  suspended: boolean;
}

export interface AdminServerSplits {
  /** The master server, when this server is itself a split. */
  parent: ParentServer | null;
  /** This server's splits, when it is a master server. */
  splits: AdminSplitSummary[];
  /** `feature_limits.splits` of a master. */
  splits_limit: number;
}

export interface AdminResizePayload {
  cpu?: number;
  memory?: number;
  disk?: number;
  feature_limits?: FeatureLimitsInput;
}

export interface PoolIssue {
  split_uuid: string;
  split_name: string;
  /** `drift`: limits differ from the ledger; `untracked`: no ledger row; `orphaned`: ledger row for a server that isn't this master's split. */
  kind: 'drift' | 'untracked' | 'orphaned';
  detail: string;
}

export interface AdminMasterRow {
  uuid: string;
  uuid_short: string;
  name: string;
  owner: { uuid: string; username: string };
  node: { uuid: string; name: string };
  splits_used: number;
  splits_limit: number;
  /** The master's own limits. */
  master: Amounts;
  /** Sum of what its splits were granted. */
  held: Amounts;
  status: 'balanced' | 'drift';
  issues: PoolIssue[];
}

export interface AdminMastersPage {
  masters: AdminMasterRow[];
  total: number;
  page: number;
  per_page: number;
}

export interface ReconcileResult {
  fixed: number;
  failed: { split_uuid: string; split_name: string; reason: string }[];
}

export async function getAdminSplitterSettings(): Promise<AdminSettingsResponse> {
  const { data } = await axiosInstance.get(`${base}/settings`);
  return data;
}

export async function updateAdminSplitterSettings(payload: UpdateAdminSettingsPayload): Promise<void> {
  await axiosInstance.put(`${base}/settings`, payload);
}

export async function createAdminEggRule(payload: EggRulePayload): Promise<EggRule> {
  const { data } = await axiosInstance.post(`${base}/egg-rules`, payload);
  return data;
}

export async function updateAdminEggRule(ruleId: string, payload: EggRulePayload): Promise<void> {
  await axiosInstance.put(`${base}/egg-rules/${ruleId}`, payload);
}

export async function deleteAdminEggRule(ruleId: string): Promise<void> {
  await axiosInstance.delete(`${base}/egg-rules/${ruleId}`);
}

export async function getAdminServerSplits(server: string): Promise<AdminServerSplits> {
  const { data } = await axiosInstance.get(`${base}/servers/${server}`);
  return data;
}

/** Resizes a split through its master's pool. `split` is the split's uuid. */
export async function adminResizeSplit(split: string, payload: AdminResizePayload): Promise<void> {
  await axiosInstance.patch(`${base}/servers/${split}/resize`, payload);
}

export async function getAdminMasters(params: {
  page: number;
  per_page: number;
  search?: string;
  status?: 'drift';
}): Promise<AdminMastersPage> {
  const { data } = await axiosInstance.get(`${base}/splits`, { params });
  return data;
}

export async function reconcileMaster(master: string): Promise<ReconcileResult> {
  const { data } = await axiosInstance.post(`${base}/splits/${master}/reconcile`);
  return data;
}

export async function reconcileAll(): Promise<ReconcileResult> {
  const { data } = await axiosInstance.post(`${base}/splits/reconcile`);
  return data;
}
