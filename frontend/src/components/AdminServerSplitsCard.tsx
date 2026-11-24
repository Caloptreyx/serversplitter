import {
  faNetworkWired,
  faTriangleExclamation,
  faUpRightAndDownLeftFromCenter,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, Text } from '@mantine/core';
import { Fragment, type ReactNode, useCallback, useEffect, useState } from 'react';
import type { AdminServer } from '@/lib/schemas/admin/servers.ts';
import ActionIcon from '@/elements/buttons/ActionIcon.tsx';
import Badge from '@/elements/data-display/Badge.tsx';
import TableLink from '@/elements/data-display/TableLink.tsx';
import TitleCard from '@/elements/data-display/TitleCard.tsx';
import Alert from '@/elements/feedback/Alert.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import Tooltip from '@/elements/overlays/Tooltip.tsx';
import { bytesToString, mbToBytes } from '@/lib/format/size.ts';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { type AdminPlan, type AdminServerSplits, type AdminSplitSummary, getAdminServerSplits } from '../api/admin.ts';
import AdminResizeModal from './AdminResizeModal.tsx';
import { formatCpu, formatMb, formatOvercommit, formatPlanLimit, PLAN_LABELS, type PlanKey } from './splitterFormat.ts';

// Same row markup as the admin server Overview cards.
function InfoRow({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div className='flex items-start justify-between gap-4 py-1.5 border-b border-(--mantine-color-default-border) last:border-b-0'>
      <div className='text-sm text-(--mantine-color-dimmed) min-w-0'>{label}</div>
      <div className='text-sm text-right'>{children}</div>
    </div>
  );
}

/** A split limit: 0 means unlimited. CPU is a percentage, memory and disk are in MB. */
function formatLimit(value: number, unit: '%' | 'MB') {
  if (value === 0) return 'Unlimited';
  return unit === '%' ? `${value}%` : bytesToString(mbToBytes(value));
}

/** What a master's limits mean: plan total, what its splits hold and what it keeps. */
function PlanBlock({ plan }: { plan: AdminPlan }) {
  const keys: PlanKey[] = plan.swap_pooled ? ['cpu', 'memory', 'disk', 'swap'] : ['cpu', 'memory', 'disk'];

  return (
    <div className='flex flex-col gap-2 py-2 border-b border-(--mantine-color-default-border) last:border-b-0'>
      <div className='grid grid-cols-[auto_1fr_1fr_1fr] gap-x-3 gap-y-1 text-sm'>
        <span />
        <span className='text-xs text-(--mantine-color-dimmed) text-right'>Plan total</span>
        <span className='text-xs text-(--mantine-color-dimmed) text-right'>Splits hold</span>
        <span className='text-xs text-(--mantine-color-dimmed) text-right'>Master keeps</span>
        {keys.map((key) => (
          <Fragment key={key}>
            <span className='text-(--mantine-color-dimmed)'>{PLAN_LABELS[key]}</span>
            <span className='text-right'>{formatPlanLimit(key, plan.total[key])}</span>
            <span className='text-right'>
              {key === 'cpu' ? formatCpu(plan.splits.cpu) : formatMb(plan.splits[key])}
            </span>
            <span className='text-right'>{formatPlanLimit(key, plan.master[key])}</span>
          </Fragment>
        ))}
      </div>
      <Text size='xs' c='dimmed'>
        {plan.mode === 'plan_total'
          ? 'Limits set on this server (Build/edit page or the admin API, e.g. a billing panel) are read as the plan total; the master keeps the total minus what its splits hold.'
          : 'Limits set on this server are what the master keeps; its splits come on top.'}
      </Text>
      {plan.overcommit && (
        <Alert color='red' icon={faTriangleExclamation} title='Over-committed'>
          The plan is smaller than what the splits hold plus the reserve. Free {formatOvercommit(plan.overcommit)} by
          shrinking or deleting splits; the master is held at its reserve until then.
        </Alert>
      )}
    </div>
  );
}

/** Admin server Overview card: which master a split belongs to, or which splits a master has. */
export default function AdminServerSplitsCard({ server }: { server: AdminServer }) {
  const canResize = useAdminCan('servers.update');
  const [data, setData] = useState<AdminServerSplits | null>(null);
  const [failed, setFailed] = useState(false);
  const [resizing, setResizing] = useState<AdminSplitSummary | null>(null);

  const load = useCallback(() => {
    let active = true;
    setFailed(false);
    getAdminServerSplits(server.uuid)
      .then((splits) => active && setData(splits))
      .catch(() => active && setFailed(true));
    return () => {
      active = false;
    };
  }, [server.uuid]);

  useEffect(() => {
    setData(null);
    return load();
  }, [load]);

  const role = data?.parent ? 'Split' : data && (data.splits.length > 0 || data.splits_limit > 0) ? 'Master' : null;

  return (
    <TitleCard
      title='Server Splitter'
      icon={<FontAwesomeIcon icon={faNetworkWired} />}
      rightSection={
        role && (
          <Badge color='gray' variant='light' size='sm' ml='auto'>
            {role}
          </Badge>
        )
      }
    >
      {failed ? (
        <Text size='sm' c='dimmed'>
          Could not load split information.
        </Text>
      ) : !data ? (
        <Spinner size={16} />
      ) : data.parent ? (
        <>
          <InfoRow label='Split of'>
            <TableLink to={`/admin/servers/${data.parent.uuid}`}>{data.parent.name}</TableLink>
          </InfoRow>
          <Text size='xs' c='dimmed' mt='xs'>
            Resources given to this server come out of the master server&apos;s pool.
          </Text>
        </>
      ) : (
        <>
          <InfoRow label='Splits'>
            {data.splits_limit > 0 ? `${data.splits.length} of ${data.splits_limit} used` : 'Disabled'}
          </InfoRow>
          {data.plan && (data.splits.length > 0 || data.plan.overcommit) && <PlanBlock plan={data.plan} />}
          {data.splits.map((split) => (
            <InfoRow
              key={split.uuid}
              label={
                <Group gap={6} wrap='wrap'>
                  <TableLink to={`/admin/servers/${split.uuid}`}>{split.name}</TableLink>
                  {split.health === 'drift' && (
                    <Badge color='orange' variant='light' size='xs'>
                      Drift
                    </Badge>
                  )}
                  {split.health === 'untracked' && (
                    <Badge color='gray' variant='light' size='xs'>
                      Untracked
                    </Badge>
                  )}
                  {split.suspended && (
                    <Badge color='red' variant='light' size='xs'>
                      Suspended
                    </Badge>
                  )}
                </Group>
              }
            >
              <Group gap={6} wrap='nowrap' justify='flex-end'>
                <span className='text-(--mantine-color-dimmed)'>
                  {formatLimit(split.limits.cpu, '%')} · {formatLimit(split.limits.memory, 'MB')} ·{' '}
                  {formatLimit(split.limits.disk, 'MB')}
                </span>
                {canResize && (
                  <Tooltip label='Resize within the pool'>
                    <ActionIcon
                      variant='subtle'
                      size='sm'
                      aria-label={`Resize ${split.name}`}
                      onClick={() => setResizing(split)}
                    >
                      <FontAwesomeIcon icon={faUpRightAndDownLeftFromCenter} />
                    </ActionIcon>
                  </Tooltip>
                )}
              </Group>
            </InfoRow>
          ))}
        </>
      )}

      <AdminResizeModal
        split={resizing}
        masterPower={data?.master_power ?? null}
        onClose={() => setResizing(null)}
        onResized={load}
      />
    </TitleCard>
  );
}
