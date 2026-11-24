import {
  faArrowRight,
  faBan,
  faHardDrive,
  faMemory,
  faMicrochip,
  faNetworkWired,
  faPause,
  faPlus,
  faTriangleExclamation,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { useEffect, useState } from 'react';
import { NavLink } from 'react-router';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import ServerContentContainer from '@/elements/containers/ServerContentContainer.tsx';
import Card from '@/elements/data-display/Card.tsx';
import StatCard from '@/elements/data-display/StatCard.tsx';
import Alert from '@/elements/feedback/Alert.tsx';
import EmptyState from '@/elements/feedback/EmptyState.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import Title from '@/elements/typography/Title.tsx';
import { useServerCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import {
  getClientSplitter,
  type ParentServer,
  type SplitEntry,
  type SplitShare,
  type SplitterClientIndex,
  syncSubusers,
} from '../api/client.ts';
import DeleteSplitModal from '../components/DeleteSplitModal.tsx';
import PoolCard from '../components/PoolCard.tsx';
import SplitDrawer from '../components/SplitDrawer.tsx';
import SplitRow from '../components/SplitRow.tsx';
import SplitSubusersModal, { syncResultMessage } from '../components/SplitSubusersModal.tsx';
import { formatLimit, plural } from '../components/splitterFormat.ts';

function SplitOwnPage({ parent, share }: { parent: ParentServer | null; share: SplitShare | null }) {
  const ledger = share?.ledger;

  return (
    <div className='flex flex-col gap-4'>
      {share?.suspended_by_master && (
        <Alert color='orange' icon={faPause} title='Suspended with master'>
          {parent?.name ?? 'The master'} is suspended, so this split is too. It is lifted when the master is
          unsuspended.
        </Alert>
      )}

      {parent && (
        <Card>
          <div className='flex w-full flex-col sm:flex-row sm:items-center sm:justify-between gap-3'>
            <div className='min-w-0'>
              <p className='text-sm text-(--mantine-color-dimmed)'>This server is a split of</p>
              <p className='text-lg font-medium truncate'>{parent.name}</p>
              <p className='text-xs text-(--mantine-color-dimmed)'>
                Its resources come from {parent.name}&apos;s pool; resize or delete it there.
              </p>
            </div>
            <NavLink to={`/server/${parent.uuid}/splitter`} className='shrink-0'>
              <Button variant='light' rightSection={<FontAwesomeIcon icon={faArrowRight} />}>
                Go to {parent.name}
              </Button>
            </NavLink>
          </div>
        </Card>
      )}

      {share && share.health !== 'balanced' && (
        <Alert color='yellow' icon={faTriangleExclamation}>
          {share.health === 'drift'
            ? "This split's limits were changed outside the splitter. An administrator can reconcile it."
            : 'The splitter has no record of what this split holds. An administrator can reconcile it.'}
        </Alert>
      )}

      {ledger && (
        <>
          <Title order={4}>Share of the pool</Title>
          <div className='grid grid-cols-1 sm:grid-cols-3 gap-4'>
            <StatCard icon={faMicrochip} label='CPU' value={formatLimit('cpu', ledger.cpu)} />
            <StatCard icon={faMemory} label='Memory' value={formatLimit('memory', ledger.memory)} />
            <StatCard icon={faHardDrive} label='Disk' value={formatLimit('disk', ledger.disk)} />
          </div>
          <div className='flex flex-wrap gap-2 text-xs'>
            {[
              plural(ledger.allocations, 'port'),
              plural(ledger.databases, 'database'),
              plural(ledger.backups, 'backup'),
              plural(ledger.schedules, 'schedule'),
              ...(ledger.swap !== 0 ? [`${formatLimit('memory', ledger.swap)} swap`] : []),
            ].map((chip) => (
              <span
                key={chip}
                className='rounded-full border border-(--mantine-color-default-border) px-3 py-1 font-medium'
              >
                {chip}
              </span>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

export default function ServerSplitterPage() {
  const server = useServerStore((state) => state.server);
  const { addToast } = useToast();
  const canCreate = useServerCan('splitter.create');

  const [data, setData] = useState<SplitterClientIndex | null>(null);
  const [loading, setLoading] = useState(true);
  // The split stays set while the drawer closes, so its content doesn't flip to "New split" mid-animation.
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [drawerSplit, setDrawerSplit] = useState<SplitEntry | null>(null);
  const [deleting, setDeleting] = useState<SplitEntry | null>(null);
  const [subusersOf, setSubusersOf] = useState<SplitEntry | null>(null);
  const [syncingSplit, setSyncingSplit] = useState<string | null>(null);

  const load = () => {
    setLoading(true);
    getClientSplitter(server.uuid)
      .then(setData)
      .catch((err) => addToast(httpErrorToHuman(err), 'error'))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    load();
  }, [server.uuid]);

  const openDrawer = (split: SplitEntry | null) => {
    setDrawerSplit(split);
    setDrawerOpen(true);
  };

  const syncUsers = async (split: SplitEntry) => {
    setSyncingSplit(split.server.uuid);

    try {
      const result = await syncSubusers(server.uuid, split.server.uuid);
      addToast(`${split.server.name}: ${syncResultMessage(result)}`, result.skipped.length > 0 ? 'warning' : 'success');
      load();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSyncingSplit(null);
    }
  };

  if (!data) {
    return (
      <ServerContentContainer title='Server Splitter'>
        <div className='flex justify-center items-center py-24'>
          {loading ? <Spinner /> : <p className='text-(--mantine-color-dimmed)'>The splitter could not be loaded.</p>}
        </div>
      </ServerContentContainer>
    );
  }

  if (data.role === 'split') {
    return (
      <ServerContentContainer
        title='Server Splitter'
        subtitle={data.parent ? `Split of ${data.parent.name}` : 'Split server'}
      >
        <SplitOwnPage parent={data.parent} share={data.share} />
      </ServerContentContainer>
    );
  }

  const { pool, splits } = data;
  const limit = pool?.splits.limit ?? 0;
  const used = pool?.splits.used ?? splits.length;
  const canCreateMore = canCreate && !!pool && used < limit;

  const newSplitButton = canCreateMore ? (
    <Button onClick={() => openDrawer(null)} leftSection={<FontAwesomeIcon icon={faPlus} />}>
      New split
    </Button>
  ) : undefined;

  return (
    <ServerContentContainer
      title='Server Splitter'
      subtitle={limit > 0 ? `${used} of ${limit} splits` : 'Splitting disabled'}
      contentRight={newSplitButton}
    >
      <div className='flex flex-col gap-6'>
        {limit === 0 && splits.length === 0 ? (
          <EmptyState
            icon={faBan}
            title='Splitting disabled'
            description='This server may not be split. Ask an administrator to give it a split limit.'
          />
        ) : (
          <>
            {pool && <PoolCard pool={pool} splits={splits} masterName={server.name} />}

            <div className='flex flex-col gap-3'>
              <Title order={3}>Splits</Title>
              {splits.length === 0 ? (
                <EmptyState
                  icon={faNetworkWired}
                  title='No splits yet'
                  description={`Give part of ${server.name}'s resources to a separate server with its own console, files and egg.`}
                >
                  {newSplitButton}
                </EmptyState>
              ) : (
                <Card p={0} className='overflow-hidden'>
                  {splits.map((split) => (
                    <SplitRow
                      key={split.server.uuid}
                      split={split}
                      busy={syncingSplit === split.server.uuid}
                      onResize={() => openDrawer(split)}
                      onSyncUsers={() => syncUsers(split)}
                      onManageSubusers={() => setSubusersOf(split)}
                      onDelete={() => setDeleting(split)}
                    />
                  ))}
                </Card>
              )}
            </div>
          </>
        )}
      </div>

      {pool && (
        <SplitDrawer
          opened={drawerOpen}
          onClose={() => setDrawerOpen(false)}
          onSaved={load}
          serverUuid={server.uuid}
          masterName={server.name}
          pool={pool}
          split={drawerSplit}
          masterPower={data.master_power}
        />
      )}
      <DeleteSplitModal
        split={deleting}
        serverUuid={server.uuid}
        masterName={server.name}
        onClose={() => setDeleting(null)}
        onDeleted={load}
        masterPower={data.master_power}
      />
      <SplitSubusersModal
        split={subusersOf}
        serverUuid={server.uuid}
        masterName={server.name}
        onClose={() => setSubusersOf(null)}
        onChanged={load}
      />
    </ServerContentContainer>
  );
}
