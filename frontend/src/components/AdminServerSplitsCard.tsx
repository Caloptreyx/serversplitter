import { faNetworkWired } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Text } from '@mantine/core';
import { type ReactNode, useEffect, useState } from 'react';
import Badge from '@/elements/data-display/Badge.tsx';
import TableLink from '@/elements/data-display/TableLink.tsx';
import TitleCard from '@/elements/data-display/TitleCard.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import { bytesToString, mbToBytes } from '@/lib/format/size.ts';
import type { AdminServer } from '@/lib/schemas/admin/servers.ts';
import { type AdminServerSplits, getAdminServerSplits } from '../api/client.ts';

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
  const [data, setData] = useState<AdminServerSplits | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    setData(null);
    setFailed(false);
    getAdminServerSplits(server.uuid)
      .then((splits) => active && setData(splits))
      .catch(() => active && setFailed(true));
    return () => {
      active = false;
    };
  }, [server.uuid]);

  const splitsLimit = typeof server.featureLimits.splits === 'number' ? server.featureLimits.splits : 0;
  const role = data?.parent ? 'Split' : data && (data.splits.length > 0 || splitsLimit > 0) ? 'Master' : null;

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
            {splitsLimit > 0 ? `${data.splits.length} of ${splitsLimit} used` : 'Disabled'}
          </InfoRow>
          {data.splits.map((split) => (
            <InfoRow key={split.uuid} label={<TableLink to={`/admin/servers/${split.uuid}`}>{split.name}</TableLink>}>
              <span className='text-(--mantine-color-dimmed)'>
                {formatLimit(split.cpu, '%')} · {formatLimit(split.memory, 'MB')} · {formatLimit(split.disk, 'MB')}
              </span>
            </InfoRow>
          ))}
        </>
      )}
    </TitleCard>
  );
}
