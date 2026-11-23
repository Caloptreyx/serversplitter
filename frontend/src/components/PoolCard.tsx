import { faCircleCheck, faTriangleExclamation } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Progress } from '@mantine/core';
import Card from '@/elements/data-display/Card.tsx';
import Tooltip from '@/elements/overlays/Tooltip.tsx';
import Title from '@/elements/typography/Title.tsx';
import type { Pool, SplitEntry } from '../api/client.ts';
import {
  colorVar,
  formatMb,
  formatResource,
  type PoolResourceKey,
  RESOURCE_LABELS,
  splitColor,
} from './splitterFormat.ts';

interface Segment {
  key: string;
  value: number;
  /** Shown inside the segment when it is wide enough. */
  label: string;
  tooltip: string;
  color: string;
  striped?: boolean;
  /** The master's free part: outlined instead of filled. */
  free?: boolean;
}

/** Below this share of the bar a segment has no room for its label. */
const LABEL_MIN_PERCENT = 12;

function ResourceBar({
  resource,
  pool,
  splits,
  masterName,
}: {
  resource: PoolResourceKey;
  pool: Pool;
  splits: SplitEntry[];
  masterName: string;
}) {
  const res = pool[resource];
  const format = (value: number) => formatResource(resource, value);
  const label = RESOURCE_LABELS[resource];

  if (res.master === 0 || res.total === 0) {
    return (
      <div className='grid grid-cols-[5rem_1fr] sm:grid-cols-[5rem_1fr_auto] items-center gap-x-4 gap-y-1'>
        <span className='font-medium'>{label}</span>
        <span className='text-sm text-(--mantine-color-dimmed)'>Unlimited</span>
        <span className='hidden sm:block text-sm text-(--mantine-color-dimmed)'>Free: Unlimited</span>
      </div>
    );
  }

  const free = Math.max(0, Math.min(res.free, res.master));
  const reserved = Math.max(0, Math.min(res.reserved, res.master - free));
  const showReserve = pool.display_reserved && reserved > 0;
  const kept = Math.max(0, res.master - free - (showReserve ? reserved : 0));

  const segments: Segment[] = [];
  if (showReserve) {
    segments.push({
      key: 'reserved',
      value: reserved,
      label: '',
      tooltip: `Reserved for ${masterName}: ${format(reserved)} (can't be split off)`,
      color: 'gray.6',
      striped: true,
    });
  }
  if (kept > 0) {
    segments.push({
      key: 'kept',
      value: kept,
      label: `${masterName} ${format(kept)}`,
      tooltip: `${masterName} keeps ${format(res.master)} in total${showReserve ? ' (incl. reserve)' : ''}`,
      color: 'blue.7',
    });
  }
  if (free > 0) {
    segments.push({
      key: 'free',
      value: free,
      label: `Free ${format(free)}`,
      tooltip: `Free: ${format(free)}, stays with ${masterName} until you split it off`,
      color: 'blue.9',
      free: true,
    });
  }
  splits.forEach((split, index) => {
    const value = split.ledger[resource];
    if (value <= 0) return;
    segments.push({
      key: split.server.uuid,
      value,
      label: `${split.server.name} ${format(value)}`,
      tooltip: `${split.server.name}: ${format(value)}`,
      color: splitColor(index),
    });
  });

  const scale = Math.max(
    res.total,
    segments.reduce((sum, segment) => sum + segment.value, 0),
  );
  const percent = (value: number) => (scale > 0 ? (value / scale) * 100 : 0);

  return (
    <div className='grid grid-cols-[5rem_1fr] sm:grid-cols-[5rem_1fr_auto] items-center gap-x-4 gap-y-1'>
      <span className='font-medium'>{label}</span>
      <Progress.Root size={22} radius='sm' aria-label={`${label} pool`}>
        {segments.map((segment) => (
          <Tooltip key={segment.key} label={segment.tooltip} unwrapped>
            <Progress.Section
              value={percent(segment.value)}
              color={segment.color}
              striped={segment.striped}
              className='border-r border-(--mantine-color-body) last:border-r-0'
              style={
                segment.free
                  ? {
                      backgroundColor: 'transparent',
                      outline: `1px dashed ${colorVar('blue.5')}`,
                      outlineOffset: '-1px',
                    }
                  : undefined
              }
            >
              {segment.label && percent(segment.value) >= LABEL_MIN_PERCENT && (
                <Progress.Label className={segment.free ? 'text-(--mantine-color-text)!' : undefined}>
                  {segment.label}
                </Progress.Label>
              )}
            </Progress.Section>
          </Tooltip>
        ))}
      </Progress.Root>
      <span className='col-span-2 sm:col-span-1 text-sm text-(--mantine-color-dimmed) sm:text-right whitespace-nowrap'>
        Free <span className='font-semibold text-(--mantine-color-text)'>{format(res.free)}</span> of{' '}
        {format(res.total)}
      </span>
    </div>
  );
}

