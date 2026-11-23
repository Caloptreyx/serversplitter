import type { KeyboardEvent, MouseEvent } from 'react';
import {
  faBan,
  faEllipsisVertical,
  faPause,
  faRotate,
  faSliders,
  faTerminal,
  faTrash,
  faTriangleExclamation,
  faUserGear,
  faUsers,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Progress } from '@mantine/core';
import classNames from 'classnames';
import { NavLink, useNavigate } from 'react-router';
import ActionIcon from '@/elements/buttons/ActionIcon.tsx';
import Button from '@/elements/buttons/Button.tsx';
import CopyOnClick from '@/elements/CopyOnClick.tsx';
import Badge from '@/elements/data-display/Badge.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import Menu from '@/elements/overlays/Menu.tsx';
import Tooltip from '@/elements/overlays/Tooltip.tsx';
import { formatAllocation, statusToColor } from '@/lib/domain/server.ts';
import { bytesToString, mbToBytes } from '@/lib/format/size.ts';
import { usageColor } from '@/lib/format/usage.ts';
import { useServerStats } from '@/plugins/server/useServerStats.ts';
import { useServerCan } from '@/plugins/usePermissions.ts';
import type { SplitEntry } from '../api/client.ts';
import { plural } from './splitterFormat.ts';

// Keep clicks on the row's own controls from opening the split. React events bubble through
// portals, so this also covers the menu dropdown.
const stopRow = (e: MouseEvent) => e.stopPropagation();

/** Used vs limit; `limit` 0 = unlimited (empty bar). */
function MiniUsage({
  label,
  used,
  limit,
  format,
}: {
  label: string;
  used: number;
  limit: number;
  format: (value: number) => string;
}) {
  return (
    <div className='flex flex-col gap-1 min-w-0'>
      <div className='flex items-baseline gap-1 text-xs whitespace-nowrap'>
        <span className='text-(--mantine-color-dimmed)'>{label}</span>
        <span className='font-semibold'>{format(used)}</span>
        <span className='text-(--mantine-color-dimmed) truncate'>/ {limit === 0 ? 'Unlimited' : format(limit)}</span>
      </div>
      <Progress
        size='sm'
        value={limit === 0 ? 0 : Math.min(100, (used / limit) * 100)}
        color={usageColor(used, limit) ?? 'blue'}
      />
    </div>
  );
}

const formatCpuUsage = (value: number) => `${Number(value.toFixed(1))}%`;

