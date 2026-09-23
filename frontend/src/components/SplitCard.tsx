import {
  faBan,
  faEllipsisVertical,
  faHardDrive,
  faMemory,
  faMicrochip,
  faSliders,
  faTrash,
  faUsers,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import type { MouseEvent, ReactNode } from 'react';
import { NavLink } from 'react-router';
import ActionIcon from '@/elements/buttons/ActionIcon.tsx';
import CopyOnClick from '@/elements/CopyOnClick.tsx';
import Card from '@/elements/data-display/Card.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import Divider from '@/elements/layout/Divider.tsx';
import Menu from '@/elements/overlays/Menu.tsx';
import ScrollingText from '@/elements/ScrollingText.tsx';
import { formatAllocation, statusToColor } from '@/lib/domain/server.ts';
import { bytesToString, mbToBytes } from '@/lib/format/size.ts';
import { useServerStats } from '@/plugins/server/useServerStats.ts';
import type { Server } from '../api/client.ts';

// Keep clicks on the card's own controls from following the card link. React events bubble
// through portals, so this also covers the menu dropdown.
const stopLink = (e: MouseEvent) => {
  e.preventDefault();
  e.stopPropagation();
};

function Stat({ icon, value, limit }: { icon: typeof faMicrochip; value: string; limit: string }) {
  return (
    <div className='flex gap-2 text-sm items-center'>
      <FontAwesomeIcon icon={icon} className='size-4 flex-none text-(--mantine-color-dimmed)' />
      <span>
        <span className='mr-1'>{value}</span>
        <span className='text-xs text-(--mantine-color-dimmed)'>/ {limit}</span>
      </span>
    </div>
  );
}

function Limit({ label, value }: { label: string; value: number }): ReactNode {
  return (
    <span>
      <span className='text-(--mantine-color-text)'>{value}</span> {label}
    </span>
  );
}

export default function SplitCard({
  server,
  syncing,
  onResize,
  onSyncUsers,
  onDelete,
}: {
  server: Server;
  syncing: boolean;
  onResize: () => void;
  onSyncUsers: () => void;
  onDelete: () => void;
}) {
  const stats = useServerStats(server);

  const cpuLimit = server.limits.cpu !== 0 ? `${server.limits.cpu}%` : 'Unlimited';
  const memoryLimit = server.limits.memory !== 0 ? bytesToString(mbToBytes(server.limits.memory)) : 'Unlimited';
  const diskLimit = server.limits.disk !== 0 ? bytesToString(mbToBytes(server.limits.disk)) : 'Unlimited';

  return (
    <NavLink to={`/server/${server.uuidShort}`} className='block min-w-0'>
      <Card
        className='h-full flex flex-col rounded-xl! overflow-hidden min-w-0'
        leftStripeClassName={statusToColor(stats?.state)}
        hoverable
      >
        <div className='flex items-start justify-between gap-3 min-w-0'>
          <div className='min-w-0 flex-1'>
            <ScrollingText className='text-lg font-medium'>{server.name}</ScrollingText>
            <p className='text-sm text-(--mantine-color-dimmed) truncate'>
              {server.egg.name}
              {server.description && ` · ${server.description}`}
            </p>
          </div>

          <div className='flex items-center gap-2 shrink-0'>
            {server.allocation && (
              <CopyOnClick content={formatAllocation(server.allocation)} className='min-w-0'>
                <Card p='xs' hoverable className='leading-[100%] rounded-lg!'>
                  <p className='text-sm text-(--mantine-color-dimmed)'>{formatAllocation(server.allocation)}</p>
                </Card>
              </CopyOnClick>
            )}

            <Menu position='bottom-end' shadow='md' width={220}>
              <Menu.Target>
                <ActionIcon
                  size='input-sm'
                  variant='light'
                  color='gray'
                  loading={syncing}
                  aria-label={`Actions for ${server.name}`}
                  onClick={stopLink}
                >
                  <FontAwesomeIcon icon={faEllipsisVertical} />
                </ActionIcon>
              </Menu.Target>
              <Menu.Dropdown onClick={stopLink}>
                <Menu.Item leftSection={<FontAwesomeIcon icon={faSliders} />} onClick={onResize}>
                  Resize
                </Menu.Item>
                <Menu.Item leftSection={<FontAwesomeIcon icon={faUsers} />} onClick={onSyncUsers}>
                  Sync users from master
                </Menu.Item>
                <Menu.Divider />
                <Menu.Item color='red' leftSection={<FontAwesomeIcon icon={faTrash} />} onClick={onDelete}>
                  Delete split
                </Menu.Item>
              </Menu.Dropdown>
            </Menu>
          </div>
        </div>

        <Divider my='md' />

        {server.isSuspended ? (
          <div className='flex items-center justify-center gap-2 text-sm'>
            <FontAwesomeIcon icon={faBan} className='text-(--mantine-color-red-filled)' />
            Suspended
          </div>
        ) : !stats ? (
          <div className='flex justify-center'>
            <Spinner size={16} />
          </div>
        ) : (
          <div className='grid grid-cols-1 sm:grid-cols-3 gap-2'>
            <Stat icon={faMicrochip} value={`${stats.cpuAbsolute.toFixed(1)}%`} limit={cpuLimit} />
            <Stat icon={faMemory} value={bytesToString(stats.memoryBytes)} limit={memoryLimit} />
            <Stat icon={faHardDrive} value={bytesToString(stats.diskBytes)} limit={diskLimit} />
          </div>
        )}

        <div className='mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-(--mantine-color-dimmed)'>
          <Limit
            label={server.featureLimits.allocations === 1 ? 'port' : 'ports'}
            value={server.featureLimits.allocations}
          />
          <Limit
            label={server.featureLimits.databases === 1 ? 'database' : 'databases'}
            value={server.featureLimits.databases}
          />
          <Limit
            label={server.featureLimits.backups === 1 ? 'backup' : 'backups'}
            value={server.featureLimits.backups}
          />
          <Limit
            label={server.featureLimits.schedules === 1 ? 'schedule' : 'schedules'}
            value={server.featureLimits.schedules}
          />
        </div>
      </Card>
    </NavLink>
  );
}
