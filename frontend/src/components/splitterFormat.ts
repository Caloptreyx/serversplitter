import { bytesToString, mbToBytes } from '@/lib/format/size.ts';
import type { ResourceAmounts } from '../api/client.ts';

export type PoolResourceKey = 'cpu' | 'memory' | 'disk';

export const RESOURCE_LABELS: Record<PoolResourceKey, string> = {
  cpu: 'CPU',
  memory: 'Memory',
  disk: 'Disk',
};

/** Colours handed to splits in order, the same split keeps its colour in every bar. */
export const SPLIT_COLORS = ['violet.6', 'teal.6', 'orange.6', 'cyan.6', 'pink.6', 'lime.6', 'indigo.6', 'yellow.6'];

export function splitColor(index: number): string {
  return SPLIT_COLORS[index % SPLIT_COLORS.length];
}

/** Mantine colour (`teal.6`) as a CSS variable. */
export function colorVar(color: string): string {
  return `var(--mantine-color-${color.replace('.', '-')})`;
}

export function formatMb(mb: number): string {
  return bytesToString(mbToBytes(mb));
}

export function formatCpu(cpu: number): string {
  return `${cpu}%`;
}

export function formatResource(key: PoolResourceKey, value: number): string {
  return key === 'cpu' ? formatCpu(value) : formatMb(value);
}

/** A limit where `0` means unlimited. */
export function formatLimit(key: PoolResourceKey, value: number): string {
  return value === 0 ? 'Unlimited' : formatResource(key, value);
}

/** cpu, memory, disk or swap of a master's plan. */
export type PlanKey = PoolResourceKey | 'swap';

export const PLAN_LABELS: Record<PlanKey, string> = { ...RESOURCE_LABELS, swap: 'Swap' };

/** A plan limit: `0` means unlimited for cpu/memory/disk, `-1` for swap. */
export function formatPlanLimit(key: PlanKey, value: number): string {
  if (key === 'swap') return value === -1 ? 'Unlimited' : formatMb(value);
  return formatLimit(key, value);
}

/** The amounts above 0, e.g. `50% CPU, 1 GiB memory`. */
export function formatOvercommit(amounts: ResourceAmounts): string {
  return (['cpu', 'memory', 'disk', 'swap'] as const)
    .filter((key) => amounts[key] > 0)
    .map((key) => (key === 'cpu' ? `${formatCpu(amounts.cpu)} CPU` : `${formatMb(amounts[key])} ${key}`))
    .join(', ');
}

/** A signed change, `±0` when nothing changes. */
export function formatDelta(key: PoolResourceKey, delta: number): string {
  if (delta === 0) return '±0';
  return `${delta > 0 ? '+' : '−'}${formatResource(key, Math.abs(delta))}`;
}

export function plural(count: number, one: string, many: string = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}
