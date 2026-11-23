import { faNetworkWired, faUpRightAndDownLeftFromCenter } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, Text } from '@mantine/core';
import { type ReactNode, useCallback, useEffect, useState } from 'react';
import type { AdminServer } from '@/lib/schemas/admin/servers.ts';
import ActionIcon from '@/elements/buttons/ActionIcon.tsx';
import Badge from '@/elements/data-display/Badge.tsx';
import TableLink from '@/elements/data-display/TableLink.tsx';
import TitleCard from '@/elements/data-display/TitleCard.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import Tooltip from '@/elements/overlays/Tooltip.tsx';
import { bytesToString, mbToBytes } from '@/lib/format/size.ts';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { type AdminServerSplits, type AdminSplitSummary, getAdminServerSplits } from '../api/admin.ts';
import AdminResizeModal from './AdminResizeModal.tsx';

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

      <AdminResizeModal split={resizing} onClose={() => setResizing(null)} onResized={load} />
    </TitleCard>
  );
}