function Chip({ label, free, of }: { label: string; free: number; of?: number }) {
  return (
    <span className='inline-flex items-center gap-1 rounded-full border border-(--mantine-color-default-border) px-3 py-1 text-xs'>
      <span className='text-(--mantine-color-dimmed)'>{label}</span>
      <span className='font-semibold'>{free} free</span>
      {of !== undefined && <span className='text-(--mantine-color-dimmed)'>of {of}</span>}
    </span>
  );
}

function LegendDot({ color, label, striped }: { color: string; label: string; striped?: boolean }) {
  return (
    <span className='inline-flex items-center gap-1.5'>
      <span
        className='inline-block size-2.5 rounded-sm shrink-0'
        style={{
          background: striped
            ? `repeating-linear-gradient(45deg, ${colorVar(color)} 0 2px, transparent 2px 4px)`
            : colorVar(color),
        }}
      />
      {label}
    </span>
  );
}

export default function PoolCard({
  pool,
  splits,
  masterName,
}: {
  pool: Pool;
  splits: SplitEntry[];
  masterName: string;
}) {
  const balanced = pool.health.status === 'balanced';
  const swap = pool.swap;

  return (
    <Card className='flex flex-col gap-4'>
      <div
        className={`flex flex-col gap-1 rounded-md border px-3 py-2 text-sm ${
          balanced
            ? 'border-(--mantine-color-green-outline) bg-(--mantine-color-green-light)'
            : 'border-(--mantine-color-orange-outline) bg-(--mantine-color-orange-light)'
        }`}
      >
        <span
          className={`flex items-center gap-2 font-medium ${
            balanced ? 'text-(--mantine-color-green-light-color)' : 'text-(--mantine-color-orange-light-color)'
          }`}
        >
          <FontAwesomeIcon icon={balanced ? faCircleCheck : faTriangleExclamation} />
          {balanced ? 'Balanced' : 'Pool needs attention'}
        </span>
        {!balanced && pool.health.issues.length > 0 && (
          <ul className='list-disc pl-8 text-xs'>
            {pool.health.issues.map((issue) => (
              <li key={issue}>{issue}</li>
            ))}
          </ul>
        )}
      </div>

      <div>
        <Title order={4}>Pool</Title>
        <p className='text-sm text-(--mantine-color-dimmed)'>Everything {masterName} owns, and how it is shared</p>
      </div>

      <div className='flex flex-col gap-3'>
        {(['cpu', 'memory', 'disk'] as const).map((resource) => (
          <ResourceBar key={resource} resource={resource} pool={pool} splits={splits} masterName={masterName} />
        ))}
      </div>

      <div className='flex flex-wrap gap-x-4 gap-y-1 text-xs text-(--mantine-color-dimmed)'>
        {pool.display_reserved && <LegendDot color='gray.6' label='Reserved' striped />}
        <LegendDot color='blue.7' label={`${masterName} keeps`} />
        <LegendDot color='blue.9' label='Free (splittable)' />
        {splits.map((split, index) => (
          <LegendDot key={split.server.uuid} color={splitColor(index)} label={split.server.name} />
        ))}
      </div>

      {swap.master !== 0 && (
        <p className='text-xs text-(--mantine-color-dimmed)'>
          Swap: {masterName} keeps {swap.master === -1 ? 'Unlimited' : formatMb(swap.master)}
          {swap.splits > 0 && ` · splits ${formatMb(swap.splits)}`} · mode {pool.swap_mode}
        </p>
      )}

      <div className='flex flex-wrap gap-2'>
        <Chip label='Ports' free={pool.allocations.free} />
        <Chip label='Databases' free={pool.databases.free} />
        <Chip label='Backups' free={pool.backups.free} />
        <Chip label='Schedules' free={pool.schedules.free} />
        <Chip label='Splits' free={Math.max(0, pool.splits.limit - pool.splits.used)} of={pool.splits.limit} />
      </div>
    </Card>
  );
}
