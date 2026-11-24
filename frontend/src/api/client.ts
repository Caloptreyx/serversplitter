import { z } from 'zod';
import { axiosInstance } from '@/api/axios.ts';
import { serverSchema } from '@/lib/schemas/server/server.ts';
import { parseFromApi } from '@/lib/serialization/api-transform.ts';

export type Server = z.infer<typeof serverSchema>;

/**
 * A split's limits as the splitter granted them (the ledger). `cpu` is a percentage, `memory`, `swap`
 * and `disk` are MB. In a `max`, `-1` means unlimited.
 */
export interface Amounts {
  cpu: number;
  memory: number;
  swap: number;
  disk: number;
  allocations: number;
  databases: number;
  backups: number;
  schedules: number;
}

/** cpu %, memory/swap/disk MB. */
export interface ResourceAmounts {
  cpu: number;
  memory: number;
  swap: number;
  disk: number;
}

export type MasterPowerAction = 'restart' | 'stop' | 'kill';
export type MasterPower = 'none' | MasterPowerAction;

/**
 * What the user may pick after a split change. `allowed` is already filtered by the admin setting, the
 * user's power permission on the master and the master's state.
 */
export interface MasterPowerOptions {
  allowed: MasterPowerAction[];
  default: MasterPower;
}

/** Result of the power action sent after the split change committed. A failure never undoes the split change. */
export interface MasterPowerResult {
  action: MasterPowerAction;
  ok: boolean;
  error: string | null;
}

/** cpu, memory, swap or disk of a master's pool. All values are MB or %, `0` = unlimited. */
export interface PoolResource {
  /** The master's own limit right now (`0` = unlimited, `-1` = unlimited swap). */
  master: number;
  /** Part of `master` that can never be handed out. */
  reserved: number;
  /** Sum of what the splits took from the master. */
  splits: number;
  /** `master + splits`, the whole pool (`0` when the master is unlimited). */
  total: number;
  /** The most a new split may take (`-1` = unlimited). */
  free: number;
}

/** allocations, databases, backups or schedules of a master's pool. */
export interface FeaturePool {
  /** The master's own limit. */
  master: number;
  /** What the master itself uses of `master`. */
  used: number;
  /** Sum the splits were granted. */
  splits: number;
  /** The most a new split may take. For allocations: when it gets a new port from the node. */
  free: number;
}

export interface SplitterPort {
  /** server allocation uuid, as in `server.allocation.uuid` */
  uuid: string;
  ip: string;
  ip_alias: string | null;
  port: number;
  notes: string | null;
}

export type SplitHealth = 'balanced' | 'drift' | 'untracked';
export type SwapMode = 'pooled' | 'none' | 'copy';

export interface PoolHealth {
  status: 'balanced' | 'drift';
  /** Human readable, one per problem. */
  issues: string[];
}

export interface Pool {
  cpu: PoolResource;
  memory: PoolResource;
  swap: PoolResource;
  disk: PoolResource;
  allocations: FeaturePool;
  databases: FeaturePool;
  backups: FeaturePool;
  schedules: FeaturePool;
  splits: { used: number; limit: number };
  /** Smallest cpu/memory/disk a split of this master may have (0 where the master is unlimited). */
  minimum: { cpu: number; memory: number; disk: number };
  /** `allocations.free` when the new split takes one of `transferable_ports` instead of a new port. */
  allocations_free_with_master_port: number;
  /** The master's non-primary ports a new split can take over as its primary port. */
  transferable_ports: SplitterPort[];
  swap_mode: SwapMode;
  /** When false, `reserved` is shown as free space (admin setting "display reserved limits"). */
  display_reserved: boolean;
  health: PoolHealth;
  /** What the splits must free because the master's plan is smaller than they hold + the reserve. */
  overcommit: ResourceAmounts | null;
}

export interface SplitEntry {
  server: Server;
  /** What the splitter granted the split. */
  ledger: Amounts;
  /** Resize bounds: never below what the split uses or the minimum size, never above pool + holding (`-1` = unlimited). */
  min: Amounts;
  max: Amounts;
  /** The split's primary port came from the master; deleting it can give it back. */
  port_from_master: boolean;
  /** Master subusers mirrored on this split. */
  synced_subusers: number;
  /** Users whose split permissions are kept as they are (not mirrored). */
  custom_subusers: number;
  health: SplitHealth;
  suspended_by_master: boolean;
}

export interface SplitShare {
  ledger: Amounts;
  health: SplitHealth;
  suspended_by_master: boolean;
}

export interface ParentServer {
  uuid: string;
  name: string;
}