export default function SplitRow({
  split,
  busy,
  onResize,
  onSyncUsers,
  onManageSubusers,
  onDelete,
}: {
  split: SplitEntry;
  busy: boolean;
  onResize: () => void;
  onSyncUsers: () => void;
  onManageSubusers: () => void;
  onDelete: () => void;
}) {
  const { server } = split;
  const navigate = useNavigate();
  const stats = useServerStats(server);
  const canUpdate = useServerCan('splitter.update');
  const canDelete = useServerCan('splitter.delete');
  const canSync = useServerCan(['splitter.update', 'subusers.create', 'subusers.update'], false);

  const splitPath = `/server/${server.uuidShort}`;
  const openSplit = () => navigate(splitPath);
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault();
      openSplit();
    }
  };

  const limits = server.featureLimits;

  return (
    <div
      role='link'
      tabIndex={0}
      aria-label={`Open ${server.name}`}
      onClick={openSplit}
      onKeyDown={onKeyDown}
      className='flex flex-col lg:flex-row lg:items-center gap-3 px-4 py-3 cursor-pointer transition-colors hover:bg-white/2 light:hover:bg-black/3 border-b border-(--mantine-color-default-border) last:border-b-0'
    >
      <div className='flex items-start gap-3 min-w-0 lg:w-80 lg:shrink-0'>
        <span
          className={classNames('mt-1.5 size-2.5 rounded-full shrink-0', statusToColor(stats?.state))}
          aria-label={stats?.state ?? 'unknown'}
        />
        <div className='min-w-0 flex flex-col gap-1'>
          <p className='truncate'>
            <span className='font-medium'>{server.name}</span>
            <span className='text-sm text-(--mantine-color-dimmed)'> · {server.egg.name}</span>
          </p>
          <div className='flex flex-wrap items-center gap-x-2 gap-y-1'>
            {server.allocation && (
              <span onClick={stopRow}>
                <CopyOnClick content={formatAllocation(server.allocation)}>
                  <span className='inline-block rounded-md border border-(--mantine-color-default-border) px-1.5 py-0.5 font-mono text-xs text-(--mantine-color-dimmed)'>
                    {formatAllocation(server.allocation)}
                  </span>
                </CopyOnClick>
              </span>
            )}
            <span className='text-xs text-(--mantine-color-dimmed)'>
              {plural(limits.allocations, 'port')} · {plural(limits.databases, 'database')} ·{' '}
              {plural(limits.backups, 'backup')} · {plural(limits.schedules, 'schedule')}
            </span>
          </div>
          <div className='flex flex-wrap gap-1 empty:hidden'>
            {split.suspended_by_master && (
              <Badge size='sm' color='orange' variant='light' leftSection={<FontAwesomeIcon icon={faPause} />}>
                Suspended with master
              </Badge>
            )}
            {split.synced_subusers > 0 && (
              <Badge size='sm' color='blue' variant='light' leftSection={<FontAwesomeIcon icon={faRotate} />}>
                Subusers synced ({split.synced_subusers})
              </Badge>
            )}
            {split.health !== 'balanced' && (
              <Tooltip
                label={
                  split.health === 'drift'
                    ? "This split's limits were changed outside the splitter"
                    : 'The splitter has no record of what this split holds'
                }
              >
                <Badge
                  size='sm'
                  color='yellow'
                  variant='light'
                  leftSection={<FontAwesomeIcon icon={faTriangleExclamation} />}
                >
                  {split.health === 'drift' ? 'Drift' : 'Untracked'}
                </Badge>
              </Tooltip>
            )}
          </div>
        </div>
      </div>

      <div className='flex-1 min-w-0'>
        {server.isSuspended ? (
          <div className='flex items-center gap-2 text-sm'>
            <FontAwesomeIcon icon={faBan} className='text-(--mantine-color-red-filled)' />
            Suspended
          </div>
        ) : !stats ? (
          <Spinner size={16} />
        ) : (
          <div className='grid grid-cols-1 sm:grid-cols-3 gap-3'>
            <MiniUsage label='CPU' used={stats.cpuAbsolute} limit={server.limits.cpu} format={formatCpuUsage} />
            <MiniUsage
              label='Mem'
              used={stats.memoryBytes}
              limit={mbToBytes(server.limits.memory)}
              format={bytesToString}
            />
            <MiniUsage
              label='Disk'
              used={stats.diskBytes}
              limit={mbToBytes(server.limits.disk)}
              format={bytesToString}
            />
          </div>
        )}
      </div>

      <div className='flex items-center gap-2 shrink-0' onClick={stopRow}>
        <NavLink to={splitPath}>
          <Button size='xs' variant='default' leftSection={<FontAwesomeIcon icon={faTerminal} />}>
            Console
          </Button>
        </NavLink>
        {canUpdate && (
          <Button size='xs' variant='default' leftSection={<FontAwesomeIcon icon={faSliders} />} onClick={onResize}>
            Resize
          </Button>
        )}
        <Menu position='bottom-end' shadow='md' width={240}>
          <Menu.Target>
            <ActionIcon size='input-xs' variant='default' loading={busy} aria-label={`Actions for ${server.name}`}>
              <FontAwesomeIcon icon={faEllipsisVertical} />
            </ActionIcon>
          </Menu.Target>
          <Menu.Dropdown>
            {canUpdate && (
              <Menu.Item leftSection={<FontAwesomeIcon icon={faSliders} />} onClick={onResize}>
                Resize
              </Menu.Item>
            )}
            {canSync && (
              <Menu.Item leftSection={<FontAwesomeIcon icon={faUsers} />} onClick={onSyncUsers}>
                Sync users from master
              </Menu.Item>
            )}
            <Menu.Item leftSection={<FontAwesomeIcon icon={faUserGear} />} onClick={onManageSubusers}>
              Manage subusers
            </Menu.Item>
            {canDelete && (
              <>
                <Menu.Divider />
                <Menu.Item color='red' leftSection={<FontAwesomeIcon icon={faTrash} />} onClick={onDelete}>
                  Delete split
                </Menu.Item>
              </>
            )}
          </Menu.Dropdown>
        </Menu>
      </div>
    </div>
  );
}