export interface SplitterClientIndex {
  role: 'master' | 'split';
  /** On a split: its master. */
  parent: ParentServer | null;
  /** On a master: its pool (`null` on a split). */
  pool: Pool | null;
  /** On a master: its splits owned by the master's owner. */
  splits: SplitEntry[];
  /** On a split: what it holds of its master's pool. */
  share: SplitShare | null;
  /** On a master: the power actions offered after a split change (`null` on a split). */
  master_power: MasterPowerOptions | null;
}

export interface EggVariable {
  env_variable: string;
  name: string;
  description: string | null;
  default_value: string | null;
  rules: string[];
}

export interface NestEggItem {
  uuid: string;
  name: string;
  description: string | null;
  /** User-editable variables of the egg, in display order. */
  variables: EggVariable[];
}

export interface FeatureLimitsInput {
  allocations: number;
  databases: number;
  backups: number;
  schedules: number;
}

export type PortChoice = { type: 'master'; allocation_uuid: string } | { type: 'node' };

export interface CreateSplitPayload {
  name: string;
  description?: string;
  egg_uuid: string;
  cpu: number;
  memory: number;
  disk: number;
  feature_limits: FeatureLimitsInput;
  port: PortChoice;
  variables?: { env_variable: string; value: string }[];
  /** Copy the master's subusers; needs `subusers.create` (the API answers 403 otherwise). */
  sync_subusers: boolean;
  master_power?: MasterPower;
}

export interface UpdateSplitPayload {
  name?: string;
  /** An empty string clears it. */
  description?: string;
  cpu?: number;
  memory?: number;
  disk?: number;
  feature_limits?: FeatureLimitsInput;
  master_power?: MasterPower;
}

export interface SplitChangeResult {
  server: Server;
  masterPower: MasterPowerResult | null;
}

export interface SyncResult {
  added: number;
  updated: number;
  /** Emails of master subusers left out because the acting user holds fewer permissions than they do. */
  skipped: string[];
}

export interface SplitSubuser {
  user: { uuid: string; username: string; email: string };
  on_master: boolean;
  on_split: boolean;
  /** Kept as set on the split: never re-added or changed by syncing. Removing from the master still removes. */
  custom: boolean;
}

const base = (server: string) => `/api/client/servers/${server}/splitter`;

// Wire format is snake_case (no key transform), except `server`, which goes through the panel's server schema.

export async function getClientSplitter(server: string): Promise<SplitterClientIndex> {
  const { data } = await axiosInstance.get(base(server));
  return {
    ...data,
    splits: (data.splits ?? []).map((entry: SplitEntry & { server: unknown }) => ({
      ...entry,
      server: parseFromApi(serverSchema, entry.server),
    })),
  };
}

export async function getClientSplitterNests(server: string): Promise<Record<string, NestEggItem[]>> {
  const { data } = await axiosInstance.get(`${base(server)}/nests`);
  return data;
}

function toSplitChangeResult(data: Record<string, unknown>): SplitChangeResult {
  const { master_power: masterPower, ...server } = data;
  return {
    server: parseFromApi(serverSchema, server),
    masterPower: (masterPower as MasterPowerResult | null | undefined) ?? null,
  };
}

export async function createSplit(server: string, payload: CreateSplitPayload): Promise<SplitChangeResult> {
  const { data } = await axiosInstance.post(base(server), payload);
  return toSplitChangeResult(data);
}

export async function updateSplit(
  server: string,
  split: string,
  payload: UpdateSplitPayload,
): Promise<SplitChangeResult> {
  const { data } = await axiosInstance.patch(`${base(server)}/${split}`, payload);
  return toSplitChangeResult(data);
}

/** `null` when no power action was requested (204). */
export async function deleteSplit(
  server: string,
  split: string,
  returnPort: boolean,
  masterPower: MasterPower = 'none',
): Promise<MasterPowerResult | null> {
  const params: Record<string, string | boolean> = { return_port: returnPort };
  if (masterPower !== 'none') params.master_power = masterPower;
  const { data } = await axiosInstance.delete(`${base(server)}/${split}`, { params });
  return data?.master_power ?? null;
}

export async function syncSubusers(server: string, split: string): Promise<SyncResult> {
  const { data } = await axiosInstance.post(`${base(server)}/${split}/subusers-sync`);
  return data;
}

export async function getSplitSubusers(server: string, split: string): Promise<SplitSubuser[]> {
  const { data } = await axiosInstance.get(`${base(server)}/${split}/subusers`);
  return data.subusers;
}

export async function setSplitSubuserCustom(server: string, split: string, user: string, custom: boolean) {
  await axiosInstance.put(`${base(server)}/${split}/subusers/${user}/custom`, { custom });
}
